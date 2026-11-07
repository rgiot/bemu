//! After a CSL script, run until the PC hits an address `n` times, printing
//! the registers each time. `probe_bp <script.csl> <pc_hex> <hits> [max_instr]`
use amsrust::csl_runner::CslRunner;
use amsrust::z80::HasValue;
use amsrust::z80::Reg8;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let pc = u16::from_str_radix(&a.next().expect("pc"), 16).unwrap();
    let hits: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let max: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(400_000_000);
    let dump_from = a.next().map(|s| u16::from_str_radix(&s, 16).unwrap());
    let dump_len: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(256);
    let mut runner = CslRunner::new("/tmp/probe_bp_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut n = 0;
    let mut count = 0u64;
    while n < hits && count < max {
        mb.execute_next_opcode_no_video();
        count += 1;
        if mb.pc() == pc {
            let cpu = mb.cpu();
            println!(
                "hit {n}: t={} HL={:04X} DE={:04X} BC={:02X}{:02X} A={:02X}",
                mb.t_state_counter().value(),
                (cpu.reg8(Reg8::H).value() as u16) << 8 | cpu.reg8(Reg8::L).value() as u16,
                (cpu.reg8(Reg8::D).value() as u16) << 8 | cpu.reg8(Reg8::E).value() as u16,
                cpu.reg8(Reg8::B).value(),
                cpu.reg8(Reg8::C).value(),
                cpu.reg8(Reg8::A).value()
            );
            {
                let cpu = mb.cpu();
                println!(
                    "  cpu SP={:04X} IX={:04X} IY={:04X} F={:02X} I={:02X} R={:02X} IFF1={}",
                    cpu.reg16(amsrust::z80::Reg16::Sp).value(),
                    cpu.reg16(amsrust::z80::Reg16::Ix).value(),
                    cpu.reg16(amsrust::z80::Reg16::Iy).value(),
                    cpu.reg8(Reg8::F).value(),
                    cpu.reg8(Reg8::I).value(),
                    cpu.reg8(Reg8::R).value(),
                    cpu.iff1()
                );
            }
            {
                let c = mb.crtc();
                println!("  crtc regs R0-R9=[{},{},{},{},{},{},{},{},{},{}] selected={} C0={} C4={:?} C9={} vsync={}", c.r0(),c.r1(),c.r2(),c.r3(),c.r4(),c.r5(),c.r6(),c.r7(),c.r8(),c.r9(), c.selected_register(), c.hcc, c.vcc, c.vlc.value(), c.vsync() as u8);
            }
            println!(
                "  psg r7={:02X} r14={:02X} sel={:?}",
                mb.psg().register(7),
                mb.psg().register(14),
                mb.psg().selected_register()
            );
            n += 1;
            if let (Some(from), 1) = (dump_from, n) {
                for row in 0..(dump_len / 16) {
                    let base = from.wrapping_add((row * 16) as u16);
                    let bytes: Vec<String> = (0..16)
                        .map(|i| format!("{:02X}", mb.read_byte(base.wrapping_add(i))))
                        .collect();
                    println!("{base:04X}: {}", bytes.join(" "));
                }
            }
        }
    }
    println!("{count} instructions");
}
