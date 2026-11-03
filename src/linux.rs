//! Linux telemetry from procfs and sysfs. No privileged helpers run in the frame loop.
use crate::metrics::{
    Battery, Core, Cpu, Disk, Memory, Network, NetworkState, Process, Snapshot, TelemetrySource,
};
use crate::profiling::{Cost, Meter};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct LinuxOptions {
    pub mount: PathBuf,
    pub interface: Option<String>,
    pub disk: Option<String>,
    pub process_hz: u32,
}

impl Default for LinuxOptions {
    fn default() -> Self {
        Self {
            mount: "/".into(),
            interface: None,
            disk: None,
            process_hz: 1,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct CollectionCost {
    pub total_us: f64,
    pub fast_us: f64,
    pub processes_us: f64,
    pub sensors_us: f64,
    pub process_poll: bool,
    pub sensor_poll: bool,
    pub parts: [Cost; 7],
}

#[derive(Clone, Copy, Debug, Default)]
struct CpuTicks {
    total: u64,
    idle: u64,
    user: u64,
    system: u64,
    wait: u64,
}

impl CpuTicks {
    fn from_fields(fields: &[u64]) -> Self {
        let get = |i| fields.get(i).copied().unwrap_or(0);
        Self {
            total: fields.iter().take(8).sum(),
            idle: get(3) + get(4),
            user: get(0) + get(1),
            system: get(2) + get(5) + get(6),
            wait: get(4),
        }
    }

    fn delta(self, before: Self) -> (f32, f32, f32, f32) {
        let total = self.total.saturating_sub(before.total);
        if total == 0 {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let pct = |value: u64| (value as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32;
        (
            pct(total.saturating_sub(self.idle.saturating_sub(before.idle))),
            pct(self.user.saturating_sub(before.user)),
            pct(self.system.saturating_sub(before.system)),
            pct(self.wait.saturating_sub(before.wait)),
        )
    }
}

fn cpu_stat(text: &str) -> BTreeMap<String, CpuTicks> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            if name != "cpu" && !(name.starts_with("cpu") && name[3..].parse::<u32>().is_ok()) {
                return None;
            }
            let fields: Vec<_> = fields.filter_map(|field| field.parse().ok()).collect();
            (fields.len() >= 4).then(|| (name.to_owned(), CpuTicks::from_fields(&fields)))
        })
        .collect()
}

fn meminfo(text: &str) -> Memory {
    let mut values = [0_u64; 9];
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let index = match key {
            "MemTotal" => 0,
            "MemFree" => 1,
            "Cached" => 2,
            "Buffers" => 3,
            "SReclaimable" => 4,
            "Shmem" => 5,
            "MemAvailable" => 6,
            "SwapTotal" => 7,
            "SwapFree" => 8,
            _ => continue,
        };
        values[index] = rest
            .split_ascii_whitespace()
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            .saturating_mul(1024);
    }
    let total = values[0];
    let free = values[1].min(total);
    let cached = values[2]
        .saturating_add(values[3])
        .saturating_add(values[4])
        .saturating_sub(values[5])
        .min(total.saturating_sub(free));
    Memory {
        total,
        used: total.saturating_sub(free).saturating_sub(cached),
        cached,
        free,
        available: if values[6] == 0 {
            free + cached
        } else {
            values[6]
        },
        swap_total: values[7],
        swap_used: values[7].saturating_sub(values[8]),
    }
}

#[derive(Debug)]
struct ProcStat {
    pid: u32,
    name: String,
    ticks: u64,
    start: u64,
    rss_pages: u64,
    threads: u32,
}

fn process_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let pid = text[..open].trim().parse().ok()?;
    let mut fields = text.get(close + 1..)?.split_ascii_whitespace();
    let user: u64 = fields.nth(11)?.parse().ok()?;
    let system: u64 = fields.next()?.parse().ok()?;
    let threads = fields.nth(4)?.parse().ok()?;
    let start = fields.nth(1)?.parse().ok()?;
    fields.next()?;
    let rss = fields.next()?.parse::<i64>().ok()?.max(0) as u64;
    Some(ProcStat {
        pid,
        name: text[open + 1..close].to_owned(),
        ticks: user.saturating_add(system),
        start,
        rss_pages: rss,
        threads,
    })
}

#[derive(Clone, Copy, Default)]
struct IoTicks {
    read_sectors: u64,
    write_sectors: u64,
    busy_ms: u64,
    weighted_ms: u64,
}

fn block_stat(text: &str) -> Option<IoTicks> {
    let fields: Vec<u64> = text
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    Some(IoTicks {
        read_sectors: *fields.get(2)?,
        write_sectors: *fields.get(6)?,
        busy_ms: *fields.get(9)?,
        weighted_ms: *fields.get(10)?,
    })
}

impl IoTicks {
    fn add(&mut self, other: Self) {
        self.read_sectors += other.read_sectors;
        self.write_sectors += other.write_sectors;
        self.busy_ms += other.busy_ms;
        self.weighted_ms += other.weighted_ms;
    }
    fn rates(self, before: Self, seconds: f64) -> (f64, f64, f32, f32) {
        if seconds <= 0.0 {
            return (0.0, 0.0, 0.0, 0.0);
        }
        // Linux diskstats sectors are always 512 bytes, independent of physical sector size.
        (
            self.read_sectors.saturating_sub(before.read_sectors) as f64 * 512.0 / seconds,
            self.write_sectors.saturating_sub(before.write_sectors) as f64 * 512.0 / seconds,
            (self.busy_ms.saturating_sub(before.busy_ms) as f64 / (seconds * 10.0)).min(100.0)
                as f32,
            (self.weighted_ms.saturating_sub(before.weighted_ms) as f64 / (seconds * 1000.0))
                as f32,
        )
    }
}

#[derive(Clone, Copy, Default)]
struct NetTicks {
    rx: u64,
    tx: u64,
    rx_packets: u64,
    tx_packets: u64,
}

fn network_stat(text: &str, interface: &str) -> Option<NetTicks> {
    text.lines().find_map(|line| {
        let (name, rest) = line.split_once(':')?;
        if name.trim() != interface {
            return None;
        }
        let fields: Vec<u64> = rest
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        Some(NetTicks {
            rx: *fields.first()?,
            rx_packets: *fields.get(1)?,
            tx: *fields.get(8)?,
            tx_packets: *fields.get(9)?,
        })
    })
}

fn link_state(up: bool, carrier: bool, traffic: bool) -> NetworkState {
    match (up, carrier, traffic) {
        (false, _, _) => NetworkState::Down,
        (true, false, _) => NetworkState::NoCarrier,
        (true, true, false) => NetworkState::Active,
        (true, true, true) => NetworkState::Transferring,
    }
}

fn read_number<T: std::str::FromStr>(path: impl AsRef<Path>) -> Option<T> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

struct CachedText {
    file: fs::File,
    bytes: Vec<u8>,
}
impl CachedText {
    fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            file: fs::File::open(path)?,
            bytes: Vec::with_capacity(1024),
        })
    }
    fn read(&mut self) -> io::Result<&str> {
        self.file.seek(SeekFrom::Start(0))?;
        self.bytes.clear();
        let mut block = [0_u8; 4096];
        loop {
            match self.file.read(&mut block) {
                Ok(0) => break,
                Ok(size) => self.bytes.extend_from_slice(&block[..size]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        std::str::from_utf8(&self.bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

fn mount_unescape(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn mount_device(text: &str, mount: &Path) -> Option<String> {
    text.lines().find_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        (Path::new(&mount_unescape(fields.get(4)?)) == mount)
            .then(|| fields.get(2).map(|s| (*s).to_owned()))
            .flatten()
    })
}

fn disk_leaves(path: &Path, names: &mut BTreeSet<String>, visited: &mut BTreeSet<PathBuf>) {
    let Ok(path) = path.canonicalize() else {
        return;
    };
    if !visited.insert(path.clone()) {
        return;
    }
    if let Ok(slaves) = fs::read_dir(path.join("slaves")) {
        let slaves: Vec<_> = slaves.flatten().collect();
        if !slaves.is_empty() {
            for slave in slaves {
                disk_leaves(&slave.path(), names, visited);
            }
            return;
        }
    }
    let path = if path.join("partition").exists() {
        path.parent().unwrap_or(&path)
    } else {
        &path
    };
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        names.insert(name.to_owned());
    }
}

fn auto_interface(proc_root: &Path, sys_root: &Path) -> Option<String> {
    if let Ok(routes) = fs::read_to_string(proc_root.join("net/route")) {
        for line in routes.lines().skip(1) {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.get(1) == Some(&"00000000")
                && fields
                    .get(3)
                    .and_then(|f| u32::from_str_radix(f, 16).ok())
                    .is_some_and(|f| f & 1 != 0)
            {
                return fields.first().map(|s| (*s).to_owned());
            }
        }
    }
    let interfaces: BTreeSet<_> = fs::read_dir(sys_root.join("class/net"))
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| name != "lo")
        .collect();
    interfaces
        .iter()
        .find(|name| {
            read_number::<u32>(sys_root.join("class/net").join(name).join("carrier")) == Some(1)
        })
        .cloned()
        .or_else(|| interfaces.first().cloned())
}

struct Temperature {
    path: PathBuf,
    core: Option<usize>,
}
struct Energy {
    path: PathBuf,
    maximum: u64,
    previous: Option<(u64, Instant)>,
}
struct ProcessPrevious {
    id: u64,
    ticks: u64,
}

struct WatchedProcess {
    start: u64,
    id: u64,
    cpu: CpuWindow,
    file: CachedText,
}

const PROCESS_CPU_WINDOW: Duration = Duration::from_millis(250);

struct CpuWindow {
    samples: VecDeque<(Instant, u64)>,
}
impl CpuWindow {
    fn new(time: Instant, ticks: u64) -> Self {
        Self {
            samples: VecDeque::from([(time, ticks)]),
        }
    }
    fn observe(&mut self, time: Instant, ticks: u64, tick_hz: u64) -> f32 {
        if let Some(&(previous_time, previous_ticks)) = self.samples.back() {
            if ticks < previous_ticks {
                self.samples.clear();
            } else if time <= previous_time {
                return 0.0;
            }
        }
        self.samples.push_back((time, ticks));
        while self.samples.len() > 2 && time.duration_since(self.samples[1].0) >= PROCESS_CPU_WINDOW
        {
            self.samples.pop_front();
        }
        let Some(&(before, old_ticks)) = self.samples.front() else {
            return 0.0;
        };
        let seconds = time.saturating_duration_since(before).as_secs_f64();
        if seconds <= 0.0 {
            return 0.0;
        }
        (ticks.saturating_sub(old_ticks) as f64 / tick_hz as f64 / seconds * 100.0) as f32
    }
}

struct SensorHelper {
    child: Child,
    receiver: Receiver<serde_json::Value>,
    latest: Option<(serde_json::Value, Instant)>,
}
impl Drop for SensorHelper {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
    }
}

pub struct LinuxSource {
    config: LinuxOptions,
    proc_root: PathBuf,
    sys_root: PathBuf,
    tick_hz: u64,
    page_size: u64,
    snapshot: Snapshot,
    cpu_previous: BTreeMap<String, CpuTicks>,
    io_previous: Option<IoTicks>,
    net_previous: Option<NetTicks>,
    fast_time: Option<Instant>,
    process_time: Option<Instant>,
    process_tick: Option<u64>,
    process_previous: BTreeMap<(u32, u64), ProcessPrevious>,
    process_files: BTreeMap<u32, CachedText>,
    ranking: Vec<Process>,
    watched_processes: Vec<WatchedProcess>,
    next_process_id: u64,
    sensor_tick: Option<u64>,
    disks: Vec<String>,
    temperatures: Vec<Temperature>,
    disk_temperatures: Vec<PathBuf>,
    energy: Vec<Energy>,
    cost: CollectionCost,
    pub read_errors: u64,
    profiling: bool,
    cpu_file: CachedText,
    memory_file: CachedText,
    network_file: CachedText,
    disk_files: Vec<CachedText>,
    carrier_file: Option<CachedText>,
    flags_file: Option<CachedText>,
    helper: Option<SensorHelper>,
}

impl LinuxSource {
    pub fn new(config: LinuxOptions) -> io::Result<Self> {
        Self::with_roots(config, "/proc".into(), "/sys".into())
    }

    fn with_roots(
        mut config: LinuxOptions,
        proc_root: PathBuf,
        sys_root: PathBuf,
    ) -> io::Result<Self> {
        let initial_cpu = cpu_stat(&fs::read_to_string(proc_root.join("stat"))?);
        if !initial_cpu.contains_key("cpu") {
            return Err(io::Error::other("/proc/stat has no CPU counters"));
        }
        fs::read_to_string(proc_root.join("meminfo"))?;
        config.mount = config.mount.canonicalize()?;
        let mut disks = BTreeSet::new();
        let block = if let Some(name) = &config.disk {
            sys_root.join("class/block").join(name)
        } else {
            let text = fs::read_to_string(proc_root.join("self/mountinfo"))?;
            let device = mount_device(&text, &config.mount)
                .ok_or_else(|| io::Error::other("mount is not present in mountinfo"))?;
            sys_root.join("dev/block").join(device)
        };
        disk_leaves(&block, &mut disks, &mut BTreeSet::new());
        if config.disk.is_some() && disks.is_empty() {
            return Err(io::Error::other("selected block device does not exist"));
        }
        let interface = config
            .interface
            .clone()
            .or_else(|| auto_interface(&proc_root, &sys_root))
            .unwrap_or_else(|| "lo".into());
        if !sys_root.join("class/net").join(&interface).exists() {
            return Err(io::Error::other(
                "selected network interface does not exist",
            ));
        }
        let mut source = Self {
            cpu_file: CachedText::open(proc_root.join("stat"))?,
            memory_file: CachedText::open(proc_root.join("meminfo"))?,
            network_file: CachedText::open(proc_root.join("net/dev"))?,
            disk_files: disks
                .iter()
                .filter_map(|disk| {
                    CachedText::open(sys_root.join("class/block").join(disk).join("stat")).ok()
                })
                .collect(),
            carrier_file: CachedText::open(
                sys_root.join("class/net").join(&interface).join("carrier"),
            )
            .ok(),
            flags_file: CachedText::open(sys_root.join("class/net").join(&interface).join("flags"))
                .ok(),
            helper: None,
            snapshot: Snapshot {
                seconds: 0.0,
                uptime: 0,
                cpu: Cpu {
                    usage: 0.0,
                    user: 0.0,
                    system: 0.0,
                    iowait: 0.0,
                    cores: Vec::new(),
                    temperature: None,
                    frequency_mhz: f32::NAN,
                    load: [0.0; 3],
                    power_watts: None,
                },
                memory: meminfo(""),
                disk: Disk {
                    name: disks.iter().cloned().collect::<Vec<_>>().join("+"),
                    total: 0,
                    used: 0,
                    read_bps: 0.0,
                    write_bps: 0.0,
                    activity: 0.0,
                    temperature: None,
                    queue_depth: None,
                },
                network: Network {
                    interface,
                    ipv4: None,
                    state: NetworkState::Down,
                    link_mbps: None,
                    download_bps: 0.0,
                    upload_bps: 0.0,
                    downloaded: 0,
                    uploaded: 0,
                },
                battery: None,
                processes: Vec::new(),
                process_count: 0,
            },
            config,
            proc_root,
            sys_root,
            tick_hz: rustix::param::clock_ticks_per_second(),
            page_size: rustix::param::page_size() as u64,
            cpu_previous: BTreeMap::new(),
            io_previous: None,
            net_previous: None,
            fast_time: None,
            process_time: None,
            process_tick: None,
            process_previous: BTreeMap::new(),
            process_files: BTreeMap::new(),
            ranking: Vec::new(),
            watched_processes: Vec::new(),
            next_process_id: 1,
            sensor_tick: None,
            disks: disks.into_iter().collect(),
            temperatures: Vec::new(),
            disk_temperatures: Vec::new(),
            energy: Vec::new(),
            cost: CollectionCost::default(),
            read_errors: 0,
            profiling: false,
        };
        source.discover_sensors();
        source.collect(0.0);
        Ok(source)
    }

    pub fn cost(&self) -> CollectionCost {
        self.cost
    }
    pub fn set_profiling(&mut self, enabled: bool) {
        self.profiling = enabled;
    }
    pub fn helper_status(&self) -> Option<&serde_json::Value> {
        self.helper
            .as_ref()?
            .latest
            .as_ref()
            .map(|(value, _)| value)
    }

    pub fn start_sensor_helper(&mut self, script: &Path, sudo: bool) -> io::Result<()> {
        let script = script.canonicalize()?;
        let mut command = if sudo {
            let mut cmd = Command::new("sudo");
            cmd.args(["-n", "/usr/bin/python3"]);
            cmd
        } else {
            Command::new("/usr/bin/python3")
        };
        command.args(["-I", "-u"]).arg(script);
        for disk in &self.disks {
            command.arg("--disk").arg(format!("/dev/{disk}"));
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("sensor helper has no stdout"))?;
        let (sender, receiver) = mpsc::sync_channel(2);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str(&line) {
                    let _ = sender.try_send(value);
                }
            }
        });
        self.helper = Some(SensorHelper {
            child,
            receiver,
            latest: None,
        });
        Ok(())
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub fn ranking_processes(&self) -> &[Process] {
        &self.ranking
    }
    pub fn complete_display(&self, shown: &mut Snapshot, selected: [Option<u64>; 2]) {
        for id in selected.into_iter().flatten() {
            if !shown.processes.iter().any(|p| p.id == id) {
                if let Some(process) = self.snapshot.processes.iter().find(|p| p.id == id) {
                    shown.processes.push(process.clone());
                }
            }
        }
    }
    pub fn process_hz(&self) -> u32 {
        self.config.process_hz
    }

    /// Scan every process only at the ranking rate; refresh displayed identities each frame.
    pub fn watch_processes(&mut self, ids: [Option<u64>; 2]) {
        self.watched_processes.retain(|p| ids.contains(&Some(p.id)));
        for id in ids.into_iter().flatten() {
            if self.watched_processes.iter().any(|p| p.id == id) {
                continue;
            }
            if let Some((&(pid, start), _)) = self.process_previous.iter().find(|(_, p)| p.id == id)
            {
                let Ok(mut file) =
                    CachedText::open(self.proc_root.join(pid.to_string()).join("stat"))
                else {
                    continue;
                };
                let Some(stat) = file
                    .read()
                    .ok()
                    .and_then(process_stat)
                    .filter(|p| p.start == start)
                else {
                    continue;
                };
                self.watched_processes.push(WatchedProcess {
                    start,
                    id,
                    cpu: CpuWindow::new(Instant::now(), stat.ticks),
                    file,
                });
            }
        }
    }
    pub fn own_cpu_seconds(&self) -> f64 {
        fs::read_to_string(self.proc_root.join("self/stat"))
            .ok()
            .and_then(|s| process_stat(&s))
            .map_or(0.0, |p| p.ticks as f64 / self.tick_hz as f64)
    }

    fn discover_sensors(&mut self) {
        if let Ok(entries) = fs::read_dir(self.sys_root.join("class/hwmon")) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = fs::read_to_string(path.join("name")).unwrap_or_default();
                let cpu = ["coretemp", "k10temp", "cpu_thermal", "soc_dts", "zenpower"]
                    .iter()
                    .any(|kind| name.trim().starts_with(kind));
                let disk = ["nvme", "drivetemp"].contains(&name.trim());
                let matches_disk =
                    fs::canonicalize(path.join("device"))
                        .ok()
                        .is_some_and(|device| {
                            self.disks.iter().any(|name| {
                                fs::canonicalize(
                                    self.sys_root.join("class/block").join(name).join("device"),
                                )
                                .ok()
                                .is_some_and(|block| {
                                    device == block
                                        || device.starts_with(&block)
                                        || block.starts_with(&device)
                                })
                            })
                        });
                if let Ok(files) = fs::read_dir(&path) {
                    for file in files.flatten() {
                        let file_name = file.file_name();
                        let file_name = file_name.to_string_lossy();
                        if !file_name.starts_with("temp") || !file_name.ends_with("_input") {
                            continue;
                        }
                        if cpu {
                            let label = fs::read_to_string(
                                path.join(file_name.replace("_input", "_label")),
                            )
                            .unwrap_or_default();
                            let core = label
                                .trim()
                                .strip_prefix("Core ")
                                .and_then(|value| value.parse().ok());
                            self.temperatures.push(Temperature {
                                path: file.path(),
                                core,
                            });
                        } else if disk && matches_disk {
                            self.disk_temperatures.push(file.path());
                        }
                    }
                }
            }
        }
        if self.temperatures.is_empty() {
            if let Ok(entries) = fs::read_dir(self.sys_root.join("class/thermal")) {
                for entry in entries.flatten() {
                    let kind = fs::read_to_string(entry.path().join("type"))
                        .unwrap_or_default()
                        .to_lowercase();
                    if kind.contains("cpu") || kind.contains("pkg") || kind.contains("x86") {
                        self.temperatures.push(Temperature {
                            path: entry.path().join("temp"),
                            core: None,
                        });
                    }
                }
            }
        }
        if let Ok(entries) = fs::read_dir(self.sys_root.join("class/powercap")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !fs::read_to_string(path.join("name"))
                    .unwrap_or_default()
                    .starts_with("package")
                {
                    continue;
                }
                if let Some(maximum) = read_number(path.join("max_energy_range_uj")) {
                    self.energy.push(Energy {
                        path: path.join("energy_uj"),
                        maximum,
                        previous: None,
                    });
                }
            }
        }
    }

    fn collect(&mut self, seconds: f64) {
        let started = Instant::now();
        let mut meter = Meter::new(self.profiling);
        self.cost = CollectionCost::default();
        let elapsed = self
            .fast_time
            .map_or(0.0, |before| started.duration_since(before).as_secs_f64());
        self.snapshot.seconds = seconds;
        match self.cpu_file.read() {
            Ok(text) => {
                let current = cpu_stat(text);
                if let Some(cpu) = current.get("cpu") {
                    let (usage, user, system, wait) =
                        cpu.delta(self.cpu_previous.get("cpu").copied().unwrap_or(*cpu));
                    self.snapshot.cpu.usage = usage;
                    self.snapshot.cpu.user = user;
                    self.snapshot.cpu.system = system;
                    self.snapshot.cpu.iowait = wait;
                }
                let mut cores: Vec<_> = current
                    .iter()
                    .filter_map(|(name, ticks)| {
                        let id: usize = name.strip_prefix("cpu")?.parse().ok()?;
                        let before = self.cpu_previous.get(name).copied().unwrap_or(*ticks);
                        let temperature = self
                            .snapshot
                            .cpu
                            .cores
                            .get(id)
                            .and_then(|core| core.temperature);
                        Some((
                            id,
                            Core {
                                usage: ticks.delta(before).0,
                                temperature,
                            },
                        ))
                    })
                    .collect();
                cores.sort_by_key(|(id, _)| *id);
                self.snapshot.cpu.cores = cores.into_iter().map(|(_, core)| core).collect();
                self.cpu_previous = current;
            }
            Err(_) => self.read_errors += 1,
        }
        self.cost.parts[0] = meter.elapsed();
        meter = Meter::new(self.profiling);
        match self.memory_file.read() {
            Ok(text) => self.snapshot.memory = meminfo(text),
            Err(_) => self.read_errors += 1,
        }
        self.cost.parts[1] = meter.elapsed();
        meter = Meter::new(self.profiling);
        let mut io = IoTicks::default();
        let mut any_io = false;
        for disk in &mut self.disk_files {
            if let Some(current) = disk.read().ok().and_then(block_stat) {
                io.add(current);
                any_io = true;
            }
        }
        if any_io {
            let (read, write, activity, queue) = io.rates(self.io_previous.unwrap_or(io), elapsed);
            self.snapshot.disk.read_bps = read;
            self.snapshot.disk.write_bps = write;
            self.snapshot.disk.activity = activity;
            self.snapshot.disk.queue_depth = Some(queue);
            self.io_previous = Some(io);
        } else {
            self.snapshot.disk.queue_depth = None;
        }
        self.cost.parts[2] = meter.elapsed();
        meter = Meter::new(self.profiling);
        let interface = &self.snapshot.network.interface;
        let flags = self
            .flags_file
            .as_mut()
            .and_then(|file| file.read().ok())
            .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        let carrier = self
            .carrier_file
            .as_mut()
            .and_then(|file| file.read().ok())
            .and_then(|s| s.trim().parse::<u32>().ok())
            == Some(1);
        let mut traffic = false;
        if let Some(current) = self
            .network_file
            .read()
            .ok()
            .and_then(|s| network_stat(s, interface))
        {
            let previous = self.net_previous.unwrap_or(current);
            if elapsed > 0.0 {
                self.snapshot.network.download_bps =
                    current.rx.saturating_sub(previous.rx) as f64 / elapsed;
                self.snapshot.network.upload_bps =
                    current.tx.saturating_sub(previous.tx) as f64 / elapsed;
            }
            traffic = current.rx_packets > previous.rx_packets
                || current.tx_packets > previous.tx_packets;
            self.snapshot.network.downloaded = current.rx;
            self.snapshot.network.uploaded = current.tx;
            self.net_previous = Some(current);
        }
        self.snapshot.network.state = link_state(flags & 1 != 0, carrier, traffic);
        if !self.snapshot.network.state.has_carrier() {
            self.snapshot.network.download_bps = 0.0;
            self.snapshot.network.upload_bps = 0.0;
        }
        self.cost.parts[3] = meter.elapsed();
        self.fast_time = Some(started);
        self.cost.fast_us = started.elapsed().as_secs_f64() * 1e6;
        let process_tick =
            (seconds.max(0.0) * self.config.process_hz.clamp(1, 60) as f64 + 1e-7).floor() as u64;
        if self.process_tick != Some(process_tick) {
            let before = Instant::now();
            meter = Meter::new(self.profiling);
            self.collect_processes();
            self.cost.parts[4] = meter.elapsed();
            self.cost.processes_us = before.elapsed().as_secs_f64() * 1e6;
            self.cost.process_poll = true;
            self.process_tick = Some(process_tick);
        }
        let before = Instant::now();
        meter = Meter::new(self.profiling);
        self.collect_watched_processes();
        self.cost.parts[5] = meter.elapsed();
        self.cost.fast_us += before.elapsed().as_secs_f64() * 1e6;
        let sensor_tick = seconds.max(0.0).floor() as u64;
        if self.sensor_tick != Some(sensor_tick) {
            let before = Instant::now();
            meter = Meter::new(self.profiling);
            self.collect_sensors();
            self.cost.parts[6] = meter.elapsed();
            self.cost.sensors_us = before.elapsed().as_secs_f64() * 1e6;
            self.cost.sensor_poll = true;
            self.sensor_tick = Some(sensor_tick);
        }
        self.cost.total_us = started.elapsed().as_secs_f64() * 1e6;
    }

    fn collect_watched_processes(&mut self) {
        for watched in &mut self.watched_processes {
            let now = Instant::now();
            let stat = watched.file.read().ok().and_then(process_stat);
            let Some(stat) = stat.filter(|stat| stat.start == watched.start) else {
                self.snapshot.processes.retain(|p| p.id != watched.id);
                continue;
            };
            let cpu = watched.cpu.observe(now, stat.ticks, self.tick_hz);
            if let Some(process) = self
                .snapshot
                .processes
                .iter_mut()
                .find(|p| p.id == watched.id)
            {
                process.cpu = cpu;
                process.rss = stat.rss_pages.saturating_mul(self.page_size);
                process.threads = stat.threads;
            }
        }
    }

    fn collect_processes(&mut self) {
        let now = Instant::now();
        let elapsed = self
            .process_time
            .map_or(0.0, |before| now.duration_since(before).as_secs_f64());
        let Ok(entries) = fs::read_dir(&self.proc_root) else {
            self.read_errors += 1;
            return;
        };
        let mut current = BTreeMap::new();
        let mut processes = Vec::new();
        let mut active_pids = BTreeSet::new();
        let mut count = 0;
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            active_pids.insert(pid);
            count += 1;
            if let std::collections::btree_map::Entry::Vacant(slot) = self.process_files.entry(pid)
            {
                let Ok(file) = CachedText::open(entry.path().join("stat")) else {
                    continue;
                };
                slot.insert(file);
            }
            let stat = self
                .process_files
                .get_mut(&pid)
                .and_then(|file| file.read().ok())
                .and_then(process_stat);
            let stat = stat.or_else(|| {
                // A dead proc inode must be reopened if Linux reused the PID.
                let mut file = CachedText::open(entry.path().join("stat")).ok()?;
                let stat = process_stat(file.read().ok()?)?;
                self.process_files.insert(pid, file);
                Some(stat)
            });
            let Some(stat) = stat else {
                self.process_files.remove(&pid);
                continue;
            };
            let key = (stat.pid, stat.start);
            let previous = self.process_previous.get(&key);
            let id = previous.map_or_else(
                || {
                    let id = self.next_process_id;
                    self.next_process_id += 1;
                    id
                },
                |p| p.id,
            );
            let cpu = previous.filter(|_| elapsed > 0.0).map_or(0.0, |before| {
                stat.ticks.saturating_sub(before.ticks) as f64 / self.tick_hz as f64 / elapsed
                    * 100.0
            }) as f32;
            current.insert(
                key,
                ProcessPrevious {
                    id,
                    ticks: stat.ticks,
                },
            );
            processes.push(Process {
                id,
                name: stat.name,
                cpu,
                rss: stat.rss_pages.saturating_mul(self.page_size),
                threads: stat.threads,
            });
        }
        self.snapshot.process_count = count;
        self.ranking = processes.clone();
        self.snapshot.processes = processes;
        self.process_files
            .retain(|pid, _| active_pids.contains(pid));
        self.process_previous = current;
        self.process_time = Some(now);
    }

    fn collect_sensors(&mut self) {
        if let Some(uptime) = fs::read_to_string(self.proc_root.join("uptime"))
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        {
            self.snapshot.uptime = uptime.max(0.0) as u64;
        }
        if let Ok(load) = fs::read_to_string(self.proc_root.join("loadavg")) {
            for (field, value) in self
                .snapshot
                .cpu
                .load
                .iter_mut()
                .zip(load.split_whitespace().take(3))
            {
                *field = value.parse().unwrap_or(0.0);
            }
        }
        let mut temperatures = Vec::new();
        for core in &mut self.snapshot.cpu.cores {
            core.temperature = None;
        }
        for sensor in &self.temperatures {
            if let Some(value) = read_number::<f32>(&sensor.path)
                .map(|v| v / 1000.0)
                .filter(|v| (-20.0..150.0).contains(v))
            {
                temperatures.push(value);
                if let Some(core) = sensor.core.and_then(|i| self.snapshot.cpu.cores.get_mut(i)) {
                    core.temperature = Some(value);
                }
            }
        }
        self.snapshot.cpu.temperature = temperatures.into_iter().max_by(f32::total_cmp);
        self.snapshot.disk.temperature = self
            .disk_temperatures
            .iter()
            .filter_map(|p| read_number::<f32>(p).map(|v| v / 1000.0))
            .filter(|v| (0.0..150.0).contains(v))
            .max_by(f32::total_cmp);
        let mut frequencies = Vec::new();
        for core in 0..self.snapshot.cpu.cores.len() {
            if let Some(value) = read_number::<f32>(self.sys_root.join(format!(
                "devices/system/cpu/cpu{core}/cpufreq/scaling_cur_freq"
            ))) {
                frequencies.push(value / 1000.0);
            }
        }
        if frequencies.is_empty() {
            if let Ok(text) = fs::read_to_string(self.proc_root.join("cpuinfo")) {
                frequencies.extend(text.lines().filter_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    (key.trim() == "cpu MHz")
                        .then(|| value.trim().parse::<f32>().ok())
                        .flatten()
                }));
            }
        }
        self.snapshot.cpu.frequency_mhz = if frequencies.is_empty() {
            f32::NAN
        } else {
            frequencies.iter().sum::<f32>() / frequencies.len() as f32
        };
        let now = Instant::now();
        let mut watts = 0.0;
        let mut known = false;
        for energy in &mut self.energy {
            if let Some(value) = read_number::<u64>(&energy.path) {
                if let Some((previous, time)) = energy.previous {
                    let elapsed = now.duration_since(time).as_secs_f64();
                    if elapsed > 0.0 {
                        let delta = if value >= previous {
                            value - previous
                        } else {
                            energy
                                .maximum
                                .saturating_sub(previous)
                                .saturating_add(value)
                        };
                        watts += delta as f64 / 1e6 / elapsed;
                        known = true;
                    }
                }
                energy.previous = Some((value, now));
            } else {
                energy.previous = None;
            }
        }
        self.snapshot.cpu.power_watts = known.then_some(watts as f32);
        if let Some(helper) = &mut self.helper {
            for value in helper.receiver.try_iter() {
                helper.latest = Some((value, Instant::now()));
            }
            if let Some((value, time)) = &helper.latest {
                if time.elapsed() < Duration::from_secs(4) {
                    if self.snapshot.cpu.power_watts.is_none() {
                        self.snapshot.cpu.power_watts = value
                            .get("cpu_watts")
                            .and_then(serde_json::Value::as_f64)
                            .filter(|v| v.is_finite() && *v >= 0.0)
                            .map(|v| v as f32);
                    }
                    if self.snapshot.disk.temperature.is_none() {
                        self.snapshot.disk.temperature = value
                            .get("disk_temperature")
                            .and_then(serde_json::Value::as_f64)
                            .filter(|v| (0.0..150.0).contains(v))
                            .map(|v| v as f32);
                    }
                }
            }
        }
        if let Ok(stat) = rustix::fs::statvfs(&self.config.mount) {
            let size = if stat.f_frsize != 0 {
                stat.f_frsize
            } else {
                stat.f_bsize
            };
            self.snapshot.disk.total = stat.f_blocks.saturating_mul(size);
            self.snapshot.disk.used = stat
                .f_blocks
                .saturating_sub(stat.f_bfree)
                .saturating_mul(size);
        }
        let net = self
            .sys_root
            .join("class/net")
            .join(&self.snapshot.network.interface);
        let mut speed = read_number::<i64>(net.join("speed"))
            .filter(|v| *v > 0)
            .map(|v| v as u32);
        if speed.is_none() {
            if let Ok(ports) = fs::read_dir(net.join("brif")) {
                speed = ports
                    .flatten()
                    .filter_map(|p| {
                        let port = self.sys_root.join("class/net").join(p.file_name());
                        (read_number::<u32>(port.join("carrier")) == Some(1))
                            .then(|| read_number::<i64>(port.join("speed")))
                            .flatten()
                            .filter(|v| *v > 0)
                            .map(|v| v as u32)
                    })
                    .max();
            }
        }
        self.snapshot.network.link_mbps = speed;
        self.snapshot.battery = fs::read_dir(self.sys_root.join("class/power_supply"))
            .ok()
            .and_then(|entries| {
                entries.flatten().find_map(|entry| {
                    if fs::read_to_string(entry.path().join("type")).ok()?.trim() != "Battery" {
                        return None;
                    }
                    Some(Battery {
                        percent: read_number::<f32>(entry.path().join("capacity"))?,
                        charging: fs::read_to_string(entry.path().join("status")).ok()?.trim()
                            == "Charging",
                    })
                })
            });
    }
}

impl TelemetrySource for LinuxSource {
    fn sample(&mut self, seconds: f64) -> Snapshot {
        self.collect(seconds);
        let processes = if self.watched_processes.is_empty() {
            self.snapshot.processes.iter().take(2).cloned().collect()
        } else {
            self.snapshot
                .processes
                .iter()
                .filter(|p| self.watched_processes.iter().any(|w| w.id == p.id))
                .cloned()
                .collect()
        };
        // Keep the full ranking internally at 1 Hz; never clone it at 30 Hz.
        Snapshot {
            seconds: self.snapshot.seconds,
            uptime: self.snapshot.uptime,
            cpu: self.snapshot.cpu.clone(),
            memory: self.snapshot.memory.clone(),
            disk: self.snapshot.disk.clone(),
            network: self.snapshot.network.clone(),
            battery: self.snapshot.battery.clone(),
            processes,
            process_count: self.snapshot.process_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_window_averages_tick_quantization_instead_of_zero_thirty_percent_pulses() {
        let started = Instant::now();
        let mut window = CpuWindow::new(started, 0);
        for frame in 1..=120 {
            let value = window.observe(
                started + Duration::from_secs_f64(frame as f64 / 30.0),
                frame / 8,
                100,
            );
            if frame >= 16 {
                assert!((value - 3.75).abs() < 0.001, "frame {frame}: {value}");
            }
            assert!(window.samples.len() <= 10);
        }
        assert_eq!(
            window.observe(started + Duration::from_secs(5), 0, 100),
            0.0
        );
    }

    #[test]
    fn cpu_deltas_exclude_idle_and_do_not_double_count_guest() {
        let before = CpuTicks::from_fields(&[100, 0, 50, 800, 50, 0, 0, 0, 20, 0]);
        let after = CpuTicks::from_fields(&[120, 0, 60, 860, 60, 0, 0, 0, 25, 0]);
        assert_eq!(after.delta(before), (30.0, 20.0, 10.0, 10.0));
        assert_eq!(after.delta(after), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn reclaimable_memory_excludes_shared_memory_and_counts_buffers() {
        let m = meminfo("MemTotal: 1000 kB\nMemFree: 100 kB\nCached: 200 kB\nBuffers: 20 kB\nSReclaimable: 50 kB\nShmem: 30 kB\nMemAvailable: 320 kB\nSwapTotal: 100 kB\nSwapFree: 80 kB\n");
        assert_eq!(m.cached, 240 * 1024);
        assert_eq!(m.used + m.cached + m.free, m.total);
        assert_eq!(m.available, 320 * 1024);
        assert_eq!(m.swap_used, 20 * 1024);
    }

    #[test]
    fn process_parser_handles_spaces_parentheses_and_signed_rss() {
        let text = "123 (worker (IO)) S 1 1 1 0 -1 0 0 0 0 0 25 15 0 0 20 0 8 0 9876 65536 42";
        let p = process_stat(text).unwrap();
        assert_eq!(p.pid, 123);
        assert_eq!(p.name, "worker (IO)");
        assert_eq!(p.ticks, 40);
        assert_eq!(p.threads, 8);
        assert_eq!(p.start, 9876);
        assert_eq!(p.rss_pages, 42);
        assert!(process_stat("123 (gone)").is_none());
    }

    #[test]
    fn disk_rate_uses_512_byte_sectors_and_weighted_queue_time() {
        let before = block_stat("0 0 100 0 0 0 200 0 0 10 20").unwrap();
        let after = block_stat("0 0 104 0 0 0 208 0 0 30 60 0 0 0 0").unwrap();
        assert_eq!(after.rates(before, 0.1), (20480.0, 40960.0, 20.0, 0.4));
    }

    #[test]
    fn network_state_uses_administrative_state_carrier_and_packet_activity() {
        let stats =
            network_stat("  eth0: 1024 2 0 0 0 0 0 0 2048 3 0 0 0 0 0 0\n", "eth0").unwrap();
        assert_eq!(
            (stats.rx, stats.tx, stats.rx_packets, stats.tx_packets),
            (1024, 2048, 2, 3)
        );
        assert_eq!(link_state(false, true, true), NetworkState::Down);
        assert_eq!(link_state(true, false, true), NetworkState::NoCarrier);
        assert_eq!(link_state(true, true, false), NetworkState::Active);
        assert_eq!(link_state(true, true, true), NetworkState::Transferring);
    }

    #[test]
    fn mount_lookup_decodes_spaces_and_uses_device_numbers() {
        assert_eq!(
            mount_device(
                "29 1 252:0 / /data\\040disk rw - ext4 /dev/mapper/root rw",
                Path::new("/data disk")
            ),
            Some("252:0".into())
        );
    }
}
