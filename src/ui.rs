use crate::{
    icons::Icon,
    metrics::{NetworkState, Snapshot, GIB, MIB},
    selection::LazyTop,
    theme::Gradient,
    Frame, Rgb, Theme,
};
use std::collections::{BTreeMap, VecDeque};

pub const DEFAULT_FPS: u32 = 30;
pub const HISTORY_SAMPLES: usize = 128;
const SELECTION_SECONDS: f64 = 1.0;
const CPU_BOTTOM: i32 = 22;
const CPU_GRAPH_X: i32 = 2;
const CPU_GRAPH_MIN_WIDTH: i32 = 38;
const CPU_LEFT_VALUE_RIGHT: i32 = 141;
const CPU_RIGHT_VALUE_RIGHT: i32 = 157;
const CPU_BAR_RIGHT: i32 = CPU_LEFT_VALUE_RIGHT - 18;
const GRID_STEP: i32 = 5;
const MEMORY_RIGHT: i32 = 38;
const BLOCK_RIGHT: i32 = 96;
const MEMORY_TOP: i32 = CPU_BOTTOM + 2;
const RING_X: i32 = MEMORY_RIGHT - 16;
const SWAP_TOP: i32 = 40;
const NETWORK_GRAPH_WIDTH: i32 = 36;
const NETWORK_ARROW_X: i32 = 136;
const NETWORK_ICON_X: i32 = BLOCK_RIGHT + 3;
const DISK_COLUMN_X: i32 = BLOCK_RIGHT - 4;
const DISK_COLUMN_WIDTH: i32 = 3;
const DISK_VALUE_RIGHT: i32 = DISK_COLUMN_X - 2;
const DISK_GRAPH_WIDTH: i32 = DISK_VALUE_RIGHT - 15 - (MEMORY_RIGHT + 10) - 2;
const QUEUE_DEPTH_FULL: f32 = 8.0;
const ACTIVITY_PERIOD: f64 = 0.2;
const ACTIVITY_ON_SECONDS: f64 = 0.1;
const ACTIVITY_HOLD_SECONDS: f64 = 0.25;
const SCALE_RISE_SECONDS: f64 = 0.5;
const SCALE_FALL_SECONDS: f64 = 3.0;
const RATE_SCALE_MIN: f32 = 0.015625;
const PROCESS_SCALE_MIN: f32 = 8.0;
const PROCESS_ROW_TOP: i32 = 57;
const PROCESS_ROW_PITCH: i32 = 11;
const PROCESS_NAME_CELLS: usize = 13;
const PROCESS_CPU_RIGHT: i32 = 110;
const PROCESS_GRAPH_X: i32 = 113;
const PROCESS_GRAPH_WIDTH: i32 = 18;
const PROCESS_MEMORY_RIGHT: i32 = 157;

#[derive(Clone, Copy, Debug)]
struct CpuLayout {
    count: usize,
    columns: usize,
    rows: usize,
    bar_width: i32,
    graph_width: i32,
}

impl CpuLayout {
    fn new(count: usize) -> Self {
        if count == 0 {
            return Self {
                count,
                columns: 0,
                rows: 0,
                bar_width: 3,
                graph_width: CPU_BAR_RIGHT - CPU_GRAPH_X,
            };
        }
        // Leave room for uptime, a useful graph, and the separator's padding.
        let available = CPU_BAR_RIGHT - CPU_GRAPH_X - CPU_GRAPH_MIN_WIDTH - 4;
        let max_columns = ((available + 1) / 2) as usize;
        let rows = count.div_ceil(max_columns);
        let columns = count.div_ceil(rows);
        let bar_width = [3, 2, 1]
            .into_iter()
            .find(|width| columns as i32 * (width + 1) - 1 <= available)
            .unwrap_or(1);
        let span = columns as i32 * (bar_width + 1) - 1;
        Self {
            count,
            columns,
            rows,
            bar_width,
            graph_width: CPU_BAR_RIGHT - span - CPU_GRAPH_X - 4,
        }
    }

    fn separator_x(self) -> Option<i32> {
        (self.count > 0).then_some(CPU_GRAPH_X + self.graph_width + 2)
    }

    fn bar(self, index: usize) -> CpuBar {
        let row = index / self.columns;
        let column = index % self.columns;
        let row_count = (self.count - row * self.columns).min(self.columns);
        let span = row_count as i32 * (self.bar_width + 1) - 1;
        let height = CPU_BOTTOM - 3;
        let row_gap = i32::from(self.rows <= ((height + 1) / 2) as usize);
        let available = height - (self.rows as i32 - 1) * row_gap;
        let top = 2 + row as i32 * available / self.rows as i32 + row as i32 * row_gap;
        let bottom = 2 + (row as i32 + 1) * available / self.rows as i32 + row as i32 * row_gap;
        let indicator_y = (bottom - top >= 3).then_some(top);
        let y = top + if indicator_y.is_some() { 2 } else { 0 };
        CpuBar {
            x: CPU_BAR_RIGHT - span + column as i32 * (self.bar_width + 1),
            y,
            width: self.bar_width,
            height: bottom - y,
            indicator_y,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CpuBar {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    indicator_y: Option<i32>,
}

#[derive(Clone, Copy)]
struct CpuSample {
    seconds: f64,
    values: [f32; 3],
}

#[derive(Default)]
struct CpuTrace(VecDeque<CpuSample>);

impl CpuTrace {
    fn push(&mut self, snapshot: &Snapshot) {
        if self.0.len() == HISTORY_SAMPLES {
            self.0.pop_front();
        }
        self.0.push_back(CpuSample {
            seconds: snapshot.seconds,
            values: [snapshot.cpu.usage, snapshot.cpu.system, snapshot.cpu.iowait],
        });
    }
}

#[derive(Default)]
struct Trace(VecDeque<f32>);

impl Trace {
    fn push(&mut self, value: f32) {
        if self.0.len() == HISTORY_SAMPLES {
            self.0.pop_front();
        }
        self.0.push_back(if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        });
    }

    fn peak(&self) -> f32 {
        self.0.iter().copied().fold(0.0, f32::max)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PlotPoint {
    ratio: f32,
    system: f32,
    wait: f32,
}

impl PlotPoint {
    fn new(values: [f64; 3], scale: f64) -> Self {
        let ratio = |value: f64| (value / scale.max(0.00001)).clamp(0.0, 1.0) as f32;
        Self {
            ratio: ratio(values[0]),
            system: ratio(values[1]),
            wait: ratio(values[2]),
        }
    }

    fn lerp(self, other: Self, fraction: f32) -> Self {
        Self {
            ratio: self.ratio + (other.ratio - self.ratio) * fraction,
            system: self.system + (other.system - self.system) * fraction,
            wait: self.wait + (other.wait - self.wait) * fraction,
        }
    }
}

/// Completed time bins are immutable curve knots, including their Y scale.
/// Their X coordinates move continuously with the clock, including across bin boundaries.
struct PixelHistory {
    width: usize,
    period: f64,
    columns: VecDeque<PlotPoint>,
    live: PlotPoint,
    bucket: u64,
    area: [f64; 3],
    duration: f64,
    previous: Option<(f64, [f64; 3], f64)>,
}

impl PixelHistory {
    fn new(width: i32, fps: u32) -> Self {
        Self {
            width: width as usize,
            period: (HISTORY_SAMPLES - 1) as f64 / fps as f64 / (width - 1) as f64,
            columns: VecDeque::with_capacity(width as usize + 1),
            live: PlotPoint::default(),
            bucket: 0,
            area: [0.0; 3],
            duration: 0.0,
            previous: None,
        }
    }

    fn observe(&mut self, seconds: f64, values: [f32; 3], scale: f32) {
        let values = values.map(|v| {
            if v.is_finite() {
                v.max(0.0) as f64
            } else {
                0.0
            }
        });
        let scale = scale.max(0.00001) as f64;
        let Some((before, old_values, old_scale)) = self.previous else {
            self.bucket = (seconds / self.period).floor() as u64;
            self.live = PlotPoint::new(values, scale);
            self.previous = Some((seconds, values, scale));
            return;
        };
        if seconds <= before {
            return;
        }
        if seconds - before > self.period * (self.width + 1) as f64 {
            self.columns.clear();
            self.area = [0.0; 3];
            self.duration = 0.0;
            self.previous = None;
            self.observe(seconds, values.map(|v| v as f32), scale as f32);
            return;
        }
        let elapsed = seconds - before;
        let mut cursor = before;
        while cursor < seconds {
            let boundary = (self.bucket + 1) as f64 * self.period;
            let end = seconds.min(boundary);
            if end > cursor {
                let left = (cursor - before) / elapsed;
                let right = (end - before) / elapsed;
                for channel in 0..3 {
                    let a = old_values[channel] + (values[channel] - old_values[channel]) * left;
                    let b = old_values[channel] + (values[channel] - old_values[channel]) * right;
                    self.area[channel] += (a + b) * 0.5 * (end - cursor);
                }
                self.duration += end - cursor;
            }
            if end >= boundary {
                let fraction = (boundary - before) / elapsed;
                let scale_at_boundary = old_scale + (scale - old_scale) * fraction;
                let average = if self.duration > 0.0 {
                    self.area.map(|v| v / self.duration)
                } else {
                    values
                };
                if self.columns.len() == self.width + 1 {
                    self.columns.pop_front();
                }
                self.columns
                    .push_back(PlotPoint::new(average, scale_at_boundary));
                self.bucket += 1;
                self.area = [0.0; 3];
                self.duration = 0.0;
            }
            cursor = end;
        }
        let average = if self.duration > 0.0 {
            self.area.map(|v| v / self.duration)
        } else {
            values
        };
        self.live = PlotPoint::new(average, scale);
        self.previous = Some((seconds, values, scale));
    }

    fn at(&self, column: i32) -> PlotPoint {
        if self.columns.is_empty() && column == self.width as i32 - 1 {
            return self.live;
        }
        let index = self.columns.len() as i32 - (self.width as i32 - column);
        if index < 0 {
            PlotPoint::default()
        } else {
            self.columns
                .get(index as usize)
                .copied()
                .unwrap_or_default()
        }
    }

    fn offset_at(&self, seconds: f64) -> f32 {
        let Some((observed, _, _)) = self.previous else {
            return 0.0;
        };
        let seconds = if seconds.is_finite() {
            seconds.max(observed)
        } else {
            observed
        };
        ((seconds - self.bucket as f64 * self.period) / self.period)
            .clamp(0.0, self.width as f64 + 1.0) as f32
    }

    fn sample_at(&self, x: f32, offset: f32) -> PlotPoint {
        let right = (self.width - 1) as f32;
        let position = x + offset;
        if position >= right {
            let fraction = if offset > 0.0 {
                ((position - right) / offset).clamp(0.0, 1.0)
            } else {
                0.0
            };
            self.at(right as i32).lerp(self.live, fraction)
        } else {
            let column = position.floor() as i32;
            self.at(column)
                .lerp(self.at(column + 1), position - column as f32)
        }
    }
}

/// Smooth the axis, keeping the original samples and numeric values untouched.
struct EmaScale {
    value: f32,
    minimum: f32,
    initialized: bool,
}

impl EmaScale {
    fn new(minimum: f32) -> Self {
        Self {
            value: minimum,
            minimum,
            initialized: false,
        }
    }

    fn observe(&mut self, peak: f32, elapsed: f64) {
        let target = (peak.max(0.0) * 1.10).max(self.minimum);
        if !self.initialized {
            self.value = target;
            self.initialized = true;
            return;
        }
        if !elapsed.is_finite() || elapsed <= 0.0 {
            return;
        }
        let tau = if target > self.value {
            SCALE_RISE_SECONDS
        } else {
            SCALE_FALL_SECONDS
        };
        let alpha = -(-elapsed / tau).exp_m1() as f32;
        self.value += (target - self.value) * alpha;
    }
}

struct ProcessTrace {
    trace: Trace,
    scale: EmaScale,
    pixels: PixelHistory,
}

impl ProcessTrace {
    fn new(fps: u32) -> Self {
        Self {
            trace: Trace::default(),
            scale: EmaScale::new(PROCESS_SCALE_MIN),
            pixels: PixelHistory::new(PROCESS_GRAPH_WIDTH, fps),
        }
    }
}

pub struct Dashboard {
    pub theme: Theme,
    pub antialias: bool,
    sample_rate: u32,
    history_tick: Option<u64>,
    history_time: Option<f64>,
    selection_time: Option<f64>,
    cpu: CpuTrace,
    download: Trace,
    upload: Trace,
    disk_read: Trace,
    disk_write: Trace,
    cpu_layout: CpuLayout,
    cpu_pixels: PixelHistory,
    download_pixels: PixelHistory,
    upload_pixels: PixelHistory,
    disk_read_pixels: PixelHistory,
    disk_write_pixels: PixelHistory,
    process_traces: BTreeMap<u64, ProcessTrace>,
    ring_mask: RingMask,
    top: LazyTop<2>,
    download_scale: EmaScale,
    upload_scale: EmaScale,
    disk_scale: EmaScale,
    network_activity_since: Option<f64>,
    network_last_activity: Option<f64>,
}

impl Dashboard {
    pub fn new(theme: Theme) -> Self {
        Self::with_sample_rate(theme, DEFAULT_FPS)
    }

    pub fn with_sample_rate(theme: Theme, fps: u32) -> Self {
        Self {
            theme,
            antialias: true,
            sample_rate: fps.clamp(1, 60),
            history_tick: None,
            history_time: None,
            selection_time: None,
            cpu: CpuTrace::default(),
            download: Trace::default(),
            upload: Trace::default(),
            disk_read: Trace::default(),
            disk_write: Trace::default(),
            cpu_layout: CpuLayout::new(0),
            cpu_pixels: PixelHistory::new(CpuLayout::new(0).graph_width, fps.clamp(1, 60)),
            download_pixels: PixelHistory::new(NETWORK_GRAPH_WIDTH, fps.clamp(1, 60)),
            upload_pixels: PixelHistory::new(NETWORK_GRAPH_WIDTH, fps.clamp(1, 60)),
            disk_read_pixels: PixelHistory::new(DISK_GRAPH_WIDTH, fps.clamp(1, 60)),
            disk_write_pixels: PixelHistory::new(DISK_GRAPH_WIDTH, fps.clamp(1, 60)),
            process_traces: BTreeMap::new(),
            ring_mask: RingMask::new(13, 5.0),
            top: LazyTop::default(),
            download_scale: EmaScale::new(RATE_SCALE_MIN),
            upload_scale: EmaScale::new(RATE_SCALE_MIN),
            disk_scale: EmaScale::new(RATE_SCALE_MIN),
            network_activity_since: None,
            network_last_activity: None,
        }
    }

    /// Every new 30 Hz tick enters the charts; numeric fields use the current snapshot.
    /// Process-name confirmation stays at 1 Hz, independent of chart/render FPS.
    pub fn observe(&mut self, snapshot: &Snapshot) {
        self.observe_ranked(snapshot, &snapshot.processes);
    }

    pub fn observe_ranked(&mut self, snapshot: &Snapshot, ranking: &[crate::metrics::Process]) {
        if !snapshot.seconds.is_finite() || snapshot.seconds < 0.0 {
            return;
        }
        let tick = (snapshot.seconds * self.sample_rate as f64 + 1e-7).floor() as u64;
        if self.history_tick.is_some_and(|last| tick <= last) {
            return;
        }
        let elapsed = self
            .history_time
            .map_or(0.0, |last| snapshot.seconds - last);
        if !snapshot.network.state.has_carrier() {
            self.network_activity_since = None;
            self.network_last_activity = None;
        } else if snapshot.network.state.has_traffic() {
            if self
                .network_last_activity
                .is_some_and(|last| snapshot.seconds - last > ACTIVITY_HOLD_SECONDS)
            {
                self.network_activity_since = None;
            }
            self.network_activity_since.get_or_insert(snapshot.seconds);
            self.network_last_activity = Some(snapshot.seconds);
        } else if self
            .network_last_activity
            .is_some_and(|last| snapshot.seconds - last > ACTIVITY_HOLD_SECONDS)
        {
            self.network_activity_since = None;
            self.network_last_activity = None;
        }
        self.cpu.push(snapshot);
        let layout = CpuLayout::new(snapshot.cpu.cores.len());
        if self.cpu_pixels.width != layout.graph_width as usize {
            // Rebuild only for a topology change; ordinary frames keep their fixed knots.
            let mut history = PixelHistory::new(layout.graph_width, self.sample_rate);
            for sample in &self.cpu.0 {
                history.observe(sample.seconds, sample.values, 100.0);
            }
            self.cpu_pixels = history;
        }
        self.cpu_layout = layout;
        self.download
            .push((snapshot.network.download_bps / MIB) as f32);
        self.upload.push((snapshot.network.upload_bps / MIB) as f32);
        self.disk_read.push((snapshot.disk.read_bps / MIB) as f32);
        self.disk_write.push((snapshot.disk.write_bps / MIB) as f32);
        self.download_scale.observe(self.download.peak(), elapsed);
        self.upload_scale.observe(self.upload.peak(), elapsed);
        self.disk_scale
            .observe(self.disk_read.peak().max(self.disk_write.peak()), elapsed);
        self.cpu_pixels.observe(
            snapshot.seconds,
            [snapshot.cpu.usage, snapshot.cpu.system, snapshot.cpu.iowait],
            100.0,
        );
        self.download_pixels.observe(
            snapshot.seconds,
            [(snapshot.network.download_bps / MIB) as f32, 0.0, 0.0],
            self.download_scale.value,
        );
        self.upload_pixels.observe(
            snapshot.seconds,
            [(snapshot.network.upload_bps / MIB) as f32, 0.0, 0.0],
            self.upload_scale.value,
        );
        self.disk_read_pixels.observe(
            snapshot.seconds,
            [(snapshot.disk.read_bps / MIB) as f32, 0.0, 0.0],
            self.disk_scale.value,
        );
        self.disk_write_pixels.observe(
            snapshot.seconds,
            [(snapshot.disk.write_bps / MIB) as f32, 0.0, 0.0],
            self.disk_scale.value,
        );
        if self
            .selection_time
            .is_none_or(|last| snapshot.seconds - last >= SELECTION_SECONDS - 1e-7)
        {
            self.top.update(snapshot.seconds, ranking);
            self.selection_time = Some(snapshot.seconds);
        }
        let selected = self.top.ids();
        self.process_traces
            .retain(|id, _| selected.contains(&Some(*id)));
        for id in selected.into_iter().flatten() {
            if let Some(process) = snapshot
                .processes
                .iter()
                .find(|p| p.id == id)
                .or_else(|| ranking.iter().find(|p| p.id == id))
            {
                let tracked = self
                    .process_traces
                    .entry(process.id)
                    .or_insert_with(|| ProcessTrace::new(self.sample_rate));
                tracked.trace.push(process.cpu);
                tracked.scale.observe(tracked.trace.peak(), elapsed);
                tracked.pixels.observe(
                    snapshot.seconds,
                    [process.cpu, 0.0, 0.0],
                    tracked.scale.value,
                );
            }
        }
        self.history_tick = Some(tick);
        self.history_time = Some(snapshot.seconds);
    }

    pub fn selected_ids(&self) -> [Option<u64>; 2] {
        self.top.ids()
    }

    /// One fixed screen: CPU hero, three resource columns, two spacious process rows.
    pub fn render(&self, snapshot: &Snapshot, frame: &mut Frame) {
        self.render_at(snapshot, frame, snapshot.seconds);
    }

    /// Presentation may follow the screen refresh clock while telemetry stays at 30 Hz.
    /// Only the X translation advances here; rendering never consumes history samples.
    pub fn render_at(&self, snapshot: &Snapshot, frame: &mut Frame, seconds: f64) {
        frame.clear(self.theme.background);
        self.borders(frame);
        self.render_cpu(snapshot, frame, seconds);
        self.render_memory(snapshot, frame);
        self.render_block(snapshot, frame, seconds);
        self.render_network(snapshot, frame, seconds);
        self.render_processes(snapshot, frame, seconds);
        self.render_uptime(snapshot, frame);
    }

    pub fn render_profiled(
        &self,
        snapshot: &Snapshot,
        frame: &mut Frame,
    ) -> [crate::profiling::Cost; 5] {
        use crate::profiling::Meter;
        frame.clear(self.theme.background);
        self.borders(frame);
        let mut costs = [crate::profiling::Cost::default(); 5];
        let mut meter = Meter::new(true);
        self.render_cpu(snapshot, frame, snapshot.seconds);
        costs[0] = meter.elapsed();
        meter = Meter::new(true);
        self.render_memory(snapshot, frame);
        costs[1] = meter.elapsed();
        meter = Meter::new(true);
        self.render_block(snapshot, frame, snapshot.seconds);
        costs[2] = meter.elapsed();
        meter = Meter::new(true);
        self.render_network(snapshot, frame, snapshot.seconds);
        costs[3] = meter.elapsed();
        meter = Meter::new(true);
        self.render_processes(snapshot, frame, snapshot.seconds);
        self.render_uptime(snapshot, frame);
        costs[4] = meter.elapsed();
        costs
    }

    fn borders(&self, f: &mut Frame) {
        let t = self.theme;
        f.line(0, 0, 159, 0, t.cpu_box);
        f.line(0, 1, 0, CPU_BOTTOM - 1, t.cpu_box);
        f.line(159, 1, 159, CPU_BOTTOM - 1, t.cpu_box);
        if let Some(x) = self.cpu_layout.separator_x() {
            f.line(x, 2, x, CPU_BOTTOM - 2, t.cpu_box.scale(0.45));
        }
        f.line(0, CPU_BOTTOM, MEMORY_RIGHT - 1, CPU_BOTTOM, t.mem_box);
        f.line(
            MEMORY_RIGHT,
            CPU_BOTTOM,
            BLOCK_RIGHT - 1,
            CPU_BOTTOM,
            t.mem_box,
        );
        f.line(BLOCK_RIGHT, CPU_BOTTOM, 159, CPU_BOTTOM, t.net_box);
        f.line(0, CPU_BOTTOM + 1, 0, 53, t.mem_box);
        f.line(
            MEMORY_RIGHT,
            CPU_BOTTOM + 1,
            MEMORY_RIGHT,
            53,
            t.mem_box.scale(0.65),
        );
        f.line(
            BLOCK_RIGHT,
            CPU_BOTTOM + 1,
            BLOCK_RIGHT,
            53,
            t.net_box.scale(0.75),
        );
        f.line(159, CPU_BOTTOM + 1, 159, 53, t.net_box);
        f.line(0, 54, 159, 54, t.proc_box);
        f.line(0, 55, 0, 78, t.proc_box);
        f.line(159, 55, 159, 78, t.proc_box);
        f.line(0, 79, 159, 79, t.proc_box);
    }

    fn render_cpu(&self, s: &Snapshot, f: &mut Frame, seconds: f64) {
        let t = self.theme;
        let usage = s.cpu.usage.clamp(0.0, 100.0);
        let usage_text = if usage >= 99.95 {
            "100%".into()
        } else {
            format!("{usage:.1}%")
        };
        f.value_right(
            CPU_LEFT_VALUE_RIGHT,
            5,
            &usage_text,
            t.cpu.at(usage / 100.0),
        );
        let temp = s
            .cpu
            .temperature
            .map_or_else(|| "--C".into(), |v| format!("{}C", v.round() as i32));
        f.value_right(
            CPU_LEFT_VALUE_RIGHT,
            15,
            &temp,
            s.cpu
                .temperature
                .map_or(t.muted, |v| t.temperature.at(v / 100.0)),
        );
        f.value_right(
            CPU_RIGHT_VALUE_RIGHT,
            15,
            &if s.cpu.frequency_mhz.is_finite() {
                format!("{:.1}G", s.cpu.frequency_mhz / 1000.0)
            } else {
                "--G".into()
            },
            t.text,
        );
        let power = s
            .cpu
            .power_watts
            .map_or_else(|| "--W".into(), |v| format!("{}W", v.round() as u32));
        f.value_right(CPU_RIGHT_VALUE_RIGHT, 5, &fit(&power, 4), t.cpu.at(0.0));

        graph(
            f,
            &self.cpu_pixels,
            Plot {
                seconds,
                x: CPU_GRAPH_X,
                y: 2,
                width: self.cpu_layout.graph_width,
                height: CPU_BOTTOM - 3,
                grid: t.plot_grid,
                grid_offset: (4, 4),
                gradient: t.cpu,
                inverted: false,
                fill: 0.32,
                antialias: self.antialias,
            },
        );
        let offset = self.cpu_pixels.offset_at(seconds);
        for x in 0..self.cpu_layout.graph_width {
            let height = (CPU_BOTTOM - 4) as f32;
            let point = self.cpu_pixels.sample_at(x as f32, offset);
            let total = point.ratio * height;
            let system = (point.system * height).min(total);
            let wait = (point.wait * height).min(total - system);
            if self.antialias {
                for dy in 0..(system + wait).ceil() as i32 {
                    let system_coverage = (system - dy as f32).clamp(0.0, 1.0);
                    let wait_coverage =
                        (system + wait - dy as f32).clamp(0.0, 1.0) - system_coverage;
                    let y = CPU_BOTTOM - 2 - dy;
                    if system_coverage > 0.0 {
                        f.blend_pixel(2 + x, y, t.cpu.at(0.15).scale(0.50), system_coverage);
                    }
                    if wait_coverage > 0.0 {
                        f.blend_pixel(2 + x, y, t.temperature.at(0.70).scale(0.70), wait_coverage);
                    }
                }
                continue;
            }
            let system = system.round() as i32;
            let wait = wait.round() as i32;
            for dy in 0..system {
                f.pixel(2 + x, CPU_BOTTOM - 2 - dy, t.cpu.at(0.15).scale(0.50));
            }
            for dy in 0..wait {
                f.pixel(
                    2 + x,
                    CPU_BOTTOM - 2 - system - dy,
                    t.temperature.at(0.70).scale(0.70),
                );
            }
        }
        for (index, core) in s.cpu.cores.iter().enumerate() {
            let bar = self.cpu_layout.bar(index);
            core_column(f, bar, core.usage, t.grid, t.cpu, self.antialias);
            if let (Some(y), Some(temp)) = (bar.indicator_y, core.temperature) {
                f.rect(bar.x, y, bar.width, 1, t.temperature.at(temp / 100.0));
            }
        }
    }

    fn render_memory(&self, s: &Snapshot, f: &mut Frame) {
        let t = self.theme;
        let m = &s.memory;
        let memory_color = if m.available_is_low() {
            t.used.at(1.0)
        } else {
            t.cached.at(0.50)
        };
        let swap_color = if m.swap_is_low() {
            t.used.at(1.0)
        } else {
            t.available.at(0.65)
        };
        Icon::Memory.draw(f, 2, MEMORY_TOP, memory_color);
        f.value(
            2,
            MEMORY_TOP + 6,
            &memory_capacity(m.used, m.total),
            memory_color,
        );
        Icon::Swap.draw(f, 2, SWAP_TOP, swap_color);
        f.value(
            2,
            SWAP_TOP + 6,
            &memory_capacity(m.swap_used, m.swap_total),
            swap_color,
        );

        let used = percent(m.used, m.total) / 100.0;
        let cached = percent(m.cached.min(m.total.saturating_sub(m.used)), m.total) / 100.0;
        // Reclaimable cache follows used clockwise; only free memory keeps the muted track.
        ring(
            f,
            &self.ring_mask,
            RING_X,
            MEMORY_TOP,
            &[
                (used, memory_color),
                (cached, t.free.at(0.60)),
                ((1.0 - used - cached).max(0.0), t.memory_track),
            ],
            self.antialias,
        );
        let swap = percent(m.swap_used, m.swap_total) / 100.0;
        ring(
            f,
            &self.ring_mask,
            RING_X,
            SWAP_TOP,
            &[(swap, swap_color), (1.0 - swap, t.swap_track)],
            self.antialias,
        );
        ring_number(
            f,
            (RING_X + 6) as f32,
            (MEMORY_TOP + 6) as f32,
            Some(percent(m.used, m.total)),
            memory_color,
        );
        ring_number(
            f,
            (RING_X + 6) as f32,
            (SWAP_TOP + 6) as f32,
            (m.swap_total != 0).then_some(swap * 100.0),
            if m.swap_total == 0 {
                t.muted
            } else {
                swap_color
            },
        );
    }

    fn render_block(&self, s: &Snapshot, f: &mut Frame, seconds: f64) {
        let t = self.theme;
        let d = &s.disk;
        let bar_x = MEMORY_RIGHT + 4;
        let bar_y = MEMORY_TOP + 2;
        let bar_width = DISK_VALUE_RIGHT - bar_x;
        let usage_color = t.available.at(0.55);
        bar(
            f,
            bar_x,
            bar_y,
            bar_width,
            2,
            percent(d.used, d.total) / 100.0,
            t.memory_track,
            usage_color,
        );

        f.value_right(
            DISK_VALUE_RIGHT,
            MEMORY_TOP + 6,
            &capacity_pair(d.used, d.total, 7),
            t.available.at(0.55),
        );
        // Queue-depth column and temperature cap keep the same width as a CPU core.
        f.rect(
            DISK_COLUMN_X,
            MEMORY_TOP,
            DISK_COLUMN_WIDTH,
            1,
            d.temperature
                .map_or(t.muted, |v| t.temperature.at(v / 100.0)),
        );
        vertical_column(
            f,
            Column {
                x: DISK_COLUMN_X,
                y: MEMORY_TOP + 2,
                width: DISK_COLUMN_WIDTH,
                height: 26,
                ratio: d.queue_depth.unwrap_or(0.0) / QUEUE_DEPTH_FULL,
                gradient: t.cpu,
            },
            t.grid,
            self.antialias,
        );

        Icon::TriangleDown.draw(f, MEMORY_RIGHT + 3, 40, t.cached.at(0.72));
        f.value_right(DISK_VALUE_RIGHT, 40, &rate(d.read_bps), t.cached.at(0.72));
        Icon::TriangleUp.draw(f, MEMORY_RIGHT + 3, 47, t.used.at(0.67));
        f.value_right(DISK_VALUE_RIGHT, 46, &rate(d.write_bps), t.used.at(0.67));
        // Fixed trace bays keep the layout still as numeric character counts change.
        graph(
            f,
            &self.disk_read_pixels,
            Plot {
                seconds,
                x: MEMORY_RIGHT + 10,
                y: 38,
                width: DISK_GRAPH_WIDTH,
                height: 6,
                grid: t.plot_grid,
                grid_offset: (GRID_STEP, 2),
                gradient: t.cached,
                inverted: false,
                fill: 0.32,
                antialias: self.antialias,
            },
        );
        graph(
            f,
            &self.disk_write_pixels,
            Plot {
                seconds,
                x: MEMORY_RIGHT + 10,
                y: 46,
                width: DISK_GRAPH_WIDTH,
                height: 6,
                grid: t.plot_grid,
                grid_offset: (GRID_STEP, 2),
                gradient: t.used,
                inverted: false,
                fill: 0.32,
                antialias: self.antialias,
            },
        );
    }

    fn render_network(&self, s: &Snapshot, f: &mut Frame, seconds: f64) {
        let t = self.theme;
        let n = &s.network;
        let graph_x = BLOCK_RIGHT + 3;
        let connected = n.state.has_carrier();
        let status_color = match n.state {
            NetworkState::Down => t.muted,
            NetworkState::NoCarrier => t.network_activity,
            NetworkState::Active | NetworkState::Transferring => t.free.at(0.70),
        };
        Icon::Ethernet.draw(
            f,
            NETWORK_ICON_X,
            MEMORY_TOP,
            if n.state == NetworkState::Down {
                t.muted
            } else {
                t.title
            },
        );
        f.value(
            NETWORK_ICON_X + 11,
            MEMORY_TOP + 1,
            n.link_label(),
            if connected { t.title } else { t.muted },
        );
        f.value_right(157, MEMORY_TOP + 1, n.state.status_label(), status_color);
        // Draw lamp surrounds over the original white lower corners.
        for x in [NETWORK_ICON_X, NETWORK_ICON_X + 8] {
            f.rect(x - 1, MEMORY_TOP + 4, 3, 4, Rgb(0, 0, 0));
        }
        f.rect(
            NETWORK_ICON_X,
            MEMORY_TOP + 5,
            1,
            2,
            if connected { t.free.at(0.70) } else { t.muted },
        );
        f.rect(
            NETWORK_ICON_X + 8,
            MEMORY_TOP + 5,
            1,
            2,
            if connected
                && self
                    .network_last_activity
                    .is_some_and(|last| seconds - last <= ACTIVITY_HOLD_SECONDS)
                && self.network_activity_since.is_some_and(|start| {
                    ((seconds - start).max(0.0) + 1e-9).rem_euclid(ACTIVITY_PERIOD)
                        < ACTIVITY_ON_SECONDS
                })
            {
                t.network_activity
            } else {
                t.muted
            },
        );
        let down = t.download.at(0.90);
        let up = t.upload.at(0.90);
        let down_rate = rate(n.download_bps);
        let up_rate = rate(n.upload_bps);
        arrow(f, NETWORK_ARROW_X, 35, false, down);
        arrow(f, NETWORK_ARROW_X, 45, true, up);
        // Restore the paired area graphs: download rises, upload falls below it.
        // The fixed graph bay remains full width when a rate gains a digit.
        graph(
            f,
            &self.download_pixels,
            Plot {
                seconds,
                x: graph_x,
                y: 33,
                width: NETWORK_GRAPH_WIDTH,
                height: 9,
                grid: t.plot_grid,
                grid_offset: (GRID_STEP, 2),
                gradient: t.download,
                inverted: false,
                fill: 0.48,
                antialias: self.antialias,
            },
        );
        graph(
            f,
            &self.upload_pixels,
            Plot {
                seconds,
                x: graph_x,
                y: 43,
                width: NETWORK_GRAPH_WIDTH,
                height: 9,
                grid: t.plot_grid,
                grid_offset: (GRID_STEP, 1),
                gradient: t.upload,
                inverted: true,
                fill: 0.48,
                antialias: self.antialias,
            },
        );
        f.value_right(157, 35, &down_rate, if connected { down } else { t.muted });
        f.value_right(157, 45, &up_rate, if connected { up } else { t.muted });
    }

    fn render_processes(&self, s: &Snapshot, f: &mut Frame, seconds: f64) {
        let t = self.theme;
        for (index, selected) in self.top.ids().into_iter().enumerate() {
            let y = PROCESS_ROW_TOP + index as i32 * PROCESS_ROW_PITCH;
            let Some(process) = selected.and_then(|id| s.processes.iter().find(|p| p.id == id))
            else {
                f.text(4, y, "--", t.muted);
                continue;
            };
            let smoothed = self.top.score(process.id);
            f.rect(2, y + 2, 1, 4, t.process.at(smoothed / 100.0));
            f.text(4, y, &fit(&process.name, PROCESS_NAME_CELLS), t.text);
            f.text_right(
                PROCESS_CPU_RIGHT,
                y,
                &format!("{}%", process.cpu.round().min(999.0) as u32),
                t.process.at(process.cpu / 100.0),
            );
            if let Some(tracked) = self.process_traces.get(&process.id) {
                graph(
                    f,
                    &tracked.pixels,
                    Plot {
                        seconds,
                        x: PROCESS_GRAPH_X,
                        y: y - 1,
                        width: PROCESS_GRAPH_WIDTH,
                        height: 9,
                        grid: t.plot_grid,
                        grid_offset: (GRID_STEP, 2),
                        gradient: t.process,
                        inverted: false,
                        fill: 0.32,
                        antialias: self.antialias,
                    },
                );
            }
            f.text_right(
                PROCESS_MEMORY_RIGHT,
                y,
                &bytes_short(process.rss),
                t.cached.at(0.65),
            );
        }
    }

    fn render_uptime(&self, s: &Snapshot, f: &mut Frame) {
        let t = self.theme;
        let uptime = uptime_short(s.uptime);
        // Keep the upper-left status readable over every CPU workload.
        f.rect(2, 2, 12 + Frame::value_width("00w00d"), 6, t.background);
        Icon::Plug.draw(f, 3, 2, t.free.at(0.65));
        f.value(13, 2, &uptime, t.muted);
    }
}

/// Geometry is cached once; 30 fps ring updates only classify precomputed angles.
struct RingPixel {
    samples: [f32; 16],
    coverage: u16,
    center: Option<f32>,
}

struct RingMask {
    diameter: i32,
    pixels: Vec<RingPixel>,
}

impl RingMask {
    fn new(diameter: i32, inner_radius: f32) -> Self {
        let center = (diameter - 1) as f32 / 2.0;
        let outer_squared = (diameter as f32 / 2.0).powi(2);
        let inner_squared = inner_radius.powi(2);
        let angle = |x: f32, y: f32| {
            let distance = x * x + y * y;
            (distance >= inner_squared && distance <= outer_squared)
                .then(|| (x.atan2(-y) / std::f32::consts::TAU).rem_euclid(1.0))
        };
        let mut pixels = Vec::with_capacity((diameter * diameter) as usize);
        for y in 0..diameter {
            for x in 0..diameter {
                let mut samples = [0.0; 16];
                let mut coverage = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let index = sy * 4 + sx;
                        let dx = x as f32 - center + (sx as f32 + 0.5) / 4.0 - 0.5;
                        let dy = y as f32 - center + (sy as f32 + 0.5) / 4.0 - 0.5;
                        if let Some(value) = angle(dx, dy) {
                            samples[index] = value;
                            coverage |= 1 << index;
                        }
                    }
                }
                pixels.push(RingPixel {
                    samples,
                    coverage,
                    center: angle(x as f32 - center, y as f32 - center),
                });
            }
        }
        Self { diameter, pixels }
    }
}

fn ring(f: &mut Frame, mask: &RingMask, x: i32, y: i32, segments: &[(f32, Rgb)], antialias: bool) {
    let color_at = |angle: f32| {
        let mut end = 0.0;
        for &(amount, color) in segments {
            end += amount.max(0.0);
            if angle < end {
                return color;
            }
        }
        segments.last().map_or(Rgb::default(), |s| s.1)
    };
    for (index, pixel) in mask.pixels.iter().enumerate() {
        let px = x + index as i32 % mask.diameter;
        let py = y + index as i32 / mask.diameter;
        if !antialias {
            if let Some(angle) = pixel.center {
                f.pixel(px, py, color_at(angle));
            }
            continue;
        }
        if pixel.coverage == 0 {
            continue;
        }
        let mut channels = [0u32; 3];
        let hits = pixel.coverage.count_ones();
        for sample in 0..16 {
            if pixel.coverage & (1 << sample) != 0 {
                let color = color_at(pixel.samples[sample]);
                channels[0] += color.0 as u32;
                channels[1] += color.1 as u32;
                channels[2] += color.2 as u32;
            }
        }
        let average = Rgb(
            (channels[0] / hits) as u8,
            (channels[1] / hits) as u8,
            (channels[2] / hits) as u8,
        );
        f.blend_pixel(px, py, average, hits as f32 / 16.0);
    }
}

fn ring_number(f: &mut Frame, center_x: f32, center_y: f32, value: Option<f32>, color: Rgb) {
    let number = value.map_or_else(
        || "--".into(),
        |v| format!("{}", v.round().clamp(0.0, 100.0) as u32),
    );
    let pitch = if number.len() == 3 { 3 } else { 4 };
    let width = (number.len() as i32 - 1) * pitch + 3;
    let x = (center_x - (width - 1) as f32 / 2.0).floor() as i32;
    let y = (center_y - 2.0).floor() as i32;
    f.digits_3x5(x, y, &number, pitch, color);
}

fn core_column(
    f: &mut Frame,
    bar: CpuBar,
    usage: f32,
    background: Rgb,
    gradient: Gradient,
    antialias: bool,
) {
    vertical_column(
        f,
        Column {
            x: bar.x,
            y: bar.y,
            width: bar.width,
            height: bar.height,
            ratio: usage / 100.0,
            gradient,
        },
        background,
        antialias,
    );
}

struct Column {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    ratio: f32,
    gradient: Gradient,
}

fn vertical_column(f: &mut Frame, c: Column, background: Rgb, antialias: bool) {
    f.rect(c.x, c.y, c.width, c.height, background);
    let exact_height = c.ratio.clamp(0.0, 1.0) * c.height as f32;
    let height = if antialias {
        exact_height.ceil() as i32
    } else {
        exact_height.round() as i32
    };
    for row in 0..height {
        let color = c.gradient.at(row as f32 / (c.height - 1).max(1) as f32);
        if antialias {
            let coverage = (exact_height - row as f32).clamp(0.0, 1.0);
            for column in 0..c.width {
                f.blend_pixel(c.x + column, c.y + c.height - 1 - row, color, coverage);
            }
        } else {
            f.rect(c.x, c.y + c.height - 1 - row, c.width, 1, color);
        }
    }
}

#[derive(Clone, Copy)]
struct Plot {
    seconds: f64,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    grid: Rgb,
    grid_offset: (i32, i32),
    gradient: Gradient,
    inverted: bool,
    fill: f32,
    antialias: bool,
}

fn graph(f: &mut Frame, trace: &PixelHistory, p: Plot) {
    graph_curve(f, trace, p);
    graph_grid(f, &p);
}

fn graph_grid(f: &mut Frame, p: &Plot) {
    for y in (p.y + p.grid_offset.1..p.y + p.height).step_by(GRID_STEP as usize) {
        for x in (p.x + p.grid_offset.0..p.x + p.width).step_by(GRID_STEP as usize) {
            // Stationary dots never change the color/coverage of a moving AA stroke.
            if f.pixels[y as usize * 160 + x as usize] == Rgb::default() {
                f.pixel(x, y, p.grid);
            }
        }
    }
}

/// Translate fixed knots instead of choosing a new sampling phase for the history.
/// Continuous distance coverage avoids quantizing motion into supersample steps.
fn graph_curve(f: &mut Frame, trace: &PixelHistory, p: Plot) {
    let width = p.width as usize;
    let mut tips = [0.0; 162];
    let mut colors = [Rgb::default(); 80];
    let mut segments = [StrokeSegment::default(); 161];
    let offset = trace.offset_at(p.seconds);
    let tip = |point: PlotPoint| {
        let height = point.ratio * (p.height - 1) as f32;
        let height = if p.antialias { height } else { height.round() };
        if p.inverted {
            p.y as f32 + height
        } else {
            (p.y + p.height - 1) as f32 - height
        }
    };
    for (row, color) in colors.iter_mut().enumerate().take(p.height as usize) {
        *color = p
            .gradient
            .at(row as f32 / (p.height - 1).max(1) as f32)
            .scale(p.fill);
    }
    for (column, y) in tips.iter_mut().enumerate().take(width + 1) {
        *y = tip(trace.at(column as i32 - 1));
    }
    tips[width + 1] = tip(trace.live);
    for index in 0..width + 1 {
        let x = index as f32 - 1.0 - offset;
        let end_x = if index == width {
            (width - 1) as f32
        } else {
            x + 1.0
        };
        segments[index] = StrokeSegment::new((x, tips[index]), (end_x, tips[index + 1]));
    }
    for column in 0..width {
        let ratio = trace.sample_at(column as f32, offset).ratio;
        let height = ratio * (p.height - 1) as f32;
        let height = if p.antialias { height } else { height.round() };
        for dy in 0..height.ceil() as i32 {
            let y = if p.inverted {
                p.y + dy
            } else {
                p.y + p.height - 1 - dy
            };
            let coverage = (height - dy as f32).clamp(0.0, 1.0);
            f.blend_pixel(p.x + column as i32, y, colors[dy as usize], coverage);
        }
        // At most three adjacent segments can cover this pixel, including the live tail.
        let center = ((column as f32 + offset).floor() as usize).min(width - 1);
        let local = &segments[center..(center + 3).min(width + 1)];
        let first_tip = tips[center];
        let top = ((tips[center + 1..center + local.len() + 1]
            .iter()
            .copied()
            .fold(first_tip, f32::min)
            - 1.0)
            .floor() as i32)
            .max(p.y);
        let bottom = ((tips[center + 1..center + local.len() + 1]
            .iter()
            .copied()
            .fold(first_tip, f32::max)
            + 1.0)
            .ceil() as i32)
            .min(p.y + p.height - 1);
        let color = p.gradient.at(ratio);
        for y in top..=bottom {
            let point = (column as f32, y as f32);
            let distance = local
                .iter()
                .map(|segment| segment.distance_squared(point))
                .fold(f32::INFINITY, f32::min);
            let coverage = if p.antialias {
                (1.0 - distance.sqrt()).max(0.0)
            } else {
                f32::from(distance <= 0.25)
            };
            if coverage > 0.0 {
                f.blend_pixel(p.x + column as i32, y, color, coverage);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct StrokeSegment {
    start: (f32, f32),
    dx: f32,
    dy: f32,
    length: f32,
}
impl StrokeSegment {
    fn new(start: (f32, f32), end: (f32, f32)) -> Self {
        let dx = end.0 - start.0;
        let dy = end.1 - start.1;
        Self {
            start,
            dx,
            dy,
            length: dx * dx + dy * dy,
        }
    }
    fn distance_squared(&self, point: (f32, f32)) -> f32 {
        let x = point.0 - self.start.0;
        let y = point.1 - self.start.1;
        let along = x * self.dx + y * self.dy;
        if along <= 0.0 {
            return x * x + y * y;
        }
        if along >= self.length {
            let x = x - self.dx;
            let y = y - self.dy;
            return x * x + y * y;
        }
        let cross = x * self.dy - y * self.dx;
        cross * cross / self.length
    }
}

#[allow(clippy::too_many_arguments)]
fn bar(
    f: &mut Frame,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    ratio: f32,
    background: Rgb,
    color: Rgb,
) {
    f.rect(x, y, width, height, background);
    f.rect(
        x,
        y,
        (ratio.clamp(0.0, 1.0) * width as f32).round() as i32,
        height,
        color,
    );
}

fn arrow(f: &mut Frame, x: i32, y: i32, up: bool, color: Rgb) {
    let flip = |dy| if up { 4 - dy } else { dy };
    f.line(x + 2, y + flip(0), x + 2, y + flip(4), color);
    f.pixel(x, y + flip(2), color);
    f.pixel(x + 1, y + flip(3), color);
    f.pixel(x + 3, y + flip(3), color);
    f.pixel(x + 4, y + flip(2), color);
}

fn percent(value: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (value as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32
    }
}

fn uptime_short(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    match seconds {
        s if s < MINUTE => format!("{s:02}s"),
        s if s < HOUR => format!("{:02}m{:02}s", s / MINUTE, s % MINUTE),
        s if s < DAY => format!("{:02}h{:02}m", s / HOUR, s / MINUTE % 60),
        s if s < WEEK => format!("{:02}d{:02}h", s / DAY, s / HOUR % 24),
        s => {
            // Two week digits have a finite range; saturate instead of wrapping or widening.
            let s = s.min(100 * WEEK - 1);
            format!("{:02}w{:02}d", s / WEEK, s / DAY % 7)
        }
    }
}

fn compact_decimal(value: f64) -> String {
    if value >= 10.0 {
        format!("{}", value.round() as u64)
    } else {
        let value = format!("{value:.1}");
        let value = value.strip_suffix(".0").unwrap_or(&value);
        value
            .strip_prefix('0')
            .filter(|v| v.starts_with('.'))
            .unwrap_or(value)
            .to_owned()
    }
}

fn rate(bytes: f64) -> String {
    if !bytes.is_finite() {
        return "--B".into();
    }
    let mut value = bytes.max(0.0);
    // Promote before rounding would create four digits; retain the binary byte units.
    for unit in ['B', 'K', 'M', 'G', 'T', 'P'] {
        if value < 999.5 || unit == 'P' {
            return if value < 9.95 {
                format!("{value:.1}{unit}")
            } else {
                format!("{}{unit}", value.round().min(999.0) as u32)
            };
        }
        value /= 1024.0;
    }
    unreachable!()
}

fn bytes_short(bytes: u64) -> String {
    let value = if bytes >= GIB as u64 {
        format!("{}G", compact_decimal(bytes as f64 / GIB))
    } else if bytes >= MIB as u64 {
        format!("{}M", (bytes as f64 / MIB).round() as u64)
    } else if bytes >= 1024 {
        format!("{}K", bytes / 1024)
    } else {
        format!("{bytes}B")
    };
    if value.len() <= 4 {
        value
    } else {
        rate(bytes as f64)
    }
}

fn capacity_pair(used: u64, total: u64, cells: usize) -> String {
    if total == 0 {
        return "--/--".into();
    }
    let (left, right) = shared_amounts(used, total, cells);
    format!("{left}/{right}")
}

fn memory_capacity(used: u64, total: u64) -> String {
    if total == 0 {
        return "--/--".into();
    }
    // Preserve a blank pixel before the ring even when both amounts gain digits.
    let pixel_budget = RING_X - 3;
    for (unit, divisor) in [("", GIB), ("T", GIB * 1024.0), ("P", GIB * 1024.0 * 1024.0)] {
        let left = used as f64 / divisor;
        let right = total as f64 / divisor;
        for pair in [
            format!("{left:.1}/{}{unit}", compact_decimal(right)),
            format!("{}/{}{unit}", compact_decimal(left), compact_decimal(right)),
        ] {
            if Frame::value_width(&pair) <= pixel_budget {
                return pair;
            }
        }
        let rounded = format!("{}/{}{unit}", left.round() as u64, right.round() as u64);
        if right >= 1.0 && Frame::value_width(&rounded) <= pixel_budget {
            return rounded;
        }
    }
    "--/--".into()
}

fn shared_amounts(left: u64, right: u64, cells: usize) -> (String, String) {
    for (unit, divisor) in [
        ('G', GIB),
        ('T', GIB * 1024.0),
        ('P', GIB * 1024.0 * 1024.0),
    ] {
        let a = left as f64 / divisor;
        let b = right as f64 / divisor;
        let left = compact_decimal(a);
        let right = format!("{}{unit}", compact_decimal(b));
        if left.len() + 1 + right.len() <= cells {
            return (left, right);
        }
        let left = format!("{}", a.round() as u64);
        let right = format!("{}{unit}", b.round() as u64);
        if left.len() + 1 + right.len() <= cells && a.max(b) >= 1.0 {
            return (left, right);
        }
    }
    ("--".into(), "--".into())
}

fn fit(value: &str, cells: usize) -> String {
    if value.chars().count() <= cells {
        value.to_owned()
    } else {
        value
            .chars()
            .take(cells.saturating_sub(1))
            .chain(std::iter::once('~'))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{metrics::Core, MockSource, Scene, ThemeKind};

    #[test]
    fn cpu_threads_have_independent_right_aligned_bars_and_preserve_padding() {
        let theme = Theme::new(ThemeKind::Default);
        for count in [1, 4, 8, 12, 16, 20, 24, 32, 40, 41, 48, 64, 128, 256] {
            let mut snapshot = MockSource::snapshot(Scene::Balanced, 0.0);
            snapshot.cpu.cores = (0..count)
                .map(|index| Core {
                    usage: if index % 2 == 0 { 0.0 } else { 100.0 },
                    temperature: Some(20.0 + (index % 60) as f32),
                })
                .collect();
            let mut dashboard = Dashboard::new(theme);
            dashboard.observe(&snapshot);
            let layout = dashboard.cpu_layout;
            assert!(layout.graph_width >= CPU_GRAPH_MIN_WIDTH);
            assert_eq!(dashboard.cpu_pixels.width, layout.graph_width as usize);
            let mut frame = Frame::default();
            dashboard.render(&snapshot, &mut frame);
            let mut occupied = [false; 160 * 22];
            for (index, core) in snapshot.cpu.cores.iter().enumerate() {
                let bar = layout.bar(index);
                assert!(
                    bar.height > 0,
                    "each of {count} threads needs its own visible bar"
                );
                assert!(bar.x >= CPU_GRAPH_X + layout.graph_width + 4);
                assert!(bar.x + bar.width <= CPU_BAR_RIGHT);
                assert!(bar.y + bar.height < CPU_BOTTOM);
                if (index + 1) % layout.columns == 0 || index + 1 == count {
                    assert_eq!(
                        bar.x + bar.width,
                        CPU_BAR_RIGHT,
                        "each row is right aligned"
                    );
                }
                let expected = if core.usage == 0.0 {
                    theme.grid
                } else {
                    theme.cpu.at(0.0)
                };
                for y in bar.y..bar.y + bar.height {
                    for x in bar.x..bar.x + bar.width {
                        let pixel = y as usize * 160 + x as usize;
                        assert!(
                            !occupied[pixel],
                            "thread bars must not overlap: count={count} index={index}"
                        );
                        occupied[pixel] = true;
                    }
                }
                assert_eq!(
                    frame.pixels[(bar.y + bar.height - 1) as usize * 160 + bar.x as usize],
                    expected,
                    "individual thread usage must not be averaged: count={count} index={index}"
                );
                if let Some(y) = bar.indicator_y {
                    assert_eq!(
                        frame.pixels[y as usize * 160 + bar.x as usize],
                        theme.temperature.at(core.temperature.unwrap() / 100.0)
                    );
                }
            }
            let separator = layout.separator_x().unwrap();
            for y in 2..CPU_BOTTOM - 1 {
                for x in [separator - 1, separator + 1, CPU_BAR_RIGHT] {
                    assert_eq!(
                        frame.pixels[y as usize * 160 + x as usize],
                        theme.background,
                        "responsive CPU bays keep their padding: count={count} x={x}"
                    );
                }
            }
        }
        assert_eq!(CpuLayout::new(4).graph_width, 102);
        assert_eq!(CpuLayout::new(8).graph_width, 86);
        assert!(CpuLayout::new(16).graph_width < CpuLayout::new(8).graph_width);
    }

    #[test]
    fn cpu_topology_resize_reuses_history_and_keeps_the_same_time_window() {
        let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
        for tick in 0..=180 {
            let mut snapshot = MockSource::snapshot(Scene::Balanced, tick as f64 / 30.0);
            snapshot.cpu.cores.truncate(4);
            dashboard.observe(&snapshot);
        }
        assert_eq!(dashboard.cpu_pixels.width, 102);
        let mut snapshot = MockSource::snapshot(Scene::Balanced, 181.0 / 30.0);
        snapshot.cpu.cores.resize(16, snapshot.cpu.cores[0].clone());
        dashboard.observe(&snapshot);
        let history = &dashboard.cpu_pixels;
        assert_eq!(history.width, CpuLayout::new(16).graph_width as usize);
        assert!(
            history.columns.len() >= history.width - 1,
            "resize must not blank the history"
        );
        assert!((history.period * (history.width - 1) as f64 - 127.0 / 30.0).abs() < 1e-9);
        let mut replayed = PixelHistory::new(history.width as i32, 30);
        for sample in &dashboard.cpu.0 {
            replayed.observe(sample.seconds, sample.values, 100.0);
        }
        assert_eq!(history.columns, replayed.columns);
        assert_eq!(history.live, replayed.live);
        let before = history.columns.clone();
        let mut frame = Frame::default();
        dashboard.render_at(&snapshot, &mut frame, snapshot.seconds + 1.0 / 60.0);
        assert_eq!(
            dashboard.cpu_pixels.columns, before,
            "presentation must not resample resized history"
        );
    }

    #[test]
    fn uptime_promotes_at_unit_boundaries_and_keeps_two_digit_fields() {
        for (seconds, expected) in [
            (0, "00s"),
            (9, "09s"),
            (59, "59s"),
            (60, "01m00s"),
            (69, "01m09s"),
            (3599, "59m59s"),
            (3600, "01h00m"),
            (7199, "01h59m"),
            (86399, "23h59m"),
            (86400, "01d00h"),
            (3 * 86400 + 5 * 3600, "03d05h"),
            (604799, "06d23h"),
            (604800, "01w00d"),
            (2 * 604800 + 3 * 86400, "02w03d"),
            (100 * 604800 - 1, "99w06d"),
            (100 * 604800, "99w06d"),
            (u64::MAX, "99w06d"),
        ] {
            let actual = uptime_short(seconds);
            assert_eq!(actual, expected, "uptime at {seconds} seconds");
            assert!(Frame::value_width(&actual) <= 23);
        }
    }

    #[test]
    fn history_accepts_all_thirty_ticks_per_second() {
        let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
        for tick in 0..=30 {
            dashboard.observe(&MockSource::snapshot(Scene::Balanced, tick as f64 / 30.0));
        }
        assert_eq!(dashboard.cpu.0.len(), 31);
        assert_eq!(dashboard.disk_read.0.len(), 31);
        assert_eq!(dashboard.download.0.len(), 31);
        assert_eq!(
            dashboard
                .process_traces
                .values()
                .next()
                .unwrap()
                .trace
                .0
                .len(),
            31
        );
        dashboard.observe(&MockSource::snapshot(Scene::Balanced, 1.0));
        assert_eq!(
            dashboard.cpu.0.len(),
            31,
            "a paused frame never advances history"
        );
    }

    #[test]
    fn axis_ema_removes_double_steps_and_releases_slowly() {
        let mut scale = EmaScale::new(RATE_SCALE_MIN);
        scale.observe(10.0, 0.0);
        assert_eq!(scale.value, 11.0);
        scale.observe(20.0, 1.0 / 30.0);
        assert!(
            scale.value > 11.0 && scale.value < 12.0,
            "a higher peak must not double the axis in one frame"
        );
        for _ in 0..180 {
            scale.observe(20.0, 1.0 / 30.0);
        }
        let before = scale.value;
        scale.observe(10.0, 1.0 / 30.0);
        assert!(
            scale.value < before && scale.value > before * 0.99,
            "a departing peak must not halve the axis"
        );
        for _ in 0..900 {
            scale.observe(0.0, 1.0 / 30.0);
        }
        assert!(scale.value >= RATE_SCALE_MIN && scale.value < 0.02);
    }

    #[test]
    fn axis_ema_uses_elapsed_time_and_does_not_advance_while_paused() {
        let settle = |fps: u32| {
            let mut scale = EmaScale::new(PROCESS_SCALE_MIN);
            scale.observe(10.0, 0.0);
            for _ in 0..fps * 2 {
                scale.observe(30.0, 1.0 / fps as f64);
            }
            for _ in 0..fps * 2 {
                scale.observe(5.0, 1.0 / fps as f64);
            }
            scale.value
        };
        assert!((settle(1) - settle(30)).abs() < 0.0001);
        assert!((settle(30) - settle(60)).abs() < 0.0001);
        let mut scale = EmaScale::new(PROCESS_SCALE_MIN);
        scale.observe(30.0, 0.0);
        let before = scale.value;
        scale.observe(5.0, 0.0);
        assert_eq!(scale.value, before);
    }

    #[test]
    fn completed_knots_move_continuously_without_rephasing_or_rescaling() {
        for width in [
            CpuLayout::new(4).graph_width,
            CpuLayout::new(8).graph_width,
            CpuLayout::new(32).graph_width,
            NETWORK_GRAPH_WIDTH,
            DISK_GRAPH_WIDTH,
            PROCESS_GRAPH_WIDTH,
        ] {
            for aa in [false, true] {
                for inverted in [false, true] {
                    let mut history = PixelHistory::new(width, 30);
                    let draw = |history: &PixelHistory, seconds| {
                        let mut frame = Frame::default();
                        graph(
                            &mut frame,
                            history,
                            Plot {
                                seconds,
                                x: 2,
                                y: 2,
                                width,
                                height: 9,
                                grid: Rgb::hex(0x505050),
                                grid_offset: (5, 2),
                                gradient: Gradient::hex(0x80d0a3, 0xdcd179, 0xd45454),
                                inverted,
                                fill: 0.32,
                                antialias: aa,
                            },
                        );
                        frame.quantize_565();
                        frame
                    };
                    let mut previous = None::<(u64, f32, Vec<PlotPoint>)>;
                    for tick in 0..=180 {
                        let seconds = tick as f64 / 30.0;
                        let value = (seconds * 5.0).sin() * 25.0 + 50.0;
                        history.observe(
                            seconds,
                            [value as f32, 0.0, 0.0],
                            if tick < 155 { 100.0 } else { 180.0 },
                        );
                        let offset = history.offset_at(seconds);
                        if let Some((bucket, old_offset, old_points)) = previous {
                            let shift = (history.bucket - bucket) as usize;
                            if tick > 150 {
                                assert!(shift <= 1);
                                let distance = shift as f32 + offset - old_offset;
                                assert!((distance - (1.0 / 30.0 / history.period) as f32).abs() < 1e-6,
                                    "phase must remain continuous at bucket wrap: width={width} tick={tick}");
                                for x in -1..width - 1 - shift as i32 {
                                    assert_eq!(history.at(x), old_points[(x + 1 + shift as i32) as usize],
                                        "completed Y coordinates must stay frozen: width={width} tick={tick} x={x}");
                                }
                            }
                        }
                        previous = Some((
                            history.bucket,
                            offset,
                            (-1..width - 1).map(|x| history.at(x)).collect(),
                        ));
                    }
                    // A complete column period is exactly a one-pixel translation at any phase.
                    let seconds = 6.0;
                    let old = draw(&history, seconds);
                    let old_offset = history.offset_at(seconds);
                    history.observe(seconds + history.period, [73.0, 0.0, 0.0], 250.0);
                    assert!(
                        (history.offset_at(seconds + history.period) - old_offset).abs() < 1e-6
                    );
                    let frame = draw(&history, seconds + history.period);
                    let curve_only = |color| {
                        if color == Rgb::from_565(Rgb::hex(0x505050).to_565()) {
                            Rgb::default()
                        } else {
                            color
                        }
                    };
                    for y in 2..11 {
                        for x in 0..width as usize - 4 {
                            assert_eq!(curve_only(frame.pixels[y * 160 + 2 + x]), curve_only(old.pixels[y * 160 + 3 + x]),
                                "a full phase cycle must only translate pixels: width={width} aa={aa} inverted={inverted} x={x}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn narrow_aa_curves_move_on_every_frame_between_column_boundaries() {
        for width in [NETWORK_GRAPH_WIDTH, DISK_GRAPH_WIDTH, PROCESS_GRAPH_WIDTH] {
            let mut history = PixelHistory::new(width, 30);
            for tick in 0..=180 {
                let seconds = tick as f64 / 30.0;
                let value = (seconds * 5.0).sin() * 25.0 + 50.0;
                history.observe(seconds, [value as f32, 0.0, 0.0], 100.0);
            }
            let frozen: Vec<_> = history.columns.iter().copied().collect();
            let mut previous = None;
            // No new telemetry: this verifies movement of the already completed history alone.
            for tick in 0..8 {
                let mut frame = Frame::default();
                graph(
                    &mut frame,
                    &history,
                    Plot {
                        seconds: 6.0 + tick as f64 / 60.0,
                        x: 2,
                        y: 2,
                        width,
                        height: 9,
                        grid: Rgb::default(),
                        grid_offset: (5, 2),
                        gradient: Gradient::hex(0x80d0a3, 0xdcd179, 0xd45454),
                        inverted: false,
                        fill: 0.32,
                        antialias: true,
                    },
                );
                frame.quantize_565();
                let curve: Vec<_> = (2..11)
                    .flat_map(|y| {
                        frame.pixels[y * 160 + 2..y * 160 + width as usize - 2]
                            .iter()
                            .copied()
                    })
                    .collect();
                if let Some(old) = previous {
                    assert_ne!(
                        curve, old,
                        "a narrow graph must move on every refresh: width={width} tick={tick}"
                    );
                }
                previous = Some(curve);
            }
            assert_eq!(history.columns.iter().copied().collect::<Vec<_>>(), frozen);
        }
    }

    #[test]
    fn each_process_keeps_its_axis_and_new_id_starts_with_fresh_scale() {
        let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
        let mut snapshot = MockSource::snapshot(Scene::Balanced, 0.0);
        snapshot.processes.truncate(2);
        snapshot.processes[0].cpu = 90.0;
        snapshot.processes[1].cpu = 10.0;
        dashboard.observe(&snapshot);
        let first = snapshot.processes[0].id;
        let second = snapshot.processes[1].id;
        assert_eq!(dashboard.process_traces[&first].scale.value, 99.0);
        assert_eq!(dashboard.process_traces[&second].scale.value, 11.0);
        snapshot.seconds = 1.0;
        snapshot.processes[0].id = first + 100_000;
        snapshot.processes[0].cpu = 1.0;
        dashboard.observe(&snapshot);
        assert!(!dashboard.process_traces.contains_key(&first));
        assert_eq!(
            dashboard.process_traces[&snapshot.processes[0].id]
                .scale
                .value,
            PROCESS_SCALE_MIN
        );
        assert_eq!(dashboard.process_traces[&second].scale.value, 11.0);
    }

    #[test]
    fn only_displayed_processes_receive_per_frame_histories() {
        let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
        let mut s = MockSource::snapshot(Scene::Balanced, 0.0);
        for id in 10000..11000 {
            let mut p = s.processes[0].clone();
            p.id = id;
            p.cpu = 0.0;
            s.processes.push(p);
        }
        dashboard.observe(&s);
        assert_eq!(dashboard.process_traces.len(), 2);
    }

    #[test]
    fn rates_fit_a_fixed_four_character_bay() {
        for bytes in [
            0.0,
            512.0,
            10_000.0,
            160_000.0,
            4.2 * MIB,
            99.9 * MIB,
            290.0 * MIB,
            1.2 * GIB,
        ] {
            assert!(
                rate(bytes).len() <= 4,
                "rate {bytes} must leave its chart bay intact"
            );
        }
    }

    #[test]
    fn rates_retain_one_decimal_below_ten_and_never_overflow_the_bay() {
        for (bytes, expected) in [
            (0.0, "0.0B"),
            (1.0, "1.0B"),
            (9.9, "9.9B"),
            (9.96, "10B"),
            (10.0, "10B"),
            (999.0, "999B"),
            (1023.0, "1.0K"),
            (1024.0, "1.0K"),
            (MIB, "1.0M"),
            (9.9 * MIB, "9.9M"),
            (10.0 * MIB, "10M"),
            (999.0 * MIB, "999M"),
            (GIB, "1.0G"),
            (GIB * 1024.0, "1.0T"),
            (-1.0, "0.0B"),
            (f64::NAN, "--B"),
        ] {
            assert_eq!(rate(bytes), expected);
            assert!(Frame::value_width(expected) <= 15);
        }
    }

    #[test]
    fn disk_temperature_and_queue_depth_control_separate_parts_of_the_indicator() {
        let dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
        let mut snapshot = MockSource::snapshot(Scene::Balanced, 70.0);
        let mut low = Frame::default();
        let mut high = Frame::default();
        snapshot.disk.queue_depth = Some(0.0);
        dashboard.render_block(&snapshot, &mut low, snapshot.seconds);
        snapshot.disk.queue_depth = Some(QUEUE_DEPTH_FULL);
        dashboard.render_block(&snapshot, &mut high, snapshot.seconds);
        let region = |f: &Frame, top: usize, height: usize| {
            (top..top + height)
                .flat_map(|y| {
                    f.pixels[y * 160 + DISK_COLUMN_X as usize
                        ..y * 160 + (DISK_COLUMN_X + DISK_COLUMN_WIDTH) as usize]
                        .iter()
                        .copied()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(region(&low, 24, 1), region(&high, 24, 1));
        assert_ne!(region(&low, 26, 26), region(&high, 26, 26));
        snapshot.disk.temperature = Some(90.0);
        dashboard.render_block(&snapshot, &mut low, snapshot.seconds);
        assert_ne!(region(&low, 24, 1), region(&high, 24, 1));
        assert_eq!(region(&low, 26, 26), region(&high, 26, 26));
    }

    #[test]
    fn memory_cache_follows_used_and_pressure_recolors_only_the_affected_resource() {
        let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
        let theme = dashboard.theme;
        let mut snapshot = MockSource::snapshot(Scene::Balanced, 0.0);
        snapshot.memory.total = 1000;
        snapshot.memory.used = 250;
        snapshot.memory.cached = 250;
        snapshot.memory.free = 500;
        snapshot.memory.swap_total = 1000;
        snapshot.memory.swap_used = 250;
        for aa in [false, true] {
            dashboard.antialias = aa;
            let mut frame = Frame::default();
            dashboard.render_memory(&snapshot, &mut frame);
            assert_eq!(
                frame.pixels[26 * 160 + 32],
                theme.cached.at(0.50),
                "used starts at the top"
            );
            assert_eq!(
                frame.pixels[34 * 160 + 32],
                theme.free.at(0.60),
                "cache follows used clockwise"
            );
            assert_eq!(
                frame.pixels[34 * 160 + 24],
                theme.memory_track,
                "free remains the muted track"
            );
        }
        snapshot.memory.used = 920;
        snapshot.memory.cached = 50;
        snapshot.memory.free = 30;
        let mut frame = Frame::default();
        dashboard.render_memory(&snapshot, &mut frame);
        assert_eq!(frame.pixels[24 * 160 + 2], theme.used.at(1.0));
        assert_eq!(frame.pixels[41 * 160 + 3], theme.available.at(0.65));
        snapshot.memory.used = 250;
        snapshot.memory.cached = 250;
        snapshot.memory.free = 500;
        snapshot.memory.swap_used = 920;
        dashboard.render_memory(&snapshot, &mut frame);
        assert_eq!(frame.pixels[24 * 160 + 2], theme.cached.at(0.50));
        assert_eq!(frame.pixels[41 * 160 + 3], theme.used.at(1.0));
    }

    #[test]
    fn tiny_plots_still_cover_the_whole_history_window() {
        for width in [
            CpuLayout::new(4).graph_width,
            CpuLayout::new(8).graph_width,
            CpuLayout::new(32).graph_width,
            NETWORK_GRAPH_WIDTH,
            DISK_GRAPH_WIDTH,
            PROCESS_GRAPH_WIDTH,
        ] {
            let history = PixelHistory::new(width, 30);
            assert!((history.period * (width - 1) as f64 - 127.0 / 30.0).abs() < 1e-9);
        }
    }

    #[test]
    fn core_aa_updates_between_integer_heights_and_preserves_endpoints() {
        let theme = Theme::new(ThemeKind::Default);
        let draw = |usage, aa| {
            let mut frame = Frame::default();
            core_column(
                &mut frame,
                CpuBar {
                    x: 74,
                    y: 4,
                    width: 3,
                    height: 17,
                    indicator_y: Some(2),
                },
                usage,
                theme.grid,
                theme.cpu,
                aa,
            );
            frame.quantize_565();
            frame
        };
        assert_eq!(draw(50.1, false).pixels, draw(50.8, false).pixels);
        assert_ne!(draw(50.1, true).pixels, draw(50.8, true).pixels);
        for usage in [0.0, 100.0] {
            assert_eq!(draw(usage, true).pixels, draw(usage, false).pixels);
        }
    }

    #[test]
    fn ring_coverage_keeps_the_hole_empty_at_zero_and_full_usage() {
        let mask = RingMask::new(13, 5.0);
        for aa in [true, false] {
            for used in [0.0, 1.0] {
                let mut frame = Frame::default();
                ring(
                    &mut frame,
                    &mask,
                    46,
                    26,
                    &[(used, Rgb(255, 0, 0)), (1.0 - used, Rgb(0, 255, 0))],
                    aa,
                );
                assert_eq!(
                    frame.pixels[32 * 160 + 52],
                    Rgb::default(),
                    "ring hole stays empty"
                );
                for pixel in frame.pixels.iter().filter(|p| **p != Rgb::default()) {
                    if used == 0.0 {
                        assert_eq!(pixel.0, 0);
                    } else {
                        assert_eq!(pixel.1, 0);
                    }
                }
            }
        }
    }

    #[test]
    fn small_ring_counters_stay_five_rows_high_for_zero_full_and_missing_values() {
        for (value, width) in [(Some(0.0), 3), (Some(45.0), 7), (Some(100.0), 9), (None, 7)] {
            let mut frame = Frame::default();
            ring_number(&mut frame, 52.0, 32.0, value, Rgb(255, 255, 255));
            let lit: Vec<_> = frame
                .pixels
                .iter()
                .enumerate()
                .filter(|(_, p)| **p != Rgb::default())
                .map(|(i, _)| i)
                .collect();
            assert!(!lit.is_empty());
            let left = lit.iter().map(|i| i % 160).min().unwrap();
            let right = lit.iter().map(|i| i % 160).max().unwrap();
            let top = lit.iter().map(|i| i / 160).min().unwrap();
            let bottom = lit.iter().map(|i| i / 160).max().unwrap();
            assert_eq!(
                right - left + 1,
                width,
                "counter width fits the existing ring"
            );
            assert!(
                top >= 30 && bottom <= 34,
                "all counter ink fits the five-row slot"
            );
            assert_eq!(
                (left + right) as f32 / 2.0,
                52.0,
                "counter and ring share their horizontal center"
            );
            if value.is_some() {
                assert_eq!(bottom - top + 1, 5);
                assert_eq!(
                    (top + bottom) as f32 / 2.0,
                    32.0,
                    "counter and ring share their vertical center"
                );
            }
        }
    }

    #[test]
    fn memory_capacity_preserves_both_amounts_within_the_left_column() {
        assert_eq!(
            memory_capacity((3.7 * GIB) as u64, (8.0 * GIB) as u64),
            "3.7/8"
        );
        assert_eq!(
            memory_capacity((1.1 * GIB) as u64, (2.0 * GIB) as u64),
            "1.1/2"
        );
        assert_eq!(memory_capacity(0, 0), "--/--");
        for total in [2.0, 8.0, 16.0, 128.0, 1024.0] {
            let value = memory_capacity((total * GIB * 0.8) as u64, (total * GIB) as u64);
            assert!(
                Frame::value_width(&value) <= RING_X - 3 && value.contains('/'),
                "capacity cannot enter the ring column"
            );
        }
    }
}
