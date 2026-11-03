use std::f64::consts::TAU;

pub const GIB: f64 = 1_073_741_824.0;
pub const MIB: f64 = 1_048_576.0;
pub const LOW_AVAILABLE_PERCENT: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    Balanced,
    Idle,
    Cpu,
    Io,
    Memory,
    Offline,
}

impl Scene {
    pub const ALL: [Self; 6] = [
        Self::Balanced,
        Self::Idle,
        Self::Cpu,
        Self::Io,
        Self::Memory,
        Self::Offline,
    ];
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scene| scene.name() == value)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Idle => "idle",
            Self::Cpu => "cpu",
            Self::Io => "io",
            Self::Memory => "memory",
            Self::Offline => "offline",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Core {
    pub usage: f32,
    pub temperature: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct Cpu {
    pub usage: f32,
    pub user: f32,
    pub system: f32,
    pub iowait: f32,
    pub cores: Vec<Core>,
    pub temperature: Option<f32>,
    pub frequency_mhz: f32,
    pub load: [f32; 3],
    pub power_watts: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct Memory {
    pub total: u64,
    pub used: u64,
    pub cached: u64,
    pub free: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

impl Memory {
    /// Used excludes cache, so free plus reclaimable cache is the available pool.
    pub fn reclaimable(&self) -> u64 {
        self.free.saturating_add(self.cached).min(self.total)
    }

    pub fn available_is_low(&self) -> bool {
        shortage(self.reclaimable(), self.total)
    }

    pub fn swap_is_low(&self) -> bool {
        shortage(
            self.swap_total.saturating_sub(self.swap_used),
            self.swap_total,
        )
    }
}

fn shortage(remaining: u64, total: u64) -> bool {
    total != 0 && remaining as u128 * 100 < total as u128 * LOW_AVAILABLE_PERCENT as u128
}

#[derive(Clone, Debug)]
pub struct Disk {
    pub name: String,
    pub total: u64,
    pub used: u64,
    pub read_bps: f64,
    pub write_bps: f64,
    pub activity: f32,
    pub temperature: Option<f32>,
    /// Mean number of outstanding requests; the UI uses a fixed 0..8 display scale.
    pub queue_depth: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkState {
    Down,
    NoCarrier,
    Active,
    Transferring,
}

impl NetworkState {
    pub const ALL: [Self; 4] = [
        Self::Down,
        Self::NoCarrier,
        Self::Active,
        Self::Transferring,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.name() == value)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::NoCarrier => "nocarrier",
            Self::Active => "active",
            Self::Transferring => "transfer",
        }
    }

    pub fn status_label(self) -> &'static str {
        match self {
            Self::Down => "DOWN",
            Self::NoCarrier => "NOCARRIER",
            Self::Active | Self::Transferring => "ACTIVE",
        }
    }

    pub fn has_carrier(self) -> bool {
        matches!(self, Self::Active | Self::Transferring)
    }

    pub fn has_traffic(self) -> bool {
        self == Self::Transferring
    }
}

#[derive(Clone, Debug)]
pub struct Network {
    pub interface: String,
    pub ipv4: Option<String>,
    /// Administrative state, carrier, and whether this sample has packet traffic.
    pub state: NetworkState,
    /// Negotiated physical link rate, independent of measured traffic.
    pub link_mbps: Option<u32>,
    pub download_bps: f64,
    pub upload_bps: f64,
    pub downloaded: u64,
    pub uploaded: u64,
}

impl Network {
    pub fn link_label(&self) -> &'static str {
        if !self.state.has_carrier() {
            return "--";
        }
        match self.link_mbps {
            Some(100) => "FE",
            Some(1000..) => "GBE",
            _ => "--",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Battery {
    pub percent: f32,
    pub charging: bool,
}

#[derive(Clone, Debug)]
pub struct Process {
    /// Includes start time so PID reuse never inherits another process's history.
    pub id: u64,
    pub name: String,
    pub cpu: f32,
    pub rss: u64,
    pub threads: u32,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub seconds: f64,
    pub uptime: u64,
    pub cpu: Cpu,
    pub memory: Memory,
    pub disk: Disk,
    pub network: Network,
    pub battery: Option<Battery>,
    pub processes: Vec<Process>,
    pub process_count: usize,
}

/// A Linux /proc + /sys collector can replace the mock without changing any UI code.
pub trait TelemetrySource {
    fn sample(&mut self, seconds: f64) -> Snapshot;
}

pub struct MockSource {
    pub scene: Scene,
    pub network_state: Option<NetworkState>,
    pub threads: usize,
}

impl MockSource {
    pub fn new(scene: Scene) -> Self {
        Self {
            scene,
            network_state: None,
            threads: 8,
        }
    }

    pub fn snapshot(scene: Scene, t: f64) -> Snapshot {
        Self::snapshot_with_network(scene, t, None)
    }

    pub fn snapshot_with_network(scene: Scene, t: f64, state: Option<NetworkState>) -> Snapshot {
        // A compressed workload cycle makes subsecond mock changes visible at 30 Hz.
        // Timestamps and process-name hold times continue to use real elapsed seconds.
        let motion = t * 4.0;
        let wave = |period: f64, phase: f64| 0.5 + 0.5 * (motion * TAU / period + phase).sin();
        let pulse = |period: f64, phase: f64| wave(period, phase).powi(6);
        let normal = 27.0 + 20.0 * wave(43.0, 0.4) + 19.0 * pulse(17.0, 0.0);
        let usage = match scene {
            Scene::Idle => 2.5 + 5.0 * pulse(13.0, 0.3),
            Scene::Cpu => 81.0 + 18.0 * wave(24.0, 0.0),
            Scene::Io => 18.0 + 28.0 * wave(31.0, 0.0),
            Scene::Memory => 48.0 + 23.0 * wave(36.0, 0.0),
            Scene::Balanced | Scene::Offline => normal,
        } as f32;
        let temperature = 36.0 + usage * 0.47;
        let cpu = Cpu {
            usage,
            user: usage * if scene == Scene::Io { 0.52 } else { 0.72 },
            system: usage * 0.24,
            iowait: if scene == Scene::Io {
                usage * 0.24
            } else {
                usage * 0.04
            },
            cores: (0..8).map(|i| mock_core(usage, t, i)).collect(),
            temperature: Some(temperature),
            frequency_mhz: 1100.0 + 22.0 * usage,
            load: [usage * 0.032, usage * 0.023, 0.82 + usage * 0.007],
            power_watts: Some(3.0 + usage * 0.17),
        };
        let total = 8.0 * GIB;
        let used_ratio = match scene {
            Scene::Idle => 0.17 + 0.01 * wave(70.0, 0.0),
            Scene::Memory => 0.90 + 0.045 * wave(36.0, 0.0),
            _ => 0.44 + 0.018 * wave(80.0, 0.0),
        };
        let cached_ratio = if scene == Scene::Memory {
            0.035
        } else {
            0.17 + 0.015 * wave(55.0, 0.7)
        };
        let used = (total * used_ratio) as u64;
        let cached = (total * cached_ratio) as u64;
        let free = total as u64 - used - cached;
        let memory = Memory {
            total: total as u64,
            used,
            cached,
            free,
            available: free + cached,
            swap_total: (2.0 * GIB) as u64,
            swap_used: (2.0
                * GIB
                * if scene == Scene::Memory {
                    0.90 + 0.075 * wave(36.0, 0.0)
                } else if scene == Scene::Idle {
                    0.0
                } else {
                    0.12
                }) as u64,
        };
        let disk_read = match scene {
            Scene::Io => 140.0 + 150.0 * wave(18.0, 0.2),
            Scene::Idle => 0.02 + 0.2 * pulse(23.0, 0.0),
            _ => 0.3 + 34.0 * pulse(23.0, 0.0) + 4.0 * wave(11.0, 1.0),
        } * MIB;
        let disk_write = match scene {
            Scene::Io => 35.0 + 100.0 * wave(27.0, 1.7),
            Scene::Memory => 11.0 + 36.0 * wave(21.0, 0.6),
            Scene::Idle => 0.0,
            _ => 0.15 + 7.0 * pulse(19.0, 1.6),
        } * MIB;
        let disk = Disk {
            name: "ssd".into(),
            total: (128.0 * GIB) as u64,
            used: (79.3 * GIB) as u64,
            read_bps: disk_read,
            write_bps: disk_write,
            activity: ((disk_read + disk_write) / (400.0 * MIB) * 100.0).min(100.0) as f32,
            temperature: Some((31.0 + (disk_read + disk_write) / (400.0 * MIB) * 20.0) as f32),
            queue_depth: Some(match scene {
                Scene::Idle => (0.02 + 0.08 * pulse(23.0, 0.0)) as f32,
                Scene::Io => (4.0 + 4.0 * wave(18.0, 0.2)) as f32,
                Scene::Memory => (2.0 + 3.0 * wave(21.0, 0.6)) as f32,
                _ => (0.2 + 1.8 * pulse(23.0, 0.0) + 0.5 * wave(11.0, 1.0)) as f32,
            }),
        };
        let network_state = state.unwrap_or(match scene {
            Scene::Offline => NetworkState::Down,
            Scene::Idle => NetworkState::Active,
            _ => NetworkState::Transferring,
        });
        let (download_bps, upload_bps) = if network_state.has_traffic() {
            if scene == Scene::Io {
                (
                    (35.0 + 72.0 * wave(27.0, 0.5)) * MIB,
                    (5.0 + 19.0 * pulse(15.0, 1.7)) * MIB,
                )
            } else {
                (
                    (1.0 + 5.0 * wave(31.0, 0.4) + 11.0 * pulse(11.0, 0.2) + 3.0 * pulse(4.5, 1.1))
                        * MIB,
                    (0.15 + 1.3 * wave(23.0, 1.7) + 2.0 * pulse(7.5, 0.3)) * MIB,
                )
            }
        } else {
            (0.0, 0.0)
        };
        let network = Network {
            interface: "eth0".into(),
            ipv4: if network_state.has_carrier() {
                Some("192.168.1.42".into())
            } else {
                None
            },
            state: network_state,
            link_mbps: if network_state.has_carrier() {
                Some(if scene == Scene::Idle { 100 } else { 1000 })
            } else {
                None
            },
            download_bps,
            upload_bps,
            downloaded: (1.2 * GIB + t.max(0.0) * 9.0 * MIB) as u64,
            uploaded: (242.0 * MIB + t.max(0.0) * MIB) as u64,
        };
        let battery = if scene == Scene::Offline {
            None
        } else {
            Some(Battery {
                percent: 82.0,
                charging: true,
            })
        };
        let process_specs = match scene {
            Scene::Cpu => [
                ("ffmpeg", 63.0),
                ("rustc", 18.0),
                ("syncthing", 2.0),
                ("containerd", 0.8),
                ("msutop", 0.2),
            ],
            Scene::Io => [
                ("syncthing", 18.0),
                ("ffmpeg", 7.0),
                ("containerd", 3.0),
                ("rustc", 1.0),
                ("msutop", 0.2),
            ],
            Scene::Memory => [
                ("rustc", 33.0),
                ("ffmpeg", 17.0),
                ("syncthing", 3.0),
                ("containerd", 0.8),
                ("msutop", 0.2),
            ],
            Scene::Idle => [
                ("containerd", 0.9),
                ("syncthing", 0.8),
                ("msutop", 0.2),
                ("ffmpeg", 0.0),
                ("rustc", 0.0),
            ],
            _ => [
                ("ffmpeg", 22.0),
                ("syncthing", 11.0),
                ("containerd", 1.8),
                ("rustc", 0.6),
                ("msutop", 0.2),
            ],
        };
        let processes = process_specs
            .into_iter()
            .map(|(name, base)| {
                let id = match name {
                    "ffmpeg" => 1426,
                    "syncthing" => 901,
                    "containerd" => 632,
                    "rustc" => 1728,
                    _ => 1903,
                };
                let phase = id as f64 * 0.001;
                let cpu = (base * (0.72 + 0.46 * wave(19.0, phase) + 0.14 * pulse(6.0, phase))
                    + if base > 5.0 {
                        2.0 * wave(17.0, phase)
                    } else {
                        0.0
                    }) as f32;
                let rss_mib = match name {
                    "ffmpeg" => 412.0,
                    "syncthing" => 184.0,
                    "rustc" => {
                        if scene == Scene::Memory {
                            2460.0
                        } else {
                            682.0
                        }
                    }
                    "containerd" => 76.0,
                    _ => 4.0,
                };
                Process {
                    id,
                    name: name.into(),
                    cpu,
                    rss: (rss_mib * MIB) as u64,
                    threads: if base > 5.0 { 8 } else { 2 },
                }
            })
            .collect();
        Snapshot {
            seconds: t,
            uptime: 3 * 86400 + 5 * 3600 + t.max(0.0) as u64,
            cpu,
            memory,
            disk,
            network,
            battery,
            processes,
            process_count: 226,
        }
    }
}

impl TelemetrySource for MockSource {
    fn sample(&mut self, seconds: f64) -> Snapshot {
        let mut snapshot = Self::snapshot_with_network(self.scene, seconds, self.network_state);
        if self.threads != snapshot.cpu.cores.len() {
            snapshot.cpu.cores = (0..self.threads)
                .map(|index| mock_core(snapshot.cpu.usage, seconds, index))
                .collect();
        }
        snapshot
    }
}

fn mock_core(usage: f32, seconds: f64, index: usize) -> Core {
    let usage = (usage + 20.0 * (seconds * 4.0 * 0.16 + index as f64 * 1.33).sin() as f32)
        .clamp(0.0, 100.0);
    Core {
        usage,
        temperature: Some(34.0 + usage * 0.50),
    }
}
