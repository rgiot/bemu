use amsrust::chip::*;
/// This test aims at checking the behavior of GA/monitor for a standard screen
use amsrust::crtc::*;
use amsrust::ga::*;
use amsrust::monitor::*;

fn beginning_steps(ga: &mut GateArray, crtc: &mut Crtc) {
    ga.save_pins_state();
    crtc.save_pins_state();
    sync_pins(ga, &[GateArrayPin::CCLK], crtc, &[CrtcPin::CCLK]);
    sync_pins(
        crtc,
        &[CrtcPin::VS, CrtcPin::HS, CrtcPin::DE],
        ga,
        &[
            GateArrayPin::VSYNC,
            GateArrayPin::HSYNC,
            GateArrayPin::DISPEN,
        ],
    );

    assert_eq!(ga.get_pin(GateArrayPin::CCLK), crtc.get_pin(CrtcPin::CCLK),);
}

/// What a tick's output must be, once it has gone through the Gate Array's output pipeline.
#[derive(Clone, Copy)]
enum Expected {
    /// The first ticks of a delayed mode: nothing to check
    Unknown,
    Black,
    /// The border ink, whatever the palette is when it comes out (the ink is looked up then)
    Border,
    /// One of the four inks of mode 1
    Mode1
}

thread_local! {
    /// The expectations of the last [`PALETTE_LOOKUP_LAG_TICKS`] ticks: the colour of a pixel comes
    /// out that many ticks after the pen was known.
    static PIPELINE: std::cell::RefCell<std::collections::VecDeque<Expected>> =
        std::cell::RefCell::new(std::collections::VecDeque::new());

    /// Was the previous pixel a border one? The pixels of the modes other than
    /// MODE 2 come out one tick late (MODE 2 is displayed a pixel earlier).
    static PREVIOUS_BORDER: std::cell::Cell<Option<(bool, [Ink; 17])>> = std::cell::Cell::new(None);

    /// The black of the VSYNC (ACCC 16.2.3): from the CRTC's VSYNC start until 26 HSYNCs have ended,
    /// whatever the VSYNC length. (previous VSYNC pin, previous HSYNC, HSYNC ends left)
    static VSYNC_BLACK: std::cell::Cell<(bool, bool, u8)> = std::cell::Cell::new((false, false, 0));
}

fn end_steps(ga: &mut GateArray, crtc: &mut Crtc, monitor: &mut Monitor) {
    let is_border_now = !ga.get_pin(GateArrayPin::DISPEN);
    let previous = PREVIOUS_BORDER.with(|p| p.replace(Some((is_border_now, *ga.palette()))));
    // nothing to check on the very first tick of a delayed mode
    let unknown = ga.current_mode() != VideoMode::Mode2 && previous.is_none();
    let (is_border, palette) = if ga.current_mode() == VideoMode::Mode2 {
        (is_border_now, *ga.palette())
    } else {
        previous.unwrap_or((true, *ga.palette()))
    };
    let palette_before_tick = *ga.palette();
    // ga cycles
    ga.tick();
    if crtc.is_activated() {
        crtc.tick();
    }

    let rgb = ga.rgb();
    palette[0].to_rgb_levels();

    let vsync = VSYNC_BLACK.with(|state| {
        let (vsync_before, hsync_before, mut left) = state.get();
        let vsync_pin = ga.get_pin(GateArrayPin::VSYNC);
        let hsync_now = ga.hsync_ga();
        if vsync_pin && !vsync_before {
            left = 26;
        }
        if hsync_before && !hsync_now && left > 0 {
            left -= 1;
        }
        state.set((vsync_pin, hsync_now, left));
        left > 0
    });
    // the display is blanked by the HSYNC as the GA sees it (a few pixels
    // after the CRTC raises/lowers it)
    let hsync = ga.hsync_ga();

    let expected_now = if !vsync && !hsync && !unknown {
        if is_border {
            Expected::Border
        } else {
            Expected::Mode1
        }
    }
    else if unknown {
        Expected::Unknown
    }
    else {
        Expected::Black
    };
    let expected = PIPELINE.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.push_back(expected_now);
        if queue.len() > PALETTE_LOOKUP_LAG_TICKS {
            queue.pop_front()
        }
        else {
            None
        }
    });
    match expected {
        None | Some(Expected::Unknown) => {},
        Some(Expected::Black) => assert_eq!(RgbLevels::from((0., 0., 0.)), rgb),
        // the palette of the tick, or of the one before (the chip may see it a tick late)
        Some(Expected::Border) => assert!(
            [*ga.palette(), palette_before_tick].iter().any(|p| p[16].to_rgb_levels() == rgb),
            "{rgb:?} is not the border ink"
        ),
        Some(Expected::Mode1) => assert!(
            [*ga.palette(), palette_before_tick]
                .iter()
                .any(|p| (0..4).any(|pen| p[pen].to_rgb_levels() == rgb)),
            "{rgb:?} is none of the palette's"
        )
    }

    // let rgb : RgbLevels= palette[1].to_rgb_levels();
    monitor.receive(rgb, Sync { vsync, hsync });

    ga.manage_clocks();
}

fn fetch_video_byte(bank: &[u8; 0x4000], ga: &GateArray, crtc: &Crtc) -> u8 {
    let crtc_address = crtc.get_address_from_pins();
    let fetch_address = ga.video_fetch_address(crtc_address);
    assert!(fetch_address >= 0xc000);

    bank[fetch_address as usize - 0xc000]
}

fn generate_screen(bank: &[u8; 0x4000], fname: &str, palette: [Ink; 17], mode: VideoMode) {
    // Configure the crtc just before raising the vsync
    let mut crtc = Crtc::default();
    crtc.do_write_register(12, 0x30);
    crtc.do_write_register(13, 0x00);
    crtc.vlc = 7.into();
    crtc.vcc = 29.into();
    crtc.hcc = 63;
    crtc.seed_vma_for_test(0x3000, 29);
    crtc.set_address_on_pins();

    // Configure the monitor in a wait state (not a big deal, it happens one char too soon)
    let mut monitor = Monitor::debug();

    let mut ga = GateArray::default();
    ga.set_palette(palette);
    ga.force_active_video_mode(mode);
    ga.set_video_mode(mode);
    ga.set_pin(GateArrayPin::CCLK, false);
    ga.seed_clock_for_test();

    /*
    // let one second  pass to properly synchronize screen
    for f in 0..50 {
        for char_line in 0..=crtc.get_r4() {
            for scan_line in 0..=crtc.get_r9() {
                for char in 0..=crtc.get_r0() {
                    for pixel in 0..16 {
                        beginning_steps(&mut ga, &mut crtc);

                        if  ga.need_memory_byte(){
                            ga.set_memory_byte(0);
                        }

                        end_steps(&mut ga, &mut crtc, &mut monitor);

                    }
                }
            }
        }
    }
    */

    // and make the real tests

    assert_eq!(ga.get_pin(GateArrayPin::CCLK), false);
    let crtc_address = crtc.get_address_from_pins();
    println!("0x{:x}", crtc_address);

    for _char_line in 0..=crtc.r4() {
        for _scan_line in 0..=crtc.r9() {
            for _char in 0..=crtc.r0() {
                for _pixel in 0..16 {
                    beginning_steps(&mut ga, &mut crtc);

                    // TODO reimplement this stuff in a more
                    // realistic way
                    if ga.need_memory_byte() {
                        // Feed GA with the byte currently selected by CRTC+CCLK phase.
                        let byte = fetch_video_byte(bank, &ga, &crtc);
                        ga.set_memory_byte(byte); // TODO do it with its pins
                    }

                    end_steps(&mut ga, &mut crtc, &mut monitor);
                }
            }
        }
    }

    monitor.image().save(fname).unwrap();
}

#[test]
fn test_crtc_catart() {
    let bank = include_bytes!("screen.scr");
    let fname = "test_standard_screen_pic.png";
    let palette: [Ink; 17] = [
        0.into(),
        8.into(),
        13.into(),
        18.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        10.into(),
    ];
    let mode = VideoMode::Mode1;
    generate_screen(&bank, fname, palette, mode);
}

#[test]
fn test_raster() {
    let bank = include_bytes!("screen.scr");
    let fname = "test_raster.png";
    let mut palette: [Ink; 17] = [
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        0.into(),
        10.into(),
    ];
    let mode = VideoMode::Mode1;

    // Configure the crtc just before raising the vsync
    let mut crtc = Crtc::default();
    crtc.do_write_register(12, 0x30);
    crtc.do_write_register(13, 0x00);
    crtc.do_write_register(1, 0);

    crtc.vlc = 7.into();
    crtc.vcc = 29.into();
    crtc.hcc = 63;
    crtc.seed_vma_for_test(0x3000, 29);
    crtc.set_address_on_pins();

    // Configure the monitor in a wait state (not a big deal, it happens one char too soon)
    let mut monitor = Monitor::debug();

    let mut ga = GateArray::default();
    ga.set_palette(palette);
    ga.force_active_video_mode(mode);
    ga.set_video_mode(mode);
    ga.set_pin(GateArrayPin::CCLK, false);
    ga.seed_clock_for_test();

    /*
    // let one second  pass to properly synchronize screen
    for f in 0..50 {
        for char_line in 0..=crtc.get_r4() {
            for scan_line in 0..=crtc.get_r9() {
                for char in 0..=crtc.get_r0() {
                    for pixel in 0..16 {
                        beginning_steps(&mut ga, &mut crtc);

                        if  ga.need_memory_byte(){
                            ga.set_memory_byte(0);
                        }

                        end_steps(&mut ga, &mut crtc, &mut monitor);

                    }
                }
            }
        }
    }
    */

    // and make the real tests

    assert_eq!(ga.get_pin(GateArrayPin::CCLK), false);
    let crtc_address = crtc.get_address_from_pins();
    println!("0x{:x}", crtc_address);

    for char_line in 0..=crtc.r4() {
        for scan_line in 0..=crtc.r9() {
            for _char in 0..=crtc.r0() {
                for _pixel in 0..16 {
                    beginning_steps(&mut ga, &mut crtc);

                    // TODO reimplement this stuff in a more
                    // realistic way
                    if ga.need_memory_byte() {
                        // Feed GA with the byte currently selected by CRTC+CCLK phase.
                        let byte = fetch_video_byte(bank, &ga, &crtc);
                        ga.set_memory_byte(byte); // TODO do it with its pins
                    }

                    end_steps(&mut ga, &mut crtc, &mut monitor);
                }
                if char_line == 20 {
                    let colors = [15, 16, 17, 18, 19, 20, 21, 26];
                    palette[16] = colors[scan_line as usize].into();
                    ga.set_palette(palette);
                }
            }
        }
    }

    monitor.image().save(fname).unwrap();
}
