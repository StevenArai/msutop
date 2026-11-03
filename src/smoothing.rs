//! Time-based EMA for displayed telemetry. Link states and memory pressure stay immediate.
use crate::metrics::{Cpu, Disk, Network};
use crate::Snapshot;
use std::collections::BTreeMap;

pub const CPU_TAU: f64 = 0.65;
pub const IO_TAU: f64 = 0.55;
pub const PROCESS_TAU: f64 = 0.90;

fn alpha(dt: f64, tau: f64) -> f64 {
    -(-dt / tau).exp_m1()
}
fn mix(before: f32, now: f32, amount: f64) -> f32 {
    if !before.is_finite() {
        return now;
    }
    if !now.is_finite() {
        return now;
    }
    (before as f64 + (now as f64 - before as f64) * amount) as f32
}
fn mix_option(before: Option<f32>, now: Option<f32>, amount: f64) -> Option<f32> {
    now.map(|now| before.map_or(now, |before| mix(before, now, amount)))
}

#[derive(Default)]
pub struct Smoother {
    previous: Option<Previous>,
    process_cpu: BTreeMap<u64, f32>,
}

struct Previous {
    seconds: f64,
    cpu: Cpu,
    disk: Disk,
    network: Network,
}

impl Smoother {
    pub fn apply(&mut self, raw: &Snapshot, selected: [Option<u64>; 2]) -> Snapshot {
        let Some(previous) = &self.previous else {
            self.process_cpu = raw
                .processes
                .iter()
                .filter(|p| selected.contains(&Some(p.id)))
                .map(|p| (p.id, p.cpu))
                .collect();
            self.previous = Some(Previous {
                seconds: raw.seconds,
                cpu: raw.cpu.clone(),
                disk: raw.disk.clone(),
                network: raw.network.clone(),
            });
            return raw.clone();
        };
        let dt = raw.seconds - previous.seconds;
        if !dt.is_finite() || dt <= 0.0 {
            let mut shown = raw.clone();
            shown.cpu = previous.cpu.clone();
            shown.disk = previous.disk.clone();
            shown.network = previous.network.clone();
            for p in &mut shown.processes {
                if let Some(value) = self.process_cpu.get(&p.id) {
                    p.cpu = *value;
                }
            }
            return shown;
        }
        let cpu = alpha(dt, CPU_TAU);
        let io = alpha(dt, IO_TAU);
        let process = alpha(dt, PROCESS_TAU);
        let mut shown = raw.clone();
        shown.cpu.usage = mix(previous.cpu.usage, raw.cpu.usage, cpu);
        shown.cpu.user = mix(previous.cpu.user, raw.cpu.user, cpu);
        shown.cpu.system = mix(previous.cpu.system, raw.cpu.system, cpu);
        shown.cpu.iowait = mix(previous.cpu.iowait, raw.cpu.iowait, cpu);
        if previous.cpu.cores.len() == raw.cpu.cores.len() {
            for (core, previous) in shown.cpu.cores.iter_mut().zip(&previous.cpu.cores) {
                core.usage = mix(previous.usage, core.usage, cpu);
                core.temperature = mix_option(previous.temperature, core.temperature, cpu);
            }
        }
        shown.cpu.frequency_mhz = mix(previous.cpu.frequency_mhz, raw.cpu.frequency_mhz, cpu);
        shown.cpu.temperature = mix_option(previous.cpu.temperature, raw.cpu.temperature, cpu);
        shown.cpu.power_watts = mix_option(previous.cpu.power_watts, raw.cpu.power_watts, cpu);
        if previous.disk.name == raw.disk.name {
            shown.disk.read_bps =
                previous.disk.read_bps + (raw.disk.read_bps - previous.disk.read_bps) * io;
            shown.disk.write_bps =
                previous.disk.write_bps + (raw.disk.write_bps - previous.disk.write_bps) * io;
            shown.disk.activity = mix(previous.disk.activity, raw.disk.activity, io);
            shown.disk.queue_depth =
                mix_option(previous.disk.queue_depth, raw.disk.queue_depth, io);
        }
        if raw.network.state.has_carrier()
            && previous.network.state.has_carrier()
            && previous.network.interface == raw.network.interface
        {
            shown.network.download_bps = previous.network.download_bps
                + (raw.network.download_bps - previous.network.download_bps) * io;
            shown.network.upload_bps = previous.network.upload_bps
                + (raw.network.upload_bps - previous.network.upload_bps) * io;
        }
        self.process_cpu
            .retain(|id, _| selected.contains(&Some(*id)));
        for p in &mut shown.processes {
            if !selected.contains(&Some(p.id)) {
                continue;
            }
            let value = self.process_cpu.entry(p.id).or_insert(p.cpu);
            *value = mix(*value, p.cpu, process);
            p.cpu = *value;
        }
        self.previous = Some(Previous {
            seconds: shown.seconds,
            cpu: shown.cpu.clone(),
            disk: shown.disk.clone(),
            network: shown.network.clone(),
        });
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::NetworkState;
    use crate::{MockSource, Scene};

    #[test]
    fn ema_has_same_response_at_different_fps_and_holds_when_paused() {
        let settle = |fps| {
            let mut smooth = Smoother::default();
            let mut raw = MockSource::snapshot(Scene::Balanced, 0.0);
            raw.cpu.usage = 0.0;
            raw.disk.read_bps = 0.0;
            smooth.apply(&raw, [None; 2]);
            for tick in 1..=fps {
                raw.seconds = tick as f64 / fps as f64;
                raw.cpu.usage = 100.0;
                raw.disk.read_bps = 1000.0;
                smooth.apply(&raw, [None; 2]);
            }
            let before = smooth.apply(&raw, [None; 2]);
            raw.cpu.usage = 0.0;
            assert_eq!(smooth.apply(&raw, [None; 2]).cpu.usage, before.cpu.usage);
            before
        };
        let a = settle(30);
        let b = settle(60);
        assert!((a.cpu.usage - b.cpu.usage).abs() < 0.0001);
        assert!((a.disk.read_bps - b.disk.read_bps).abs() < 0.0001);
        assert!(a.cpu.usage > 70.0 && a.cpu.usage < 90.0);
    }

    #[test]
    fn new_identity_and_disconnect_reset_and_pressure_remains_raw() {
        let mut smooth = Smoother::default();
        let mut raw = MockSource::snapshot(Scene::Balanced, 0.0);
        raw.processes[0].cpu = 99.0;
        smooth.apply(&raw, [Some(raw.processes[0].id), None]);
        raw.seconds = 1.0 / 30.0;
        raw.processes[0].id += 10000;
        raw.processes[0].cpu = 0.0;
        raw.network.state = NetworkState::Down;
        raw.network.download_bps = 0.0;
        raw.network.upload_bps = 0.0;
        raw.memory.used = raw.memory.total;
        raw.memory.free = 0;
        raw.memory.cached = 0;
        raw.cpu.power_watts = None;
        let shown = smooth.apply(&raw, [Some(raw.processes[0].id), None]);
        assert_eq!(shown.processes[0].cpu, 0.0);
        assert_eq!(shown.network.download_bps, 0.0);
        assert_eq!(shown.network.state, NetworkState::Down);
        assert!(shown.memory.available_is_low());
        assert_eq!(shown.cpu.power_watts, None);
    }
}
