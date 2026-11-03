use crate::metrics::Process;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug)]
pub struct LazyPolicy {
    pub smoothing_seconds: f64,
    pub minimum_hold_seconds: f64,
    pub absolute_margin: f32,
    pub relative_margin: f32,
    pub confirmation_samples: u32,
    pub idle_floor: f32,
}

impl Default for LazyPolicy {
    fn default() -> Self {
        Self {
            smoothing_seconds: 3.0,
            minimum_hold_seconds: 6.0,
            absolute_margin: 2.0,
            relative_margin: 0.15,
            confirmation_samples: 3,
            idle_floor: 0.5,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Slot {
    id: Option<u64>,
    since: f64,
    challenger: Option<u64>,
    confirmations: u32,
}

/// CPU-lazy selection: smooth ranks, hold names, confirm sustained challengers.
/// Slots stay in place even if the two selected processes swap CPU rank.
pub struct LazyTop<const N: usize> {
    pub policy: LazyPolicy,
    scores: BTreeMap<u64, f32>,
    slots: [Slot; N],
    last_time: Option<f64>,
}

impl<const N: usize> Default for LazyTop<N> {
    fn default() -> Self {
        Self::new(LazyPolicy::default())
    }
}

impl<const N: usize> LazyTop<N> {
    pub fn new(policy: LazyPolicy) -> Self {
        Self {
            policy,
            scores: BTreeMap::new(),
            slots: [Slot::default(); N],
            last_time: None,
        }
    }

    pub fn update(&mut self, now: f64, processes: &[Process]) {
        if self.last_time.is_some_and(|last| now <= last) {
            return;
        }
        let dt = self.last_time.map_or(1.0, |last| now - last);
        let alpha = (1.0 - (-dt / self.policy.smoothing_seconds.max(0.01)).exp()) as f32;
        let active: BTreeSet<_> = processes.iter().map(|p| p.id).collect();
        self.scores.retain(|id, _| active.contains(id));
        for process in processes {
            let measured = if process.cpu.is_finite() {
                process.cpu.max(0.0)
            } else {
                0.0
            };
            self.scores
                .entry(process.id)
                .and_modify(|v| *v += (measured - *v) * alpha)
                .or_insert(measured);
        }
        for slot in &mut self.slots {
            if slot.id.is_some_and(|id| !self.scores.contains_key(&id)) {
                *slot = Slot::default();
            }
        }
        let mut ranked: Vec<_> = self
            .scores
            .iter()
            .map(|(&id, &score)| (id, score))
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        // Fill vacancies before considering replacements, so an exited process is
        // replaced immediately even when the remaining slots are still in their hold period.
        for index in 0..N {
            if self.slots[index].id.is_some() {
                continue;
            }
            if let Some(&(id, _)) = ranked
                .iter()
                .find(|(id, _)| !self.slots.iter().any(|slot| slot.id == Some(*id)))
            {
                self.slots[index] = Slot {
                    id: Some(id),
                    since: now,
                    ..Slot::default()
                };
            }
        }
        // The weakest displayed process yields first; a new third-place contender
        // must never evict the strongest CPU user merely because it occupies slot 0.
        let mut replacement_order: Vec<_> = (0..N).collect();
        replacement_order.sort_by(|&a, &b| {
            let score = |i: usize| {
                self.slots[i]
                    .id
                    .and_then(|id| self.scores.get(&id))
                    .copied()
                    .unwrap_or(0.0)
            };
            score(a).total_cmp(&score(b))
        });
        for index in replacement_order {
            let candidate = ranked
                .iter()
                .find(|(id, _)| !self.slots.iter().any(|slot| slot.id == Some(*id)))
                .copied();
            let Some((candidate_id, candidate_score)) = candidate else {
                continue;
            };
            let slot = &mut self.slots[index];
            let Some(current_id) = slot.id else {
                continue;
            };
            let current = self.scores.get(&current_id).copied().unwrap_or(0.0);
            let margin = self
                .policy
                .absolute_margin
                .max(current * self.policy.relative_margin);
            if now - slot.since < self.policy.minimum_hold_seconds
                || candidate_score < self.policy.idle_floor
                || candidate_score <= current + margin
            {
                slot.challenger = None;
                slot.confirmations = 0;
                continue;
            }
            if slot.challenger == Some(candidate_id) {
                slot.confirmations += 1;
            } else {
                slot.challenger = Some(candidate_id);
                slot.confirmations = 1;
            }
            if slot.confirmations >= self.policy.confirmation_samples {
                *slot = Slot {
                    id: Some(candidate_id),
                    since: now,
                    ..Slot::default()
                };
            }
        }
        self.last_time = Some(now);
    }

    pub fn ids(&self) -> [Option<u64>; N] {
        self.slots.map(|slot| slot.id)
    }
    pub fn score(&self, id: u64) -> f32 {
        self.scores.get(&id).copied().unwrap_or(0.0)
    }
}
