// The original implementation is included unchanged, so all plots, gradients,
// subpixel history, AA, rings and activity-lamp timing are the upstream algorithms.
include!("../../src/ui.rs");

pub struct MacPanels {
    pub gpu: f32,
    pub gpu_available: bool,
    pub demo: bool,
    pub volumes: Vec<crate::MacVolume>,
    gpu_pixels: PixelHistory,
    gpu_time: Option<f64>,
}
impl MacPanels {
    pub fn new() -> Self {
        Self { gpu: 0.0, gpu_available: false, demo: false, volumes: Vec::new(),
            gpu_pixels: PixelHistory::new(48, 30), gpu_time: None }
    }
    pub fn observe(&mut self, shown: &Snapshot, _dashboard: &Dashboard, gpu: f32, available: bool) {
        let dt = self.gpu_time.map_or(0.0, |before| shown.seconds - before);
        self.gpu = if self.gpu_available && available && dt > 0.0 && dt < 2.0 {
            self.gpu + (gpu - self.gpu) * (1.0 - (-dt / 0.65).exp()) as f32
        } else { gpu };
        self.gpu_available = available; self.gpu_time = Some(shown.seconds);
        self.gpu_pixels.observe(shown.seconds, [self.gpu, 0.0, 0.0], 100.0);
    }
}

impl Dashboard {
    /// Preserve the original 160x80 resource layout on the left. Add a narrower
    /// 96x80 GPU/storage extension; disk names are overlaid at display resolution.
    pub fn render_mac_panels(&self, s: &Snapshot, panels: &MacPanels, left: &mut Frame, right: &mut Frame, seconds: f64) {
        let t = self.theme;
        self.render_at(s, left, seconds);
        // The optional extension adds only the right panel. The original panel
        // keeps exactly the same fonts, layout and animation in either mode.
        if panels.demo { left.rect(12, 1, 29, 7, t.background); left.value(13, 2, "DEMO", t.network_activity); }

        right.clear(t.background);
        let gpu_gradient = Gradient::hex(0x291f75, 0x4f43a3, 0xb0a9de);
        let gpu_color = gpu_gradient.at(0.95);
        right.value(3, 4, "GPU", gpu_color);
        right.value_right(92, 4, &if panels.gpu_available { format!("{:.1}%", panels.gpu) } else { "--".into() }, gpu_color);
        graph(right, &panels.gpu_pixels, Plot { seconds, x: 20, y: 3, width: 48, height: 8,
            grid: t.plot_grid, grid_offset: (3, 2), gradient: gpu_gradient, inverted: false, fill: 0.32, antialias: self.antialias });
        right.line(0, 0, 95, 0, t.net_box); right.line(0, 0, 0, 14, t.net_box); right.line(95, 0, 95, 14, t.net_box);
        right.line(0, 14, 95, 14, t.mem_box);
        // Eight source pixels are reserved for a crisp 6x8/native volume label.
        // The label itself never goes through this low-resolution framebuffer.
        for (i, volume) in panels.volumes.iter().take(4).enumerate() {
            let y = 15 + i as i32 * 16;
            let pair = format!("{:.0}/{:.0}G", volume.used as f64 / 1e9, volume.total as f64 / 1e9);
            let ratio = if volume.total > 0 { volume.used as f32 / volume.total as f32 } else { 0.0 };
            let color = if ratio > 0.9 { t.used.at(1.0) } else if ratio > 0.75 { t.available.at(0.65) } else { t.cached.at(0.5) };
            right.value(3, y+9, &format!("{:.0}%", ratio * 100.0), color);
            right.value_right(92, y+9, &pair, t.text);
            bar(right, 3, y+15, 90, 1, ratio, t.memory_track, color);
        }
        right.line(0, 15, 0, 79, t.mem_box); right.line(95, 15, 95, 79, t.mem_box); right.line(0, 79, 95, 79, t.mem_box);
    }
}
