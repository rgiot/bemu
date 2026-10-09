//! After a CSL script: wait for the next interrupt (PC reaching #0038) and
//! print the next instructions with their duration in T-states (CPC
//! rounding), PC and bytes. `probe_int_handler <script.csl> [count] [skip_interrupts]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let count: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(40);
    let skip: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut runner = CslRunner::new("/tmp/probe_int_handler_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut seen = 0;
    let mut guard = 0u64;
    loop {
        let before = mb.pc();
        mb.execute_next_opcode_no_video();
        guard += 1;
        assert!(guard < 50_000_000, "no interrupt");
        if mb.pc() == 0x38 && before != 0x38 {
            seen += 1;
            if seen > skip { break; }
        }
    }
    println!("interrupt accepted, PC=0038");
    for _ in 0..count {
        let pc = mb.pc();
        let b: Vec<String> = (0..4).map(|i| format!("{:02X}", mb.read_byte(pc.wrapping_add(i)))).collect();
        let t0 = mb.t_state_counter().value();
        let c = mb.crtc().hcc;
        mb.execute_next_opcode_no_video();
        let dt = mb.t_state_counter().value() - t0;
        println!("{pc:04X}: {:<12} {dt:3}T  (C0={c})", b.join(" "));
    }
}
