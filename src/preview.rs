use crate::{prepared, Options};
use msutop::{NetworkState, Scene, TelemetrySource, ThemeKind, HEIGHT, WIDTH};
use sdl2::{event::Event, keyboard::Keycode, pixels::PixelFormatEnum, rect::Rect};
use std::time::{Duration, Instant};

pub fn run(options: &Options) -> Result<(), String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    sdl2::hint::set("SDL_RENDER_SCALE_QUALITY", "0");
    let mut settings = options.clone();
    let window = video
        .window(
            "msutop / 160 x 80 / 6 x 8",
            (WIDTH * settings.scale) as u32,
            (HEIGHT * settings.scale) as u32,
        )
        .position_centered()
        .resizable()
        .build()
        .map_err(|e| e.to_string())?;
    let mut canvas = match window
        .clone()
        .into_canvas()
        .accelerated()
        .present_vsync()
        .build()
    {
        Ok(canvas) => canvas,
        Err(error) => {
            eprintln!("SDL vsync unavailable ({error}); using a paced software preview");
            window
                .into_canvas()
                .software()
                .build()
                .map_err(|e| e.to_string())?
        }
    };
    let renderer = canvas.info();
    let vsync = renderer.flags & 0x4 != 0;
    let refresh_hz = canvas
        .window()
        .display_index()
        .ok()
        .and_then(|display| video.desktop_display_mode(display).ok())
        .map_or(60, |mode| mode.refresh_rate.max(1)) as u32;
    eprintln!(
        "SDL renderer: {}, vsync={}, screen={} Hz, telemetry={} Hz",
        renderer.name, vsync, refresh_hz, settings.fps
    );
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(PixelFormatEnum::RGB24, WIDTH as u32, HEIGHT as u32)
        .map_err(|e| e.to_string())?;
    let mut events = sdl.event_pump()?;
    let (mut source, mut dashboard, mut frame) = prepared(&settings).map_err(|e| e.to_string())?;
    let mut scratch = vec![0; WIDTH * HEIGHT * 3];
    let started = Instant::now();
    let mut mock_time = if settings.live { 0.0 } else { settings.time };
    let mut clock_updated = started;
    let mut paused = false;
    let period = Duration::from_secs_f64(1.0 / refresh_hz.max(settings.fps) as f64);
    let mut frames = 0u64;
    let mut samples = 0u64;
    let mut sample_tick = None;
    let mut shown = None;
    let mut title = String::new();
    let mut presentation_times = Vec::new();
    let mut sample_times = Vec::new();
    let mut smoother = msutop::smoothing::Smoother::default();
    'running: loop {
        let now = Instant::now();
        if !paused {
            mock_time += now.duration_since(clock_updated).as_secs_f64();
        }
        clock_updated = now;
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                Event::KeyDown {
                    keycode: Some(key),
                    repeat: false,
                    ..
                } => {
                    let scene = match key {
                        Keycode::Num1 => Some(Scene::Balanced),
                        Keycode::Num2 => Some(Scene::Idle),
                        Keycode::Num3 => Some(Scene::Cpu),
                        Keycode::Num4 => Some(Scene::Io),
                        Keycode::Num5 => Some(Scene::Memory),
                        Keycode::Num6 => Some(Scene::Offline),
                        _ => None,
                    };
                    if let Some(scene) = scene.filter(|_| !settings.live) {
                        settings.scene = scene;
                        settings.time = mock_time;
                        (source, dashboard, frame) =
                            prepared(&settings).map_err(|e| e.to_string())?;
                        shown = None;
                        sample_tick = None;
                    }
                    match key {
                        Keycode::Space => paused = !paused,
                        Keycode::N if !settings.live => {
                            settings.network_state =
                                Some(match source.sample(mock_time).network.state {
                                    NetworkState::Down => NetworkState::NoCarrier,
                                    NetworkState::NoCarrier => NetworkState::Active,
                                    NetworkState::Active => NetworkState::Transferring,
                                    NetworkState::Transferring => NetworkState::Down,
                                });
                            settings.time = mock_time;
                            (source, dashboard, frame) =
                                prepared(&settings).map_err(|e| e.to_string())?;
                            shown = None;
                            sample_tick = None;
                        }
                        Keycode::T => {
                            settings.theme = match settings.theme {
                                ThemeKind::Default => ThemeKind::Vivid,
                                ThemeKind::Vivid => ThemeKind::Default,
                            };
                            dashboard.theme = msutop::Theme::new(settings.theme);
                        }
                        Keycode::Q => settings.quantize = !settings.quantize,
                        Keycode::A => {
                            settings.antialias = !settings.antialias;
                            dashboard.antialias = settings.antialias;
                        }
                        Keycode::Equals | Keycode::Plus | Keycode::KpPlus => {
                            settings.scale = (settings.scale + 1).min(16);
                            canvas
                                .window_mut()
                                .set_size(
                                    (WIDTH * settings.scale) as u32,
                                    (HEIGHT * settings.scale) as u32,
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        Keycode::Minus | Keycode::KpMinus => {
                            settings.scale = settings.scale.saturating_sub(1).max(1);
                            canvas
                                .window_mut()
                                .set_size(
                                    (WIDTH * settings.scale) as u32,
                                    (HEIGHT * settings.scale) as u32,
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        _ => (),
                    }
                }
                _ => (),
            }
        }
        let tick = (mock_time * settings.fps as f64 + 1e-7).floor() as u64;
        if shown.is_none() || (!paused && sample_tick != Some(tick)) {
            let snapshot = source.sample(mock_time);
            let mut values = if settings.live && settings.smoothing {
                smoother.apply(&snapshot, dashboard.selected_ids())
            } else {
                snapshot.clone()
            };
            dashboard.observe_ranked(&values, source.ranking(&snapshot));
            source.watch_processes(dashboard.selected_ids());
            source.complete_display(&mut values, dashboard.selected_ids());
            shown = Some(values);
            sample_tick = Some(tick);
            samples += 1;
            if settings.profile.is_some() {
                sample_times.push(started.elapsed().as_secs_f64());
            }
        }
        dashboard.render_at(
            shown.as_ref().expect("initial sample"),
            &mut frame,
            mock_time,
        );
        if settings.quantize {
            frame.quantize_565();
        }
        frame.rgb24(&mut scratch);
        texture
            .update(None, &scratch, WIDTH * 3)
            .map_err(|e| e.to_string())?;
        canvas.set_draw_color(sdl2::pixels::Color::RGB(0, 0, 0));
        canvas.clear();
        let (w, h) = canvas.output_size()?;
        let scale = (w / WIDTH as u32).min(h / HEIGHT as u32).max(1);
        let width = WIDTH as u32 * scale;
        let height = HEIGHT as u32 * scale;
        let destination = Rect::new(
            (w as i32 - width as i32) / 2,
            (h as i32 - height as i32) / 2,
            width,
            height,
        );
        canvas.copy(&texture, None, destination)?;
        canvas.present();
        if settings.profile.is_some() {
            presentation_times.push(started.elapsed().as_secs_f64());
        }
        let new_title = format!(
            "msutop | 160x80 | {} Hz data | {} {} Hz | {} | {} | {} | {}{}",
            settings.fps,
            if vsync { "vsync" } else { "preview" },
            refresh_hz,
            if settings.live {
                "linux live"
            } else {
                settings.scene.name()
            },
            settings.theme.name(),
            if settings.antialias { "AA" } else { "pixel" },
            if settings.quantize {
                "RGB565"
            } else {
                "RGB888"
            },
            if paused { " | paused" } else { "" }
        );
        if title != new_title {
            canvas
                .window_mut()
                .set_title(&new_title)
                .map_err(|e| e.to_string())?;
            title = new_title;
        }
        frames += 1;
        if settings.frames.is_some_and(|count| frames >= count)
            || settings
                .seconds
                .is_some_and(|s| started.elapsed().as_secs_f64() >= s)
        {
            break;
        }
        // Vsync owns presentation timing. Never add a second, competing sleep clock.
        if !vsync {
            let deadline = started + period.mul_f64(frames as f64);
            if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    eprintln!(
        "SDL preview: {frames} presentations, {samples} samples in {elapsed:.2}s; {:.2} fps, {:.2} Hz data",
        frames as f64 / elapsed,
        samples as f64 / elapsed,
    );
    if let Some(path) = &settings.profile {
        let intervals = |times: &[f64]| {
            times
                .windows(2)
                .map(|pair| (pair[1] - pair[0]) * 1e6)
                .collect::<Vec<_>>()
        };
        let report = format!("{{\n\"renderer\":\"{}\",\"vsync\":{vsync},\"screen_hz\":{refresh_hz},\"target_data_hz\":{},\"frames\":{frames},\"samples\":{samples},\"elapsed_seconds\":{elapsed:.6},\"presentation_fps\":{:.3},\"sample_hz\":{:.3},\n\"presentation_interval_us\":{},\n\"sample_interval_us\":{}\n}}\n",
            renderer.name, settings.fps, frames as f64 / elapsed, samples as f64 / elapsed,
            crate::distribution(&intervals(&presentation_times)), crate::distribution(&intervals(&sample_times)));
        std::fs::write(path, report).map_err(|e| e.to_string())?;
    }
    Ok(())
}
