//! `probe_vc <script.csl> [lines]`: after the script, prints C4/C9 and the VSYNC pin at every line end
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let script = a.next().expect("script.csl");
    let lines: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(400);
    let mut runner = CslRunner::new("/tmp/probe_vc_shots");
    let base = std::path::Path::new(&script).parent().unwrap().to_path_buf();
    runner.run_text(&std::fs::read_to_string(&script).unwrap(), base).unwrap();
    let mb = runner.motherboard_mut();
    let mut last = mb.crtc().line_ends;
    let mut n = 0;
    while n < lines {
        mb.tick();
        let c = mb.crtc();
        if c.line_ends != last {
            last = c.line_ends;
            println!("C4={} C9={} R4={} R7={} R9={} R5={} vs={}", c.vcc.value(), c.c9(), c.regs_r(4), c.regs_r(7), c.regs_r(9), c.regs_r(5), c.vsync());
            n += 1;
        }
    }
}
