//! After a CSL script, run until the PC reaches an address, then print the
//! next N executed instructions as `PC: bytes`.
//! `probe_trace <script.csl> <pc_hex> <n> [skip]`
use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16, Reg8};

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let pc = u16::from_str_radix(&a.next().expect("pc"), 16).unwrap();
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    let skip: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut runner = CslRunner::new("/tmp/probe_trace_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut guard = 0u64;
    while mb.pc() != pc {
        mb.execute_next_opcode_no_video();
        guard += 1;
        assert!(guard < 400_000_000, "never reached the pc");
    }
    for i in 0..(n + skip) {
        let p = mb.pc();
        let bytes: Vec<String> = (0..4)
            .map(|k| format!("{:02X}", mb.read_byte(p.wrapping_add(k))))
            .collect();
        if i >= skip {
            let c = mb.cpu();
            println!("{p:04X}: {} | A={:02X} F={:02X} B={:02X} C={:02X} D={:02X} E={:02X} H={:02X} L={:02X} SP={:04X}", bytes.join(" "),
                c.reg8(Reg8::A).value(), c.reg8(Reg8::F).value(), c.reg8(Reg8::B).value(), c.reg8(Reg8::C).value(),
                c.reg8(Reg8::D).value(), c.reg8(Reg8::E).value(), c.reg8(Reg8::H).value(), c.reg8(Reg8::L).value(), c.reg16(Reg16::Sp).value());
        }
        mb.execute_next_opcode_no_video();
    }
}
