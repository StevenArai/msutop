//! C ABI around the UNMODIFIED upstream animation/rendering core.
//! macOS supplies real telemetry; Dashboard, Smoother, fonts, AA, RGB565,
//! selection hysteresis and activity-lamp timing are upstream msutop code.
use msutop::metrics::{Core, Cpu, Disk, Memory, Network, Process};
use msutop::smoothing::Smoother;
pub use msutop::{icons, metrics, profiling, selection, theme, Frame, Rgb, Theme};
use msutop::{MockSource, NetworkState, Scene, Snapshot, ThemeKind};
use std::ffi::{c_char, c_int, c_void, CStr};
#[allow(dead_code)]
mod canvases;
use canvases::{Dashboard, MacPanels};

#[repr(C)]
pub struct MacSample {
    cpu_user: f64,
    cpu_system: f64,
    cores: [f64; 128],
    core_count: c_int,
    cpu_available: c_int,
    memory_total: u64,
    memory_used: u64,
    memory_cached: u64,
    memory_wired: u64,
    memory_compressed: u64,
    swap_used: u64,
    swap_total: u64,
    memory_available: c_int,
    swap_available: c_int,
    gpu_usage: f64,
    gpu_available: c_int,
    download: f64,
    upload: f64,
    disk_read: f64,
    disk_write: f64,
    uptime: f64,
    network_available: c_int,
    disk_io_available: c_int,
    interfaces: [c_char; 96],
    link_mbps: u32,
    cpu_power_watts: f64,
    cpu_temperature_c: f64,
    cpu_frequency_mhz: f64,
    cpu_sensor_flags: u32,
}
#[repr(C)]
pub struct MacProcess {
    pid: c_int,
    cpu: f64,
    memory: u64,
    identity: u64,
    name: [c_char; 128],
}
#[repr(C)]
#[derive(Clone)]
pub struct MacVolume {
    used: u64,
    total: u64,
    name: [c_char; 128],
}
pub fn volume_name(volume: &MacVolume) -> String {
    unsafe {
        CStr::from_ptr(volume.name.as_ptr())
            .to_string_lossy()
            .into_owned()
    }
}

struct Engine {
    dashboard: Dashboard,
    smoother: Smoother,
    frame: Frame,
    shown: Option<Snapshot>,
    vivid: bool,
    antialias: bool,
    demo: bool,
    panels: MacPanels,
    left: Frame,
    right: Frame,
}
impl Engine {
    fn new(vivid: bool, antialias: bool, demo: bool) -> Self {
        let theme = Theme::new(if vivid {
            ThemeKind::Vivid
        } else {
            ThemeKind::Default
        });
        let mut dashboard = Dashboard::with_sample_rate(theme, 30);
        dashboard.antialias = antialias;
        Self {
            dashboard,
            smoother: Smoother::default(),
            frame: Frame::default(),
            shown: None,
            vivid,
            antialias,
            demo,
            panels: MacPanels::new(),
            left: Frame::default(),
            right: Frame::default(),
        }
    }
    fn observe(&mut self, native: Snapshot) {
        // The upstream demo intentionally has exaggerated mock workloads. Never use it for live metrics.
        let raw = if self.demo {
            MockSource::snapshot(Scene::Balanced, native.seconds)
        } else {
            native
        };
        let shown = if self.demo {
            raw
        } else {
            self.smoother.apply(&raw, self.dashboard.selected_ids())
        };
        self.dashboard.observe(&shown);
        self.shown = Some(shown);
    }
    fn render(&mut self, seconds: f64) -> bool {
        let Some(shown) = &self.shown else {
            return false;
        };
        self.dashboard.render_at(shown, &mut self.frame, seconds);
        if self.demo {
            let t = self.dashboard.theme;
            self.frame.rect(12, 1, 29, 7, t.background);
            self.frame.value(13, 2, "DEMO", t.network_activity);
        }
        self.frame.quantize_565();
        true
    }
}
#[no_mangle]
pub extern "C" fn msutop_renderer_create() -> *mut c_void {
    Box::into_raw(Box::new(Engine::new(false, true, false))).cast()
}
#[no_mangle]
pub unsafe extern "C" fn msutop_renderer_destroy(renderer: *mut c_void) {
    if !renderer.is_null() {
        drop(Box::from_raw(renderer.cast::<Engine>()));
    }
}
#[no_mangle]
pub unsafe extern "C" fn msutop_renderer_options(
    renderer: *mut c_void,
    vivid: c_int,
    antialias: c_int,
    demo: c_int,
) {
    let Some(engine) = renderer.cast::<Engine>().as_mut() else {
        return;
    };
    let (vivid, antialias, demo) = (vivid != 0, antialias != 0, demo != 0);
    if engine.demo != demo {
        let volumes = std::mem::take(&mut engine.panels.volumes);
        *engine = Engine::new(vivid, antialias, demo);
        engine.panels.volumes = volumes;
    }
    engine.panels.demo = demo;
    engine.vivid = vivid;
    engine.antialias = antialias;
    engine.dashboard.theme = Theme::new(if vivid {
        ThemeKind::Vivid
    } else {
        ThemeKind::Default
    });
    engine.dashboard.antialias = antialias;
}
#[no_mangle]
pub unsafe extern "C" fn msutop_renderer_observe(
    renderer: *mut c_void,
    sample: *const MacSample,
    seconds: f64,
    disk_used: u64,
    disk_total: u64,
    processes: *const MacProcess,
    count: c_int,
) {
    let (Some(engine), Some(s)) = (renderer.cast::<Engine>().as_mut(), sample.as_ref()) else {
        return;
    };
    if !seconds.is_finite() || seconds < 0.0 {
        return;
    }
    let name = CStr::from_ptr(s.interfaces.as_ptr())
        .to_string_lossy()
        .into_owned();
    let online = !name.is_empty() && s.network_available != 0;
    let cached = s
        .memory_cached
        .min(s.memory_total.saturating_sub(s.memory_used));
    let rows = if !processes.is_null() && count > 0 {
        std::slice::from_raw_parts(processes, (count as usize).min(64))
    } else {
        &[]
    };
    let snapshot = Snapshot {
        seconds,
        uptime: s.uptime.max(0.0) as u64,
        cpu: Cpu {
            usage: (s.cpu_user + s.cpu_system).clamp(0.0, 100.0) as f32,
            user: s.cpu_user as f32,
            system: s.cpu_system as f32,
            iowait: 0.0,
            cores: s.cores[..s.core_count.clamp(0, 128) as usize]
                .iter()
                .map(|v| Core {
                    usage: v.clamp(0.0, 100.0) as f32,
                    temperature: None,
                })
                .collect(),
            temperature: (s.cpu_sensor_flags & 2 != 0).then_some(s.cpu_temperature_c as f32),
            frequency_mhz: if s.cpu_sensor_flags & 4 != 0 {
                s.cpu_frequency_mhz as f32
            } else {
                f32::NAN
            },
            load: [0.0; 3],
            power_watts: (s.cpu_sensor_flags & 1 != 0).then_some(s.cpu_power_watts as f32),
        },
        memory: Memory {
            total: s.memory_total,
            used: s.memory_used,
            cached,
            free: s
                .memory_total
                .saturating_sub(s.memory_used)
                .saturating_sub(cached),
            available: s.memory_total.saturating_sub(s.memory_used),
            swap_used: s.swap_used,
            swap_total: s.swap_total,
        },
        disk: Disk {
            name: "macOS volumes (aggregate I/O)".into(),
            used: disk_used,
            total: disk_total,
            read_bps: s.disk_read,
            write_bps: s.disk_write,
            activity: 0.0,
            temperature: None,
            queue_depth: None,
        },
        network: Network {
            interface: name,
            ipv4: None,
            state: if !online {
                NetworkState::Down
            } else if s.download > 0.0 || s.upload > 0.0 {
                NetworkState::Transferring
            } else {
                NetworkState::Active
            },
            link_mbps: (s.link_mbps > 0).then_some(s.link_mbps),
            download_bps: s.download,
            upload_bps: s.upload,
            downloaded: 0,
            uploaded: 0,
        },
        battery: None,
        processes: rows
            .iter()
            .map(|p| Process {
                id: p.identity,
                name: CStr::from_ptr(p.name.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
                cpu: p.cpu.max(0.0) as f32,
                rss: p.memory,
                threads: 0,
            })
            .collect(),
        process_count: rows.len(),
    };
    engine.observe(snapshot);
    if let Some(shown) = &engine.shown {
        engine.panels.observe(
            shown,
            &engine.dashboard,
            s.gpu_usage as f32,
            s.gpu_available != 0,
        );
    }
}
#[no_mangle]
pub unsafe extern "C" fn msutop_renderer_render(
    renderer: *mut c_void,
    seconds: f64,
    rgba: *mut u8,
    capacity: c_int,
) -> c_int {
    let Some(engine) = renderer.cast::<Engine>().as_mut() else {
        return 0;
    };
    if rgba.is_null() || capacity < 160 * 80 * 4 || !engine.render(seconds) {
        return 0;
    }
    let output = std::slice::from_raw_parts_mut(rgba, 160 * 80 * 4);
    for (pixel, out) in engine.frame.pixels.iter().zip(output.chunks_exact_mut(4)) {
        out.copy_from_slice(&[pixel.0, pixel.1, pixel.2, 255]);
    }
    output.len() as c_int
}
#[no_mangle]
pub unsafe extern "C" fn msutop_renderer_volumes(
    renderer: *mut c_void,
    volumes: *const MacVolume,
    count: c_int,
) {
    let Some(engine) = renderer.cast::<Engine>().as_mut() else {
        return;
    };
    engine.panels.volumes = if !volumes.is_null() && count > 0 {
        std::slice::from_raw_parts(volumes, count.min(4) as usize).to_vec()
    } else {
        Vec::new()
    };
}
#[no_mangle]
pub unsafe extern "C" fn msutop_renderer_render_panels(
    renderer: *mut c_void,
    seconds: f64,
    left: *mut u8,
    right: *mut u8,
    capacity: c_int,
) -> c_int {
    let Some(engine) = renderer.cast::<Engine>().as_mut() else {
        return 0;
    };
    if left.is_null() || right.is_null() || capacity < 160 * 80 * 4 {
        return 0;
    }
    let Some(shown) = &engine.shown else { return 0 };
    engine.dashboard.render_mac_panels(
        shown,
        &engine.panels,
        &mut engine.left,
        &mut engine.right,
        seconds,
    );
    engine.left.quantize_565();
    engine.right.quantize_565();
    let out_left = std::slice::from_raw_parts_mut(left, 160 * 80 * 4);
    for (pixel, rgba) in engine.left.pixels.iter().zip(out_left.chunks_exact_mut(4)) {
        rgba.copy_from_slice(&[pixel.0, pixel.1, pixel.2, 255]);
    }
    let out_right = std::slice::from_raw_parts_mut(right, 96 * 80 * 4);
    for y in 0..80 {
        for x in 0..96 {
            let pixel = engine.right.pixels[y * 160 + x];
            out_right[(y * 96 + x) * 4..(y * 96 + x + 1) * 4]
                .copy_from_slice(&[pixel.0, pixel.1, pixel.2, 255]);
        }
    }
    160 * 80 * 4
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_and_extended_left_match_upstream_at_60hz() {
        let mut engine = Engine::new(false, true, false);
        let mut reference = msutop::Dashboard::with_sample_rate(Theme::new(ThemeKind::Default), 30);
        let mut expected = Frame::default();
        let mut last_tick = None;
        for frame in 0..600 {
            let time = 70.0 + frame as f64 / 60.0;
            let tick = frame / 2;
            if last_tick != Some(tick) {
                let snapshot = MockSource::snapshot(Scene::Balanced, time);
                engine.observe(snapshot);
                reference.observe(engine.shown.as_ref().unwrap());
                last_tick = Some(tick);
            }
            reference.render_at(engine.shown.as_ref().unwrap(), &mut expected, time);
            expected.quantize_565();
            assert!(engine.render(time));
            assert_eq!(engine.frame.pixels, expected.pixels, "frame {frame}");
            // Enabling the optional extension must not alter the original panel.
            engine.dashboard.render_mac_panels(
                engine.shown.as_ref().unwrap(),
                &engine.panels,
                &mut engine.left,
                &mut engine.right,
                time,
            );
            engine.left.quantize_565();
            assert_eq!(
                engine.left.pixels, expected.pixels,
                "extended left frame {frame}"
            );
        }
    }
    #[test]
    fn animation_changes_between_samples_and_static_ui_stays_crisp() {
        let mut engine = Engine::new(false, true, true);
        for tick in 0..150 {
            engine.observe(MockSource::snapshot(
                Scene::Balanced,
                70.0 + tick as f64 / 30.0,
            ));
        }
        let time = 70.0 + 149.0 / 30.0;
        engine.render(time);
        let before = engine.frame.clone();
        engine.render(time + 1.0 / 60.0);
        let changed = before
            .pixels
            .iter()
            .zip(&engine.frame.pixels)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > 10,
            "subframe animation must continue without a telemetry sample"
        );
        assert_eq!(
            before.pixels[79 * 160..],
            engine.frame.pixels[79 * 160..],
            "bottom border must remain fixed"
        );
    }
    #[test]
    fn both_native_canvases_animate_without_another_sample() {
        let mut engine = Engine::new(false, true, true);
        for tick in 0..150 {
            let time = 70.0 + tick as f64 / 30.0;
            engine.observe(MockSource::snapshot(Scene::Balanced, time));
            engine.panels.observe(
                engine.shown.as_ref().unwrap(),
                &engine.dashboard,
                15.0 + tick as f32 * 0.1,
                true,
            );
        }
        let time = 70.0 + 149.0 / 30.0;
        let shown = engine.shown.as_ref().unwrap();
        engine.dashboard.render_mac_panels(
            shown,
            &engine.panels,
            &mut engine.left,
            &mut engine.right,
            time,
        );
        let before_left = engine.left.clone();
        let before_right = engine.right.clone();
        engine.dashboard.render_mac_panels(
            shown,
            &engine.panels,
            &mut engine.left,
            &mut engine.right,
            time + 1.0 / 60.0,
        );
        assert_eq!(engine.left.pixels.len(), 160 * 80);
        assert_eq!(engine.right.pixels.len(), 160 * 80);
        assert_ne!(
            before_left.pixels, engine.left.pixels,
            "left canvas must scroll at presentation rate"
        );
        assert_ne!(
            before_right.pixels, engine.right.pixels,
            "right canvas must scroll at presentation rate"
        );
        assert_eq!(
            before_right.pixels[79 * 160..],
            engine.right.pixels[79 * 160..],
            "borders must remain stationary"
        );
    }
    #[test]
    fn asymmetric_ffi_output_preserves_buffer_boundaries() {
        let mut engine = Box::new(Engine::new(false, true, true));
        engine.observe(MockSource::snapshot(Scene::Balanced, 70.0));
        let mut left = vec![0xa5; 160 * 80 * 4 + 16];
        let mut right = vec![0xa5; 96 * 80 * 4 + 16];
        let size = unsafe {
            msutop_renderer_render_panels(
                (&mut *engine as *mut Engine).cast(),
                70.0,
                left.as_mut_ptr(),
                right.as_mut_ptr(),
                160 * 80 * 4,
            )
        };
        assert_eq!(size, 160 * 80 * 4);
        assert!(left[160 * 80 * 4..].iter().all(|b| *b == 0xa5));
        assert!(right[96 * 80 * 4..].iter().all(|b| *b == 0xa5));
        assert!(right[..96 * 80 * 4].chunks_exact(4).all(|p| p[3] == 255));
    }
}
