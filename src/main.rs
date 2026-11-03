use msutop::frame::{FrameSink, Rgb565Writer};
use msutop::rawfb::AckMode;
use msutop::ui::{DEFAULT_FPS, HISTORY_SAMPLES};
use msutop::{
    Dashboard, Frame, MockSource, NetworkState, Scene, TelemetrySource, Theme, ThemeKind,
    RGB565_BYTES,
};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
use msutop::linux::{LinuxOptions, LinuxSource};

#[cfg(feature = "sdl")]
mod preview;

#[derive(Clone)]
pub struct Options {
    pub live: bool,
    pub interface: Option<String>,
    pub disk: Option<String>,
    pub mount: PathBuf,
    pub process_hz: u32,
    pub profile: Option<PathBuf>,
    pub smoothing: bool,
    pub sensor_helper: Option<PathBuf>,
    pub sensor_sudo: bool,
    pub scene: Scene,
    pub mock_threads: Option<usize>,
    pub network_state: Option<NetworkState>,
    pub theme: ThemeKind,
    pub time: f64,
    pub scale: usize,
    pub fps: u32,
    pub antialias: bool,
    pub quantize: bool,
    pub export: Option<PathBuf>,
    pub headless: bool,
    pub raw_output: Option<String>,
    pub rawfb_port: Option<String>,
    pub rawfb_ack: AckMode,
    pub rawfb_ready: bool,
    pub msu_port: Option<String>,
    pub msu_ready: bool,
    pub baud: u32,
    pub frames: Option<u64>,
    pub seconds: Option<f64>,
    pub benchmark: Option<u64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            live: false,
            interface: None,
            disk: None,
            mount: "/".into(),
            process_hz: 1,
            profile: None,
            smoothing: true,
            sensor_helper: None,
            sensor_sudo: false,
            scene: Scene::Balanced,
            mock_threads: None,
            network_state: None,
            theme: ThemeKind::Default,
            time: 70.0,
            scale: 6,
            fps: DEFAULT_FPS,
            antialias: true,
            quantize: true,
            export: None,
            headless: false,
            raw_output: None,
            rawfb_port: None,
            rawfb_ack: AckMode::Commands,
            rawfb_ready: false,
            msu_port: None,
            msu_ready: false,
            baud: 115200,
            frames: None,
            seconds: None,
            benchmark: None,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("msutop: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(options) = parse()? else {
        return Ok(());
    };
    if let Some(directory) = &options.export {
        if options.live {
            return Err("--export uses reproducible mock scenes; capture live data with --headless --rgb565".into());
        }
        export_all(directory, &options)?;
        return Ok(());
    }
    if let Some(frames) = options.benchmark {
        if options.live {
            return Err("live counters require elapsed time; use --live --headless --seconds N --profile PATH".into());
        }
        benchmark(&options, frames);
        return Ok(());
    }
    if options.headless
        || options.raw_output.is_some()
        || options.rawfb_port.is_some()
        || options.msu_port.is_some()
    {
        return headless(&options);
    }
    #[cfg(feature = "sdl")]
    {
        preview::run(&options).map_err(Into::into)
    }
    #[cfg(not(feature = "sdl"))]
    {
        Err("Preview requires --features sdl. Use --export output or --headless --rgb565 frame.rgb565 on a host without a GUI.".into())
    }
}

fn parse() -> Result<Option<Options>, Box<dyn std::error::Error>> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("msutop — one fixed 160x80 frame, 6x8 font\n\
                    --scene balanced|idle|cpu|io|memory|offline\n\
                    --mock-threads N       preview 1..512 logical CPU threads (default 8)\n\
                    --live                 collect actual Linux procfs/sysfs values\n\
                    --interface NAME       live network interface (default route by default)\n\
                    --disk NAME            live physical block device (mount backing disks by default)\n\
                    --mount PATH           filesystem capacity to show (default /)\n\
                    --process-hz N         all-process scan frequency, 1..60 (default 1; visible processes at frame rate)\n\
                    --profile PATH         save collection/render or SDL presentation timing as JSON\n\
                    --no-ema               show raw live measurements without display smoothing\n\
                    --sensor-helper PATH   read-only power/SMART helper, outside the frame loop\n\
                    --sensor-sudo          run that helper with non-interactive sudo\n\
                    --network down|nocarrier|active|transfer   override the mock link state\n\
                    --theme default|vivid   btop default or requested bright CPU stops\n\
                    --time N               deterministic mock start time (default 70)\n\
                    --scale N              integer nearest-neighbor preview scale, 1..16\n\
                    --fps N                charts + numbers + render rate, 1..60 (default 30)\n\
                    --no-aa                disable graph, column and ring antialiasing\n\
                    --rgb888               disable the default RGB565 preview quantization\n\
                    --export DIRECTORY     export all scene/theme BMPs and raw 160x80 RGB565\n\
                    --headless             no SDL, display server, or window\n\
                    --rgb565 PATH|-        write raw big-endian frames to file or stdout\n\
                    --rawfb PORT           send legacy image commands (requires serial feature)\n\
                    --rawfb-ack commands|segments  repository or original firmware replies\n\
                    --rawfb-ready          resume a confirmed N-mode session without handshake\n\
                    --msu PORT             RGB565 R / RAW FULL + pipeline, same as RAW_DIRECT_POC.py\n\
                    --msu-ready            resume a confirmed R-mode session without handshake\n\
                    --baud N               serial line coding (default 115200)\n\
                    --frames N             stop after N frames (headless defaults to one)\n\
                    --seconds N            bounded run (preview or headless)\n\
                    --benchmark N          render and RGB565-encode N unpaced frames\n\n\
                    SDL keys: Space pause; 1..6 mock scene; N network state; T palette; A AA; Q RGB565; +/- zoom; Esc exit.\n\
                    All monitoring regions remain visible. There is no page switching.");
                return Ok(None);
            }
            "--scene" => {
                let v = value(&mut args, &arg)?;
                options.scene = Scene::parse(&v).ok_or("unknown mock scene")?;
            }
            "--live" => options.live = true,
            "--mock-threads" => options.mock_threads = Some(value(&mut args, &arg)?.parse()?),
            "--interface" => options.interface = Some(value(&mut args, &arg)?),
            "--disk" => options.disk = Some(value(&mut args, &arg)?),
            "--mount" => options.mount = value(&mut args, &arg)?.into(),
            "--process-hz" => options.process_hz = value(&mut args, &arg)?.parse()?,
            "--profile" => options.profile = Some(value(&mut args, &arg)?.into()),
            "--no-ema" => options.smoothing = false,
            "--sensor-helper" => options.sensor_helper = Some(value(&mut args, &arg)?.into()),
            "--sensor-sudo" => options.sensor_sudo = true,
            "--network" => {
                let v = value(&mut args, &arg)?;
                options.network_state =
                    Some(NetworkState::parse(&v).ok_or("unknown mock network state")?);
            }
            "--theme" => {
                let v = value(&mut args, &arg)?;
                options.theme = ThemeKind::parse(&v).ok_or("unknown theme")?;
            }
            "--time" => options.time = value(&mut args, &arg)?.parse()?,
            "--scale" => options.scale = value(&mut args, &arg)?.parse()?,
            "--fps" => options.fps = value(&mut args, &arg)?.parse()?,
            "--export" => options.export = Some(value(&mut args, &arg)?.into()),
            "--frames" => options.frames = Some(value(&mut args, &arg)?.parse()?),
            "--seconds" => options.seconds = Some(value(&mut args, &arg)?.parse()?),
            "--benchmark" => options.benchmark = Some(value(&mut args, &arg)?.parse()?),
            "--rgb565" => options.raw_output = Some(value(&mut args, &arg)?),
            "--rawfb" => options.rawfb_port = Some(value(&mut args, &arg)?),
            "--msu" => options.msu_port = Some(value(&mut args, &arg)?),
            "--msu-ready" => options.msu_ready = true,
            "--rawfb-ready" => options.rawfb_ready = true,
            "--rawfb-ack" => {
                options.rawfb_ack =
                    AckMode::parse(&value(&mut args, &arg)?).ok_or("unknown RAWFB ACK mode")?
            }
            "--baud" => options.baud = value(&mut args, &arg)?.parse()?,
            "--headless" => options.headless = true,
            "--no-aa" => options.antialias = false,
            "--rgb888" => options.quantize = false,
            _ => return Err(format!("unknown argument {arg}; see --help").into()),
        }
    }
    if !(1..=16).contains(&options.scale) {
        return Err("--scale must be 1..16".into());
    }
    if !(1..=60).contains(&options.fps) {
        return Err("--fps must be 1..60".into());
    }
    if !(1..=60).contains(&options.process_hz) {
        return Err("--process-hz must be 1..60".into());
    }
    if options.live && options.network_state.is_some() {
        return Err("--network overrides only mock data".into());
    }
    if options.live && options.mock_threads.is_some() {
        return Err("--mock-threads overrides only mock data".into());
    }
    if options
        .mock_threads
        .is_some_and(|count| !(1..=512).contains(&count))
    {
        return Err("--mock-threads must be 1..512".into());
    }
    if options.sensor_sudo && options.sensor_helper.is_none() {
        return Err("--sensor-sudo requires --sensor-helper".into());
    }
    if options.sensor_helper.is_some() && !options.live {
        return Err("--sensor-helper requires --live".into());
    }
    if !options.time.is_finite() || options.time < 0.0 {
        return Err("--time must be finite and nonnegative".into());
    }
    if options.seconds.is_some_and(|s| !s.is_finite() || s <= 0.0) {
        return Err("--seconds must be positive and finite".into());
    }
    if options.frames == Some(0) || options.benchmark == Some(0) {
        return Err("frame count must be positive".into());
    }
    if options.baud == 0 {
        return Err("--baud must be positive".into());
    }
    if options.rawfb_ready && options.rawfb_port.is_none() {
        return Err("--rawfb-ready requires --rawfb PORT".into());
    }
    if options.msu_ready && options.msu_port.is_none() {
        return Err("--msu-ready requires --msu PORT".into());
    }
    if options.msu_port.is_some() && options.rawfb_port.is_some() {
        return Err("choose one device transport: --msu or --rawfb".into());
    }
    if (options.rawfb_port.is_some() || options.msu_port.is_some())
        && (options.export.is_some() || options.benchmark.is_some())
    {
        return Err("--rawfb/--msu cannot be combined with --export or --benchmark".into());
    }
    Ok(Some(options))
}

fn value(
    args: &mut impl Iterator<Item = String>,
    name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    args.next()
        .ok_or_else(|| format!("{name} requires a value").into())
}

pub enum Source {
    Mock(MockSource),
    #[cfg(target_os = "linux")]
    Linux(Box<LinuxSource>),
}

impl Source {
    pub fn complete_display(&self, _shown: &mut msutop::Snapshot, _ids: [Option<u64>; 2]) {
        #[cfg(target_os = "linux")]
        if let Self::Linux(source) = self {
            source.complete_display(_shown, _ids);
        }
    }
    pub fn ranking<'a>(&'a self, snapshot: &'a msutop::Snapshot) -> &'a [msutop::metrics::Process] {
        #[cfg(target_os = "linux")]
        if let Self::Linux(source) = self {
            return source.ranking_processes();
        }
        &snapshot.processes
    }
    pub fn watch_processes(&mut self, _ids: [Option<u64>; 2]) {
        #[cfg(target_os = "linux")]
        if let Self::Linux(source) = self {
            source.watch_processes(_ids);
        }
    }

    fn own_cpu_seconds(&self) -> f64 {
        msutop::profiling::process_cpu_seconds()
    }

    fn collection_parts(&self) -> [msutop::profiling::Cost; 7] {
        #[cfg(target_os = "linux")]
        if let Self::Linux(source) = self {
            return source.cost().parts;
        }
        [msutop::profiling::Cost::default(); 7]
    }

    fn collection_cost(&self) -> (f64, f64, f64, bool, bool) {
        #[cfg(target_os = "linux")]
        if let Self::Linux(source) = self {
            let cost = source.cost();
            return (
                cost.fast_us,
                cost.processes_us,
                cost.sensors_us,
                cost.process_poll,
                cost.sensor_poll,
            );
        }
        (0.0, 0.0, 0.0, false, false)
    }

    fn report_metadata(&self) -> String {
        #[cfg(target_os = "linux")]
        if let Self::Linux(source) = self {
            let s = source.snapshot();
            return format!("\"source\":\"linux\",\"interface\":{},\"disk\":{},\"cores\":{},\"process_count\":{},\"process_hz\":{},\"read_errors\":{},\"cpu_temperature_available\":{},\"cpu_power_available\":{},\"disk_temperature_available\":{},\"disk_capacity_bytes\":{},\"memory_bytes\":{},\"sensor_helper\":{}",
                json_string(&s.network.interface), json_string(&s.disk.name), s.cpu.cores.len(),
                s.process_count, source.process_hz(), source.read_errors, s.cpu.temperature.is_some(),
                s.cpu.power_watts.is_some(), s.disk.temperature.is_some(), s.disk.total, s.memory.total,
                source.helper_status().map_or_else(|| "null".into(), |value| value.to_string()));
        }
        "\"source\":\"mock\"".into()
    }
}

impl TelemetrySource for Source {
    fn sample(&mut self, seconds: f64) -> msutop::Snapshot {
        match self {
            Self::Mock(source) => source.sample(seconds),
            #[cfg(target_os = "linux")]
            Self::Linux(source) => source.sample(seconds),
        }
    }
}

pub fn prepared(
    options: &Options,
) -> Result<(Source, Dashboard, Frame), Box<dyn std::error::Error>> {
    let mut source = if options.live {
        #[cfg(target_os = "linux")]
        {
            let mut source = LinuxSource::new(LinuxOptions {
                mount: options.mount.clone(),
                interface: options.interface.clone(),
                disk: options.disk.clone(),
                process_hz: options.process_hz,
            })?;
            source.set_profiling(options.profile.is_some());
            if let Some(helper) = &options.sensor_helper {
                source.start_sensor_helper(helper, options.sensor_sudo)?;
            }
            Source::Linux(Box::new(source))
        }
        #[cfg(not(target_os = "linux"))]
        {
            return Err(
                "--live requires Linux; mock SDL preview remains available on this host".into(),
            );
        }
    } else {
        let mut source = MockSource::new(options.scene);
        source.network_state = options.network_state;
        if let Some(count) = options.mock_threads {
            source.threads = count;
        }
        Source::Mock(source)
    };
    let mut dashboard = Dashboard::with_sample_rate(Theme::new(options.theme), options.fps);
    dashboard.antialias = options.antialias;
    if !options.live {
        let last_tick = (options.time * options.fps as f64 + 1e-7).floor() as u64;
        let first_tick = last_tick.saturating_sub(HISTORY_SAMPLES as u64 - 1);
        for tick in first_tick..=last_tick {
            let t = tick as f64 / options.fps as f64;
            dashboard.observe(&source.sample(t));
        }
    }
    let mut snapshot = source.sample(if options.live { 0.0 } else { options.time });
    dashboard.observe_ranked(&snapshot, source.ranking(&snapshot));
    source.watch_processes(dashboard.selected_ids());
    source.complete_display(&mut snapshot, dashboard.selected_ids());
    let mut frame = Frame::default();
    dashboard.render(&snapshot, &mut frame);
    if options.quantize {
        frame.quantize_565();
    }
    Ok((source, dashboard, frame))
}

fn export_all(directory: &Path, options: &Options) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(directory)?;
    for theme in [ThemeKind::Default, ThemeKind::Vivid] {
        for scene in Scene::ALL {
            let options = Options {
                theme,
                scene,
                ..options.clone()
            };
            let (_, _, frame) = prepared(&options)?;
            let stem = format!("{}-{}", scene.name(), theme.name());
            frame.write_bmp(
                &mut File::create(directory.join(format!("{stem}-160x80.bmp")))?,
                1,
            )?;
            frame.write_bmp(
                &mut File::create(directory.join(format!("{stem}-{}x.bmp", options.scale)))?,
                options.scale,
            )?;
            let mut raw = vec![0; RGB565_BYTES];
            frame.rgb565_be(&mut raw);
            fs::write(directory.join(format!("{stem}.rgb565")), raw)?;
        }
    }
    eprintln!(
        "Exported 12 fixed-screen mock scenes in {}",
        directory.display()
    );
    Ok(())
}

fn headless(options: &Options) -> Result<(), Box<dyn std::error::Error>> {
    let output: Box<dyn Write> = match options.raw_output.as_deref() {
        Some("-") => Box::new(io::stdout()),
        Some(path) => Box::new(File::create(path)?),
        None => Box::new(io::sink()),
    };
    let mut sink = Rgb565Writer::new(output);
    #[cfg(feature = "serial")]
    let mut rawfb_device = options
        .rawfb_port
        .as_ref()
        .map(|port| msutop::rawfb::open(port, options.baud, options.rawfb_ack, options.rawfb_ready))
        .transpose()?;
    #[cfg(feature = "serial")]
    let mut msu_device = options
        .msu_port
        .as_ref()
        .map(|port| msutop::rgb565::open(port, options.baud, options.msu_ready))
        .transpose()?;
    #[cfg(not(feature = "serial"))]
    if options.rawfb_port.is_some() || options.msu_port.is_some() {
        return Err("--rawfb/--msu requires a build with --features serial".into());
    }
    #[cfg(feature = "serial")]
    if rawfb_device.is_some() {
        eprintln!("RAWFB {} on {} at {} 8N1, ACK mode {}; N-mode repository firmware uses a 16-color palette",
            if options.rawfb_ready { "resuming confirmed N mode" } else { "handshake confirmed" },
            options.rawfb_port.as_deref().unwrap_or_default(), options.baud, options.rawfb_ack.name());
    }
    #[cfg(feature = "serial")]
    if msu_device.is_some() {
        eprintln!(
            "RGB565 R {} on {} at {} 8N1; RAW FULL + pipeline, 11 checked ACKs/frame",
            if options.msu_ready {
                "resuming confirmed R mode"
            } else {
                "handshake confirmed"
            },
            options.msu_port.as_deref().unwrap_or_default(),
            options.baud
        );
    }
    let (mut source, mut dashboard, mut frame) = prepared(options)?;
    let start = Instant::now();
    let period = Duration::from_secs_f64(1.0 / options.fps as f64);
    let limit = options.frames.or_else(|| {
        if options.seconds.is_some() {
            None
        } else {
            Some(1)
        }
    });
    let mut count = 0;
    let cpu_before = source.own_cpu_seconds();
    let mut timing = RunTiming::default();
    let mut parts = [msutop::profiling::Cost::default(); 15];
    let mut smoother = msutop::smoothing::Smoother::default();
    let mut trace = if let Some(path) = &options.profile {
        Some(io::BufWriter::new(File::create(
            path.with_extension("csv"),
        )?))
    } else {
        None
    };
    if let Some(trace) = &mut trace {
        writeln!(trace, "seconds,collect_us,fast_us,process_us,sensor_us,render_us,output_us,work_us,late_us,cpu_percent,cpu_mhz,cpu_temp_c,cpu_watts,memory_used,memory_cached,memory_free,swap_used,disk_read_bps,disk_write_bps,queue_depth,network_rx_bps,network_tx_bps,network_state,process_count,shown_cpu,shown_disk_read,shown_disk_write,shown_network_rx,shown_network_tx,shown_process0,shown_process1,disk_temperature")?;
    }
    loop {
        let frame_start = Instant::now();
        let time_offset = if options.live { 0.0 } else { options.time };
        let t = time_offset + start.elapsed().as_secs_f64();
        let phase = Instant::now();
        let snapshot = source.sample(t);
        let collect = phase.elapsed().as_secs_f64() * 1e6;
        let ema_meter = msutop::profiling::Meter::new(options.profile.is_some());
        let mut shown = if options.live && options.smoothing {
            smoother.apply(&snapshot, dashboard.selected_ids())
        } else {
            snapshot.clone()
        };
        let ema_cost = ema_meter.elapsed();
        let phase = Instant::now();
        let cpu_meter = msutop::profiling::Meter::new(options.profile.is_some());
        dashboard.observe_ranked(&shown, source.ranking(&snapshot));
        source.watch_processes(dashboard.selected_ids());
        source.complete_display(&mut shown, dashboard.selected_ids());
        let observe_cost = cpu_meter.elapsed();
        let render_parts = if options.profile.is_some() {
            dashboard.render_profiled(&shown, &mut frame)
        } else {
            dashboard.render(&shown, &mut frame);
            [msutop::profiling::Cost::default(); 5]
        };
        let render = phase.elapsed().as_secs_f64() * 1e6;
        let phase = Instant::now();
        let cpu_meter = msutop::profiling::Meter::new(options.profile.is_some());
        sink.present(&frame)?;
        #[cfg(feature = "serial")]
        if let Some(device) = &mut rawfb_device {
            device.present(&frame)?;
        }
        #[cfg(feature = "serial")]
        if let Some(device) = &mut msu_device {
            device.present(&frame)?;
        }
        let output_cost = cpu_meter.elapsed();
        let output = phase.elapsed().as_secs_f64() * 1e6;
        let work = frame_start.elapsed().as_secs_f64() * 1e6;
        let expected = start + period.mul_f64(count as f64);
        let late = frame_start
            .saturating_duration_since(expected)
            .as_secs_f64()
            * 1e6;
        let (fast, processes, sensors, process_poll, sensor_poll) = source.collection_cost();
        if options.profile.is_some() {
            parts[14].wall_us += ema_cost.wall_us;
            parts[14].cpu_us += ema_cost.cpu_us;
            for (sum, value) in parts.iter_mut().zip(
                source
                    .collection_parts()
                    .into_iter()
                    .chain([observe_cost])
                    .chain(render_parts)
                    .chain([output_cost]),
            ) {
                sum.wall_us += value.wall_us;
                sum.cpu_us += value.cpu_us;
            }
            timing.collect.push(collect);
            timing.render.push(render);
            timing.output.push(output);
            timing.work.push(work);
            timing.late.push(late);
            timing.fast.push(fast);
            if process_poll {
                timing.processes.push(processes);
            }
            if sensor_poll {
                timing.sensors.push(sensors);
            }
            if work > period.as_secs_f64() * 1e6 {
                timing.over_budget += 1;
            }
        }
        if let Some(trace) = &mut trace {
            let process_values = dashboard.selected_ids().map(|id| {
                id.and_then(|id| shown.processes.iter().find(|p| p.id == id))
                    .map_or(0.0, |p| p.cpu)
            });
            writeln!(trace, "{t:.6},{collect:.3},{fast:.3},{processes:.3},{sensors:.3},{render:.3},{output:.3},{work:.3},{late:.3},{:.3},{:.3},{},{},{},{},{},{},{:.3},{:.3},{},{:.3},{:.3},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{}",
                snapshot.cpu.usage, snapshot.cpu.frequency_mhz,
                snapshot.cpu.temperature.map_or_else(String::new, |v| v.to_string()), snapshot.cpu.power_watts.map_or_else(String::new, |v| v.to_string()),
                snapshot.memory.used, snapshot.memory.cached, snapshot.memory.free, snapshot.memory.swap_used,
                snapshot.disk.read_bps, snapshot.disk.write_bps, snapshot.disk.queue_depth.map_or_else(String::new, |v| v.to_string()),
                snapshot.network.download_bps, snapshot.network.upload_bps, snapshot.network.state.name(), snapshot.process_count,
                shown.cpu.usage, shown.disk.read_bps, shown.disk.write_bps, shown.network.download_bps, shown.network.upload_bps,
                process_values[0], process_values[1], snapshot.disk.temperature.map_or_else(String::new, |v| v.to_string()))?;
        }
        count += 1;
        if limit.is_some_and(|n| count >= n)
            || options
                .seconds
                .is_some_and(|s| start.elapsed().as_secs_f64() >= s)
        {
            break;
        }
        // If a device is slower than the target, sample the current time without a burst of catch-up frames.
        let deadline = if options.rawfb_port.is_some() || options.msu_port.is_some() {
            frame_start + period
        } else {
            start + period.mul_f64(count as f64)
        };
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    if let Some(trace) = &mut trace {
        trace.flush()?;
    }
    #[cfg(feature = "serial")]
    if let Some(device) = &rawfb_device {
        let stats = device.stats();
        eprintln!(
            "RAWFB: {} completed frames, {} transmitted bytes, {} reply bytes, {:.2} fps",
            stats.frames,
            stats.wire_bytes,
            stats.reply_bytes,
            stats.frames as f64 / start.elapsed().as_secs_f64()
        );
    }
    #[cfg(feature = "serial")]
    if let Some(device) = &msu_device {
        let stats = device.stats();
        eprintln!(
            "RGB565 R: {} completed frames, {} transmitted bytes, {} reply bytes, {:.2} fps",
            stats.frames,
            stats.wire_bytes,
            stats.reply_bytes,
            stats.frames as f64 / start.elapsed().as_secs_f64()
        );
    }
    if let Some(path) = &options.profile {
        let elapsed = start.elapsed().as_secs_f64();
        let cpu_percent = (source.own_cpu_seconds() - cpu_before).max(0.0) / elapsed * 100.0;
        #[cfg(feature = "serial")]
        let transport = if let Some(device) = &rawfb_device {
            let stats = device.stats();
            format!("{{\"kind\":\"rawfb\",\"port\":{},\"ack_mode\":\"{}\",\"frames\":{},\"wire_bytes\":{},\"reply_bytes\":{}}}",
                json_string(options.rawfb_port.as_deref().unwrap_or_default()), options.rawfb_ack.name(),
                stats.frames, stats.wire_bytes, stats.reply_bytes)
        } else if let Some(device) = &msu_device {
            let stats = device.stats();
            format!("{{\"kind\":\"rgb565-v1\",\"port\":{},\"encoding\":\"RAW\",\"coverage\":\"FULL\",\"pipeline\":true,\"acks_per_frame\":11,\"frames\":{},\"wire_bytes\":{},\"reply_bytes\":{}}}",
                json_string(options.msu_port.as_deref().unwrap_or_default()), stats.frames, stats.wire_bytes, stats.reply_bytes)
        } else {
            "null".to_owned()
        };
        #[cfg(not(feature = "serial"))]
        let transport = "null";
        let report = format!("{{\n{},\n\"transport\":{transport},\n\"frames\":{count},\"target_fps\":{},\"elapsed_seconds\":{elapsed:.6},\"effective_fps\":{:.3},\"cpu_percent_one_core\":{cpu_percent:.3},\"over_budget_frames\":{},\n\"collect_us\":{},\n\"fast_us\":{},\n\"process_poll_us\":{},\n\"sensor_poll_us\":{},\n\"render_us\":{},\n\"output_us\":{},\n\"work_us\":{},\n\"late_us\":{},\n\"cpu_breakdown\":{}\n}}\n",
            source.report_metadata(), options.fps, count as f64 / elapsed, timing.over_budget,
            distribution(&timing.collect), distribution(&timing.fast), distribution(&timing.processes), distribution(&timing.sensors),
            distribution(&timing.render), distribution(&timing.output), distribution(&timing.work), distribution(&timing.late), cpu_breakdown(&parts, count, elapsed));
        fs::write(path, report)?;
        eprintln!(
            "Profile written to {} (raw timings: {})",
            path.display(),
            path.with_extension("csv").display()
        );
    }
    eprintln!(
        "Rendered {count} raw 160x80 frames ({} bytes/frame), {:.2}s",
        RGB565_BYTES,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn cpu_breakdown(parts: &[msutop::profiling::Cost; 15], count: u64, seconds: f64) -> String {
    let names = [
        "cpu_counters",
        "memory_counters",
        "disk_counters",
        "network_counters",
        "process_scan",
        "visible_processes",
        "sensors",
        "history_and_selection",
        "cpu_graph",
        "memory_rings",
        "disk_graphs",
        "network_graphs",
        "process_graphs",
        "rgb565_output",
        "smoothing",
    ];
    let values: Vec<_> = names.into_iter().zip(parts).map(|(name, cost)| format!(
        "\"{name}\":{{\"cpu_ms\":{:.3},\"wall_ms\":{:.3},\"cpu_percent_one_core\":{:.4},\"mean_cpu_us_per_frame\":{:.3}}}",
        cost.cpu_us / 1000.0, cost.wall_us / 1000.0, cost.cpu_us / 1e6 / seconds * 100.0, cost.cpu_us / count as f64)).collect();
    format!("{{{}}}", values.join(","))
}

fn benchmark(options: &Options, count: u64) {
    let (mut source, mut dashboard, mut frame) = prepared(options).expect("mock source");
    let mut scratch = vec![0; RGB565_BYTES];
    let start = Instant::now();
    for index in 0..count {
        let snapshot = source.sample(options.time + index as f64 / options.fps as f64);
        dashboard.observe(&snapshot);
        dashboard.render(&snapshot, &mut frame);
        frame.rgb565_be(&mut scratch);
        std::hint::black_box(&scratch);
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!("{count} frames, {:.3}s, {:.1} us/frame, {:.0} frames/s (mock + render + RGB565; no device transport)",
        elapsed, elapsed * 1e6 / count as f64, count as f64 / elapsed);
}

#[derive(Default)]
struct RunTiming {
    collect: Vec<f64>,
    fast: Vec<f64>,
    processes: Vec<f64>,
    sensors: Vec<f64>,
    render: Vec<f64>,
    output: Vec<f64>,
    work: Vec<f64>,
    late: Vec<f64>,
    over_budget: u64,
}

fn distribution(values: &[f64]) -> String {
    if values.is_empty() {
        return "{\"samples\":0,\"mean\":0,\"p50\":0,\"p95\":0,\"p99\":0,\"max\":0}".into();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| sorted[((sorted.len() - 1) as f64 * p).ceil() as usize];
    format!(
        "{{\"samples\":{},\"mean\":{:.3},\"p50\":{:.3},\"p95\":{:.3},\"p99\":{:.3},\"max\":{:.3}}}",
        values.len(),
        values.iter().sum::<f64>() / values.len() as f64,
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
        sorted[sorted.len() - 1]
    )
}

#[cfg(any(target_os = "linux", feature = "serial"))]
fn json_string(text: &str) -> String {
    let mut result = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            ch if ch.is_control() => result.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}
