//! After a CSL script, follow the CPC+ ASIC's 16 hardware sprites through whole frames: every
//! change of a sprite's position, magnification or pixels, with the CRTC line (C4/C9) it happens
//! on. A sprite moved during the frame, below where it was shown, is multiplexed: one hardware
//! sprite drawn several times per frame (rings, enemies).
//! `probe_sprite_mux <script.csl> [frames] [quiet]`: `quiet` prints only the changes made while
//! the picture is drawn (line < 272) and a line per frame that has some.
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let frames: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let quiet = a.next().as_deref() == Some("quiet");
    let mut runner = CslRunner::new("/tmp/probe_sprite_mux_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    // start on a VSYNC
    let in_vsync = |mb: &amsrust::motherboard::Motherboard| mb.crtc().vsync();
    while in_vsync(mb) {
        mb.execute_next_opcode_no_video();
    }
    while !in_vsync(mb) {
        mb.execute_next_opcode_no_video();
    }
    let mut previous = mb.asic_snapshot_chunk().expect("a CPC+");
    for sprite in 0..16 {
        let attr = 0x800 + sprite * 8;
        println!(
            "sprite {sprite:2}: x={:4} y={:4} mag={:02X}",
            i16::from_le_bytes([previous[attr], previous[attr + 1]]),
            i16::from_le_bytes([previous[attr + 2], previous[attr + 3]]),
            previous[attr + 4]
        );
    }
    let mut visible_changes = 0;
    let mut frame = 0;
    let mut was_vsync = true;
    let mut moves = vec![0usize; 16];
    let mut pixel_writes = vec![0usize; 16];
    let line = |mb: &amsrust::motherboard::Motherboard| {
        let c = mb.crtc().counters();
        // a CRTC row is 8 lines on this game (R9=7)
        (c.vcc as usize) * 8 + c.vlc as usize
    };
    println!("frame 0");
    while frame < frames {
        mb.execute_next_opcode_no_video();
        let vsync = in_vsync(mb);
        if vsync && !was_vsync {
            frame += 1;
            if !quiet {
                println!(
                    "  moved: {:?}\n  pixels rewritten (bytes): {:?}",
                    moves, pixel_writes
                );
            } else if visible_changes > 0 {
                println!(
                    "frame {}: {visible_changes} changes while the picture is drawn",
                    frame - 1
                );
            }
            visible_changes = 0;
            moves.fill(0);
            pixel_writes.fill(0);
            if frame < frames && !quiet {
                println!("frame {frame}");
            }
        }
        was_vsync = vsync;
        let now = mb.asic_snapshot_chunk().unwrap();
        if now == previous {
            continue;
        }
        for sprite in 0..16 {
            let attr = 0x800 + sprite * 8;
            if now[attr..attr + 5] != previous[attr..attr + 5] {
                if line(mb) < 272 {
                    visible_changes += 1;
                }
                let x = i16::from_le_bytes([now[attr], now[attr + 1]]);
                let y = i16::from_le_bytes([now[attr + 2], now[attr + 3]]);
                if !quiet || line(mb) < 272 {
                    println!(
                        "  line {:3} sprite {sprite:2}: x={x:4} y={y:4} mag={:02X}",
                        line(mb),
                        now[attr + 4]
                    );
                }
                moves[sprite] += 1;
            }
            let pixels = sprite * 128..sprite * 128 + 128;
            pixel_writes[sprite] += now[pixels.clone()]
                .iter()
                .zip(&previous[pixels])
                .filter(|(a, b)| a != b)
                .count();
        }
        previous = now;
    }
}
