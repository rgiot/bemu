//! After a CSL script (typically a `snapshot_load`): for the lines starting `<from_us>..<to_us>`
//! microseconds after it, the time, C4/C9 and the MA of the line's first character, to compare
//! with CPCSyntaxError's `CPCSE_TRACE_LINE` (`us=`, `ma=`).
//! `probe_line_ma <script.csl> <from_us> <to_us>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let from: u64 = a.next().unwrap().parse().unwrap();
    let to: u64 = a.next().unwrap().parse().unwrap();
    let mut runner = CslRunner::new("/tmp/probe_line_ma_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut ticks = 0u64;
    let mut last_c0 = 255u8;
    while ticks / 16 < to {
        mb.tick();
        ticks += 1;
        let c = mb.crtc();
        if c.hcc != last_c0 && c.hcc == 0 && ticks / 16 >= from {
            println!(
                "us={} C4={} C9={} ma={:04x}",
                ticks / 16,
                c.vcc.value(),
                c.vlc.value(),
                c.vma()
            );
        }
        last_c0 = c.hcc;
    }
}
