//! After a CSL script: for every CRTC line of the next frame (counted from
//! the VSYNC start) print how many times the output colour changes in a tick
//! window, and how many of those changes follow a run of 1-2 ticks only (a
//! sign of a 1 or 2-tick pixel mode).
//! `probe_line_stats <script.csl> <from_tick> <to_tick> [<lines>]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let from: u32 = a.next().unwrap().parse().unwrap();
    let to: u32 = a.next().unwrap().parse().unwrap();
    let lines: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(312);
    let mut runner = CslRunner::new("/tmp/probe_line_stats_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut prev_vs = mb.crtc().vsync();
    loop {
        mb.tick();
        let vs = mb.crtc().vsync();
        if vs && !prev_vs { break; }
        prev_vs = vs;
    }
    let mut n = 0usize;
    let mut last_hcc = 255u8;
    let mut tick = 0u32;
    let mut cols: Vec<(u8, u8, u8)> = Vec::new();
    while n < lines {
        let (rgb, _) = mb.tick();
        tick += 1;
        if tick >= from && tick < to {
            cols.push((rgb.r() as u8, rgb.g() as u8, rgb.b() as u8));
        }
        let hcc = mb.crtc().hcc;
        if hcc == 0 && last_hcc != 0 {
            let mut changes = 0;
            let mut fast = 0;
            let mut run = 1;
            for i in 1..cols.len() {
                if cols[i] != cols[i - 1] {
                    changes += 1;
                    if run <= 2 { fast += 1; }
                    run = 1;
                } else { run += 1; }
            }
            let mut d = cols.clone(); d.sort(); d.dedup();
            println!("line {n:3} changes={changes:3} fast={fast:3} distinct={}", d.len());
            cols.clear();
            n += 1;
            tick = 0;
        }
        last_hcc = hcc;
    }
}
