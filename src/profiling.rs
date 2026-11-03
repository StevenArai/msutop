//! Profiling is opt-in; normal frames never query the thread CPU clock.
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default)]
pub struct Cost {
    pub wall_us: f64,
    pub cpu_us: f64,
}

pub struct Meter {
    started: Instant,
    cpu: f64,
    enabled: bool,
}

impl Meter {
    pub fn new(enabled: bool) -> Self {
        Self {
            started: Instant::now(),
            cpu: if enabled { thread_cpu_seconds() } else { 0.0 },
            enabled,
        }
    }
    pub fn elapsed(&self) -> Cost {
        Cost {
            wall_us: self.started.elapsed().as_secs_f64() * 1e6,
            cpu_us: if self.enabled {
                (thread_cpu_seconds() - self.cpu).max(0.0) * 1e6
            } else {
                0.0
            },
        }
    }
}

pub fn thread_cpu_seconds() -> f64 {
    #[cfg(target_os = "linux")]
    {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
        t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}

pub fn process_cpu_seconds() -> f64 {
    #[cfg(target_os = "linux")]
    {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
        t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}
