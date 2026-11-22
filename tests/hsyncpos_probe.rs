//! HSYNCPOS (tools/hsyncpos of CLAUDESTRAD, tests/fixtures/claudestrad/hsyncpos/HSYNCPOS.BIN): the
//! HSYNC of every line is moved by the program (R2 and R3 rewritten line by line, 11 bands of 16
//! lines) and what is drawn shows how the monitor's line oscillator follows it. Run on a bare machine
//! with a CRTC 1, the monitor fed directly, in seconds. The target is the real CRTC 1 and CTM 644:
//! the left edge of the picture on each line, read off the photo
//! (tests/fixtures/claudestrad/truth/HSYNCPOS_crtc1_claudestrad_profile.txt, in ticks).

use amsrust::crtc::CrtcType;
use amsrust::monitor::Monitor;
use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};

/// The left edge of the picture (first bright pixel) on each line, in ticks from the first lines'.
fn profile() -> Vec<Option<f64>> {
    let bin = include_bytes!("fixtures/claudestrad/hsyncpos/HSYNCPOS.BIN");
    let code = &bin[128..128 + 0x375];
    let mut builder = MotherboardBuilder::default();
    builder.set_crtc(CrtcType::Crtc1);
    let mut mb = builder.disable_lower_rom().disable_upper_rom().enable_interruptions().build();
    mb.write_buffer(0x8000, code);
    mb.set_pc(0x8000);
    mb.cpu_mut().reg16_mut(Reg16::Sp).set(0xbf00);
    mb.set_video(true);
    let mut monitor = Monitor::default();
    let mut last = None;
    let start = mb.t_state_counter().value();
    while mb.t_state_counter().value() - start < 4_000_000 * 3 {
        mb.execute_next_opcode_with_video(|(rgb, sync)| monitor.receive(rgb, sync));
        if let Some(frame) = monitor.take_frame() {
            last = Some(frame);
        }
    }
    let frame = last.expect("a frame");
    let edges: Vec<Option<f64>> = (0..frame.height())
        .map(|y| {
            (0..frame.width())
                .find(|&x| {
                    let p = frame.get_pixel(x, y);
                    (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3 > 128
                })
                .map(|x| x as f64)
        })
        .collect();
    let base = {
        let mut first: Vec<f64> = edges[2..20].iter().flatten().copied().collect();
        first.sort_by(|a, b| a.partial_cmp(b).unwrap());
        first[first.len() / 2]
    };
    edges.iter().map(|e| e.map(|x| x - base)).collect()
}

/// The real profile of the photo (None where the photo has no edge).
fn real_profile() -> Vec<Option<f64>> {
    include_str!("fixtures/claudestrad/truth/HSYNCPOS_crtc1_claudestrad_profile.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.trim().parse::<f64>().ok())
        .collect()
}

/// Root mean square difference over the lines 4 to 264 that both have.
fn rms(a: &[Option<f64>], b: &[Option<f64>]) -> f64 {
    let (mut sum, mut n) = (0.0, 0.0);
    for y in 4..265 {
        if let (Some(Some(x)), Some(Some(y))) = (a.get(y), b.get(y)) {
            sum += (x - y) * (x - y);
            n += 1.0;
        }
    }
    (sum / n).sqrt()
}

#[test]
#[ignore = "prints the profiles: cargo test --release --test hsyncpos_probe -- --ignored --nocapture"]
fn print_the_profiles() {
    let (ours, real) = (profile(), real_profile());
    println!("line  bemu  real");
    for y in (0..272).step_by(1) {
        let f = |v: &Option<f64>| v.map_or("  .".to_owned(), |v| format!("{v:3.0}"));
        println!("{y:3}  {}  {}", f(&ours[y]), f(&real[y]));
    }
    println!("rms {:.2} ticks", rms(&ours, &real));
}
