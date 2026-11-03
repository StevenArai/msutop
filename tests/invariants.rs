use msutop::{
    frame::{FONT, FONT_3X5, PIXELS},
    metrics::Process,
    selection::LazyTop,
};
use msutop::{
    Dashboard, Frame, MockSource, NetworkState, Rgb, Scene, Theme, ThemeKind, RGB565_BYTES,
};

fn process(id: u64, cpu: f32) -> Process {
    Process {
        id,
        name: format!("p{id}"),
        cpu,
        rss: 1024,
        threads: 1,
    }
}

#[test]
fn font_cells_are_six_by_eight_and_msb_first() {
    let source = include_str!("../assets/font6x8.h");
    assert!(source.contains("SPDX-License-Identifier: BSD-2-Clause"));
    assert_eq!(FONT.len(), 95 * 8);
    assert!(FONT.iter().all(|row| row & 3 == 0));
    let mut frame = Frame::default();
    let color = Rgb(255, 255, 255);
    frame.text(0, 0, "A", color);
    // A's first row is 0x20, therefore x=2; preserve MSB-first orientation.
    assert_eq!(frame.pixels[2], color);
    assert_eq!(frame.pixels[1], Rgb::default());
    assert_eq!(frame.pixels[3], Rgb::default());
    assert!(frame.pixels[7 * 160..8 * 160]
        .iter()
        .all(|&p| p == Rgb::default()));
    frame.text(10, 10, "08", color);
    assert_eq!(Frame::text_width("08"), 12);
    assert_eq!(frame.pixels[10 * 160 + 12], color);
    assert_eq!(frame.pixels[16 * 160 + 12], color);
    assert_eq!(frame.pixels[10 * 160 + 18], color);
    assert_eq!(frame.pixels[16 * 160 + 18], color);
}

#[test]
fn credits_font_keeps_user_glyphs_and_row_major_orientation() {
    assert_eq!(FONT_3X5.len(), 95);
    for (ch, glyph) in [
        ('0', 0x3b6e),
        ('1', 0x2c97),
        ('F', 0x79e4),
        ('E', 0x79e7),
        ('G', 0x796f),
        ('B', 0x6baf),
    ] {
        assert_eq!(FONT_3X5[ch as usize - 32], glyph);
        let mut frame = Frame::default();
        let color = Rgb(255, 255, 255);
        frame.value(10, 10, &ch.to_string(), color);
        for y in 0..5 {
            for x in 0..3 {
                assert_eq!(
                    frame.pixels[(10 + y) * 160 + 10 + x] == color,
                    glyph & (1 << (14 - y * 3 - x)) != 0,
                    "{ch} must retain the supplied pixels"
                );
            }
        }
        assert_eq!(Frame::value_width(&ch.to_string()), 3);
    }
    assert_eq!(Frame::value_width("FE"), 7);
    assert_eq!(Frame::value_width("GBE"), 11);
}

#[test]
fn rgb565_has_the_future_transport_size_order_and_endianness() {
    let mut frame = Frame::default();
    frame.pixels[0] = Rgb(255, 0, 0);
    frame.pixels[1] = Rgb(0, 255, 0);
    frame.pixels[2] = Rgb(0, 0, 255);
    frame.pixels[160] = Rgb(255, 255, 255);
    let mut output = vec![0; RGB565_BYTES];
    frame.rgb565_be(&mut output);
    assert_eq!(output.len(), 25_600);
    assert_eq!(&output[..6], &[0xf8, 0x00, 0x07, 0xe0, 0x00, 0x1f]);
    assert_eq!(&output[320..322], &[0xff, 0xff]);
    let original = output.clone();
    frame.quantize_565();
    frame.rgb565_be(&mut output);
    assert_eq!(
        original, output,
        "preview quantization matches transmitted pixels"
    );
}

#[test]
fn lazy_names_do_not_reorder_for_jitter_or_swapped_ranks() {
    let mut top = LazyTop::<2>::default();
    top.update(0.0, &[process(1, 20.0), process(2, 19.8), process(3, 19.6)]);
    assert_eq!(top.ids(), [Some(1), Some(2)]);
    for time in 1..30 {
        top.update(
            time as f64,
            &[process(1, 19.7), process(2, 20.1), process(3, 20.2)],
        );
        assert_eq!(
            top.ids(),
            [Some(1), Some(2)],
            "small CPU fluctuations must not jump names"
        );
    }
}

#[test]
fn lazy_names_require_a_sustained_challenger_but_replace_exited_processes() {
    let mut top = LazyTop::<2>::default();
    top.update(0.0, &[process(1, 20.0), process(2, 10.0), process(3, 1.0)]);
    top.update(
        1.0,
        &[process(1, 20.0), process(2, 10.0), process(3, 100.0)],
    );
    assert_eq!(
        top.ids(),
        [Some(1), Some(2)],
        "one spike does not replace a process"
    );
    for time in 2..6 {
        top.update(
            time as f64,
            &[process(1, 20.0), process(2, 10.0), process(3, 100.0)],
        );
        assert_eq!(top.ids(), [Some(1), Some(2)], "minimum hold is six seconds");
    }
    for time in 6..9 {
        top.update(
            time as f64,
            &[process(1, 20.0), process(2, 10.0), process(3, 100.0)],
        );
    }
    assert!(
        top.ids().contains(&Some(3)),
        "sustained CPU winner must become visible"
    );
    top.update(9.0, &[process(1, 20.0), process(2, 10.0)]);
    assert!(
        !top.ids().contains(&Some(3)),
        "an exited process is removed immediately"
    );
}

#[test]
fn new_cpu_winner_replaces_the_weaker_name_without_moving_the_stronger_one() {
    let mut top = LazyTop::<2>::default();
    top.update(0.0, &[process(1, 50.0), process(2, 10.0), process(3, 1.0)]);
    for time in 1..=20 {
        top.update(
            time as f64,
            &[process(1, 50.0), process(2, 10.0), process(3, 80.0)],
        );
        assert_eq!(
            top.ids()[0],
            Some(1),
            "the stronger existing name must keep its slot"
        );
    }
    assert_eq!(top.ids(), [Some(1), Some(3)]);
}

#[test]
fn single_layout_keeps_resource_icons_for_every_mock_scene() {
    for kind in [ThemeKind::Default, ThemeKind::Vivid] {
        for scene in Scene::ALL {
            let theme = Theme::new(kind);
            let mut dashboard = Dashboard::new(theme);
            for tick in 0..128 {
                dashboard.observe(&MockSource::snapshot(scene, 68.0 + tick as f64 / 30.0));
            }
            let snapshot = MockSource::snapshot(scene, 72.0);
            let mut frame = Frame::default();
            dashboard.render(&snapshot, &mut frame);
            assert_eq!(frame.pixels.len(), PIXELS);
            for (x, y, width, height, name) in [
                (2, 24, 9, 5, "memory"),
                (2, 40, 9, 5, "swap"),
                (99, 24, 9, 7, "Ethernet"),
            ] {
                assert!(
                    (y..y + height).any(|row| {
                        frame.pixels[row * 160 + x..row * 160 + x + width]
                            .iter()
                            .any(|&color| color != theme.background)
                    }),
                    "{scene:?} {name} must remain on the same screen"
                );
            }
            assert_eq!(dashboard.selected_ids().iter().flatten().count(), 2);
            let mut raw = vec![0; RGB565_BYTES];
            frame.rgb565_be(&mut raw);
            assert!(raw.iter().any(|&b| b != 0));
        }
    }
}

#[test]
fn network_link_type_is_independent_of_throughput_and_totals_are_not_displayed() {
    let mut snapshot = MockSource::snapshot(Scene::Balanced, 72.0);
    let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
    dashboard.observe(&snapshot);
    assert_eq!(snapshot.network.link_label(), "GBE");
    let mut frame = Frame::default();
    dashboard.render(&snapshot, &mut frame);
    snapshot.network.downloaded = u64::MAX;
    snapshot.network.uploaded = u64::MAX;
    let mut changed = Frame::default();
    dashboard.render(&snapshot, &mut changed);
    assert_eq!(
        frame.pixels, changed.pixels,
        "no cumulative traffic on screen"
    );
    snapshot.network.download_bps = 0.0;
    snapshot.network.upload_bps = 0.0;
    assert_eq!(snapshot.network.link_label(), "GBE");
    snapshot.network.link_mbps = Some(100);
    assert_eq!(snapshot.network.link_label(), "FE");
    dashboard.render(&snapshot, &mut changed);
    assert_ne!(
        &frame.pixels[25 * 160 + 110..25 * 160 + 121],
        &changed.pixels[25 * 160 + 110..25 * 160 + 121],
        "the negotiated link rate must update the header"
    );
    snapshot.network.state = NetworkState::Down;
    assert_eq!(snapshot.network.link_label(), "--");
    snapshot.network.state = NetworkState::Active;
    snapshot.network.link_mbps = None;
    assert_eq!(snapshot.network.link_label(), "--");
}

#[test]
fn network_lights_follow_link_and_packet_state_and_stop_blinking_when_idle() {
    let theme = Theme::new(ThemeKind::Default);
    let mut dashboard = Dashboard::new(theme);
    let mut snapshot = MockSource::snapshot(Scene::Balanced, 0.0);
    let mut frame = Frame::default();
    for (seconds, state, link_lit, traffic_lit) in [
        (0.0, NetworkState::Transferring, true, true),
        (0.13, NetworkState::Transferring, true, false),
        (0.23, NetworkState::Transferring, true, true),
        (0.33, NetworkState::Active, true, false),
        (0.46, NetworkState::Active, true, true),
        (0.51, NetworkState::Active, true, false),
        (0.56, NetworkState::NoCarrier, false, false),
        (0.66, NetworkState::Down, false, false),
        (0.76, NetworkState::Transferring, true, true),
    ] {
        snapshot.seconds = seconds;
        snapshot.network.state = state;
        dashboard.observe(&snapshot);
        dashboard.render(&snapshot, &mut frame);
        let green = if link_lit {
            theme.free.at(0.70)
        } else {
            theme.muted
        };
        let orange = if traffic_lit {
            theme.network_activity
        } else {
            theme.muted
        };
        for y in 29..31 {
            assert_eq!(
                frame.pixels[y * 160 + 99],
                green,
                "{state:?} connection light"
            );
            assert_eq!(
                frame.pixels[y * 160 + 107],
                orange,
                "{state:?} activity light"
            );
        }
        for x in [99, 107] {
            for y in 28..32 {
                for column in x - 1..=x + 1 {
                    if column != x || y == 28 || y == 31 {
                        assert_eq!(
                            frame.pixels[y * 160 + column],
                            Rgb(0, 0, 0),
                            "black lamp outline"
                        );
                    }
                }
            }
        }
        let socket_color = if state == NetworkState::Down {
            theme.muted
        } else {
            theme.title
        };
        let mut socket = Frame::default();
        msutop::icons::Icon::Ethernet.draw(&mut socket, 99, 24, socket_color);
        for y in 24..31 {
            for x in 99..108 {
                let covered_by_lamp = y >= 28 && (x <= 100 || x >= 106);
                if socket.pixels[y * 160 + x] == socket_color && !covered_by_lamp {
                    assert_eq!(
                        frame.pixels[y * 160 + x],
                        socket_color,
                        "original white outline remains outside the corner lamp masks"
                    );
                }
            }
        }
        let mut repeated = Frame::default();
        dashboard.render(&snapshot, &mut repeated);
        assert_eq!(
            frame.pixels, repeated.pixels,
            "paused rendering cannot animate the light"
        );
    }
}

#[test]
fn intermittent_packets_keep_the_activity_phase_until_a_real_idle_gap() {
    let theme = Theme::new(ThemeKind::Default);
    let mut dashboard = Dashboard::new(theme);
    let mut snapshot = MockSource::snapshot(Scene::Balanced, 0.0);
    let mut frame = Frame::default();
    for tick in 0..=30 {
        snapshot.seconds = tick as f64 / 30.0;
        snapshot.network.state = if tick % 4 == 0 {
            NetworkState::Transferring
        } else {
            NetworkState::Active
        };
        dashboard.observe(&snapshot);
        dashboard.render(&snapshot, &mut frame);
        let lit = (snapshot.seconds + 1e-9).rem_euclid(0.2) < 0.1;
        assert_eq!(
            frame.pixels[29 * 160 + 107],
            if lit {
                theme.network_activity
            } else {
                theme.muted
            },
            "empty 33ms samples must not restart blink phase: tick={tick}"
        );
    }
    snapshot.seconds = 1.3;
    snapshot.network.state = NetworkState::Active;
    dashboard.observe(&snapshot);
    dashboard.render(&snapshot, &mut frame);
    assert_eq!(frame.pixels[29 * 160 + 107], theme.muted);
    snapshot.seconds = 1.34;
    snapshot.network.state = NetworkState::Transferring;
    dashboard.observe(&snapshot);
    dashboard.render(&snapshot, &mut frame);
    assert_eq!(frame.pixels[29 * 160 + 107], theme.network_activity);
    snapshot.seconds = 1.38;
    snapshot.network.state = NetworkState::NoCarrier;
    dashboard.observe(&snapshot);
    dashboard.render(&snapshot, &mut frame);
    assert_eq!(frame.pixels[29 * 160 + 107], theme.muted);
}

#[test]
fn all_network_states_keep_colored_status_words_inside_the_header() {
    let theme = Theme::new(ThemeKind::Default);
    for state in NetworkState::ALL {
        let snapshot = MockSource::snapshot_with_network(Scene::Balanced, 1.0, Some(state));
        let mut dashboard = Dashboard::new(theme);
        dashboard.observe(&snapshot);
        let mut frame = Frame::default();
        dashboard.render(&snapshot, &mut frame);
        let color = match state {
            NetworkState::Down => theme.muted,
            NetworkState::NoCarrier => theme.network_activity,
            NetworkState::Active | NetworkState::Transferring => theme.free.at(0.70),
        };
        let mut expected = Frame::default();
        expected.value_right(157, 25, state.status_label(), color);
        assert!(state
            .status_label()
            .chars()
            .all(|ch| ch.is_ascii_uppercase()));
        let status_left = 157 - Frame::value_width(state.status_label()) as usize;
        for y in 25..30 {
            assert_eq!(
                &frame.pixels[y * 160 + status_left..y * 160 + 159],
                &expected.pixels[y * 160 + status_left..y * 160 + 159]
            );
        }
        assert_eq!(snapshot.network.download_bps > 0.0, state.has_traffic());
        assert_eq!(snapshot.network.upload_bps > 0.0, state.has_traffic());
    }
}

#[test]
fn memory_pressure_counts_reclaimable_cache_and_checks_swap_separately() {
    let mut memory = MockSource::snapshot(Scene::Balanced, 0.0).memory;
    memory.total = 1000;
    memory.free = 0;
    memory.cached = 100;
    memory.available = 0;
    memory.swap_total = 1000;
    memory.swap_used = 900;
    assert!(
        !memory.available_is_low(),
        "cache is available and exactly ten percent is sufficient"
    );
    assert!(!memory.swap_is_low());
    memory.cached = 99;
    assert!(memory.available_is_low());
    assert!(
        !memory.swap_is_low(),
        "memory shortage must not recolor healthy swap"
    );
    memory.free = 1;
    memory.swap_used = 901;
    assert!(!memory.available_is_low());
    assert!(memory.swap_is_low());
    memory.total = 0;
    memory.swap_total = 0;
    assert!(
        !memory.available_is_low(),
        "missing total is not a shortage"
    );
    assert!(!memory.swap_is_low(), "disabled swap is not a shortage");
}

#[test]
fn extra_render_calls_cannot_advance_a_thirty_hz_history() {
    let theme = Theme::new(ThemeKind::Default);
    let mut slow = Dashboard::new(theme);
    let mut fast = Dashboard::new(theme);
    for tick in 0..=600 {
        slow.observe(&MockSource::snapshot(Scene::Balanced, tick as f64 / 30.0));
    }
    for tick in 0..=1200 {
        fast.observe(&MockSource::snapshot(Scene::Balanced, tick as f64 / 60.0));
    }
    let snapshot = MockSource::snapshot(Scene::Balanced, 20.0);
    let mut a = Frame::default();
    let mut b = Frame::default();
    slow.render(&snapshot, &mut a);
    fast.render(&snapshot, &mut b);
    assert_eq!(
        a.pixels, b.pixels,
        "extra rendering calls within a 30 Hz tick must not consume history"
    );
}

#[test]
fn charts_and_numeric_fields_change_inside_a_single_second() {
    let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
    let mut previous = None::<Frame>;
    for tick in 0..30 {
        let mut snapshot = MockSource::snapshot(Scene::Balanced, tick as f64 / 30.0);
        snapshot.cpu.usage = 10.0 + tick as f32 * 2.0;
        snapshot.cpu.system = 0.0;
        snapshot.cpu.iowait = 0.0;
        dashboard.observe(&snapshot);
        let mut frame = Frame::default();
        dashboard.render(&snapshot, &mut frame);
        if let Some(before) = &previous {
            let region = |f: &Frame, x: usize, y: usize, width: usize, height: usize| {
                (y..y + height)
                    .flat_map(|row| {
                        f.pixels[row * 160 + x..row * 160 + x + width]
                            .iter()
                            .copied()
                    })
                    .collect::<Vec<_>>()
            };
            assert_ne!(
                region(before, 124, 5, 17, 5),
                region(&frame, 124, 5, 17, 5),
                "CPU number must use every fresh snapshot"
            );
            assert_ne!(
                region(before, 2, 2, 86, 19),
                region(&frame, 2, 2, 86, 19),
                "CPU history must enter every 30 Hz tick"
            );
        }
        previous = Some(frame);
    }
}

#[test]
fn thirty_fps_preserves_process_name_confirmation_times() {
    let theme = Theme::new(ThemeKind::Default);
    let mut slow = Dashboard::with_sample_rate(theme, 1);
    let mut fast = Dashboard::with_sample_rate(theme, 30);
    for tick in 0..=450 {
        let t = tick as f64 / 30.0;
        let mut snapshot = MockSource::snapshot(Scene::Balanced, t);
        snapshot.processes = vec![
            process(1, 50.0),
            process(2, 10.0),
            process(3, if t < 1.0 { 1.0 } else { 80.0 }),
        ];
        fast.observe(&snapshot);
        if tick % 30 == 0 {
            slow.observe(&snapshot);
            assert_eq!(
                fast.selected_ids(),
                slow.selected_ids(),
                "30 fps must not turn three confirmations into 100 ms"
            );
        }
    }
    assert!(
        fast.selected_ids().contains(&Some(3)),
        "a sustained winner still becomes visible"
    );
}

#[test]
fn antialias_switch_preserves_font_padding_and_unaffected_panels() {
    let mut changed = [0; 7];
    for scene in [Scene::Idle, Scene::Balanced, Scene::Cpu, Scene::Io] {
        let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
        for tick in 0..128 {
            dashboard.observe(&MockSource::snapshot(scene, 66.0 + tick as f64 / 30.0));
        }
        let snapshot = MockSource::snapshot(scene, 70.0);
        let mut aa = Frame::default();
        let mut pixel = Frame::default();
        dashboard.render(&snapshot, &mut aa);
        dashboard.antialias = false;
        dashboard.render(&snapshot, &mut pixel);
        for (offset, (a, b)) in aa.pixels.iter().zip(&pixel.pixels).enumerate() {
            let x = offset % 160;
            let y = offset / 160;
            if (2..88).contains(&x) && (2..21).contains(&y) {
                changed[0] += usize::from(a != b);
            } else if (92..123).contains(&x) && (4..21).contains(&y) {
                changed[1] += usize::from(a != b);
            } else if (22..35).contains(&x) && ((24..37).contains(&y) || (40..53).contains(&y)) {
                changed[2] += usize::from(a != b);
            } else if (48..73).contains(&x) && ((38..44).contains(&y) || (46..52).contains(&y)) {
                changed[3] += usize::from(a != b);
            } else if (99..135).contains(&x) && ((33..42).contains(&y) || (43..52).contains(&y)) {
                changed[4] += usize::from(a != b);
            } else if (113..131).contains(&x) && ((56..65).contains(&y) || (67..76).contains(&y)) {
                changed[5] += usize::from(a != b);
            } else if (92..95).contains(&x) && (26..52).contains(&y) {
                changed[6] += usize::from(a != b);
            } else {
                assert_eq!(
                    a, b,
                    "AA must preserve font, padding, borders and temperature indicators"
                );
            }
        }
    }
    assert!(
        changed.iter().all(|&n| n > 0),
        "AA must affect CPU, disk, network and process curves, both columns, and rings; aligned horizontal strokes may remain crisp"
    );
}

#[test]
fn every_panel_keeps_one_empty_pixel_between_content_and_border() {
    for scene in Scene::ALL {
        let mut dashboard = Dashboard::new(Theme::new(ThemeKind::Default));
        let snapshot = MockSource::snapshot(scene, 70.0);
        dashboard.observe(&snapshot);
        for aa in [true, false] {
            dashboard.antialias = aa;
            let mut frame = Frame::default();
            dashboard.render(&snapshot, &mut frame);
            let background = dashboard.theme.background;
            for x in 1..159 {
                assert_eq!(frame.pixels[160 + x], background, "top padding");
                assert_eq!(frame.pixels[21 * 160 + x], background, "CPU bottom padding");
                if x != 38 && x != 96 {
                    assert_eq!(
                        frame.pixels[23 * 160 + x],
                        background,
                        "resource top padding"
                    );
                    assert_eq!(
                        frame.pixels[53 * 160 + x],
                        background,
                        "resource bottom padding"
                    );
                }
                assert_eq!(
                    frame.pixels[55 * 160 + x],
                    background,
                    "process top padding"
                );
                assert_eq!(
                    frame.pixels[78 * 160 + x],
                    background,
                    "process bottom padding"
                );
                assert_eq!(
                    frame.pixels[79 * 160 + x],
                    dashboard.theme.proc_box,
                    "process panel closes with the same border color"
                );
            }
            for y in 24..53 {
                for x in [1, 37, 39, 95, 97, 158] {
                    assert_eq!(
                        frame.pixels[y * 160 + x],
                        background,
                        "resource side padding"
                    );
                }
            }
            for y in 37..40 {
                assert!(
                    frame.pixels[y * 160 + 1..y * 160 + 38]
                        .iter()
                        .all(|&color| color == background),
                    "memory and swap keep three blank rows between them"
                );
            }
            for y in 65..67 {
                assert!(
                    frame.pixels[y * 160 + 1..y * 160 + 159]
                        .iter()
                        .all(|&color| color == background),
                    "expanded process graphs keep two blank rows between them"
                );
            }
            for y in 57..76 {
                for x in [3, 82, 83, 84, 85, 110, 111, 112, 131, 132, 157, 158] {
                    assert_eq!(
                        frame.pixels[y * 160 + x],
                        background,
                        "process fields keep clear column gaps"
                    );
                }
            }
            for y in 2..21 {
                for x in [1, 89, 91, 158] {
                    assert_eq!(frame.pixels[y * 160 + x], background, "CPU side padding");
                }
            }
        }
    }
}
