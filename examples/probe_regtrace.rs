//! After a CSL script, run to a PC, then print every CRTC register change by
//! instruction: `<n> PC=xxxx sel=<selected> [(reg, old, new)]`.
//! `probe_regtrace <script.csl> <pc_hex> <instructions>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let pc = u16::from_str_radix(&a.next().expect("pc"), 16).unwrap();
    let count: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(2000);
    let mut runner = CslRunner::new("/tmp/probe_regtrace_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut guard = 0u64;
    while mb.pc() != pc {
        mb.execute_next_opcode_no_video();
        guard += 1;
        assert!(guard < 80_000_000);
    }
    let regs = |mb: &amsrust::motherboard::Motherboard| { let c = mb.crtc(); [c.r0(), c.r1(), c.r2(), c.r3(), c.r4(), c.r5(), c.r6(), c.r7(), c.r8(), c.r9(), 0, 0, c.r12(), c.r13()] };
    let mut prev = regs(mb);
    for i in 0..count {
        mb.execute_next_opcode_no_video();
        let now = regs(mb);
        if now != prev {
            let ch: Vec<String> = (0..14).filter(|r| now[*r] != prev[*r]).map(|r| format!("({r},{},{})", prev[r], now[r])).collect();
            println!("{i} PC={:04X} sel={} {}", mb.pc(), mb.crtc().selected_register(), ch.join(" "));
            prev = now;
        }
    }
}
