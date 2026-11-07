//! Run a CSL script, then print where the machine is: PC samples, CRTC
//! registers, GA modes, interrupt state. `probe_state <script.csl> [samples]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let samples: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut runner = CslRunner::new("/tmp/probe_state_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    for _ in 0..samples {
        for _ in 0..20000 {
            mb.execute_next_opcode_no_video();
        }
        let c = mb.crtc();
        println!(
            "PC={:04X} R0-R9={:?} mode next={:?} active={:?}",
            mb.pc(),
            [
                c.r0(),
                c.r1(),
                c.r2(),
                c.r3(),
                c.r4(),
                c.r5(),
                c.r6(),
                c.r7(),
                c.r8(),
                c.r9()
            ],
            mb.ga().next_video_mode(),
            mb.ga().active_video_mode()
        );
    }
}
