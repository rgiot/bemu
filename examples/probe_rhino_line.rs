//! After a CSL script (validation/demos/rhino/rhino.csl): trace what happens on one scanline of the
//! picture: every change of the GA palette (tick in the line, pen, ink) and the runs of pixel colours
//! of the visible part. `probe_rhino_line <script.csl> <picture line> [<from_x> <to_x>]`
//! (picture line as in the 768x272 screenshots; x in screenshot pixels).
use amsrust::csl_runner::CslRunner;
use amsrust::monitor::Monitor;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let line: usize = a.next().expect("picture line").parse().unwrap();
    let from: usize = a.next().map_or(0, |v| v.parse().unwrap());
    let to: usize = a.next().map_or(768, |v| v.parse().unwrap());
    let mut runner = CslRunner::new("/tmp/probe_rhino_line_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut monitor = Monitor::default();
    // two whole frames to settle, then trace the next
    let mut frames = 0;
    let mut last_palette = *mb.ga().palette();
    let mut events = Vec::new();
    let mut runs: Vec<(usize, [u8; 3])> = Vec::new();
    let mut instruction_pc = mb.pc();
    let mut ticks_in = 0u32;
    while frames < 3 {
        if mb.is_aligned_with_new_instruction() {
            instruction_pc = mb.pc();
            ticks_in = 0;
        }
        ticks_in += 1;
        let (rgb, sync) = mb.tick();
        monitor.receive(rgb, sync);
        if frames == 2 {
            let (x, beam_line) = monitor.beam();
            let (crop_x, crop_y) = monitor.crop_origin();
            let y = beam_line as isize - crop_y as isize;
            if y == line as isize {
                let palette = *mb.ga().palette();
                for pen in 0..17 {
                    if palette[pen] != last_palette[pen] {
                        events.push((x as isize - crop_x, pen, format!("{:?} by the instruction at {:04X}, tick {} of it", palette[pen], instruction_pc, ticks_in)));
                    }
                }
                let px = x as isize - crop_x;
                let color = [(rgb.r() * 2.55) as u8, (rgb.g() * 2.55) as u8, (rgb.b() * 2.55) as u8];
                if px >= from as isize && (px as usize) < to {
                    if runs.last().map_or(true, |r| r.1 != color) {
                        runs.push((px as usize, color));
                    }
                }
            }
            last_palette = *mb.ga().palette();
        }
        if monitor.take_frame().is_some() {
            frames += 1;
        }
        if frames < 2 {
            last_palette = *mb.ga().palette();
        }
    }
    println!("palette changes on picture line {line} (x in picture pixels; 16 per usec):");
    for (x, pen, ink) in &events {
        println!("  x={x:5} pen {pen:2} -> {ink}");
    }
    println!("colour runs from x={from}:");
    let mut pieces = Vec::new();
    for (i, (x, c)) in runs.iter().enumerate() {
        let end = runs.get(i + 1).map_or(to, |r| r.0);
        pieces.push(format!("{x}..{end}:{c:?}"));
    }
    println!("  {}", pieces.join("  "));
}
