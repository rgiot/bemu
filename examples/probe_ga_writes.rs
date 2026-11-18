//! `probe_ga_writes <script.csl> [frames]`: plays the script, then lists every write to the Gate
//! Array (port &7Fxx) of the next frames: the instruction (`OUT (C),r`, `OUT (n),A`, `OUTI`...),
//! what it writes (pen select, ink, mode/ROM, RAM), the CRTC position (C4/C9, C0) and the
//! microsecond (and T-state) of the instruction's start. To find out how a palette effect
//! is made.
use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16, Reg8};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("script");
    let frames: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut runner = CslRunner::new("/tmp/probe_ga_writes_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut frame = 0;
    let mut was_vsync = mb.crtc().vsync();
    while frame < frames {
        let pc = mb.pc();
        let (op, op2, op3) = (
            mb.read_byte(pc),
            mb.read_byte(pc.wrapping_add(1)),
            mb.read_byte(pc.wrapping_add(2)),
        );
        let cpu = mb.cpu();
        let b = cpu.reg8(Reg8::B).value();
        let a = cpu.reg8(Reg8::A).value();
        let hl = cpu.reg16(Reg16::Hl).value();
        // the instruction, the port's high byte and the value written
        let write = match (op, op2) {
            (0xd3, _) => Some(("OUT (n),A", a, a)),
            (0xed, 0x41 | 0x49 | 0x51 | 0x59 | 0x61 | 0x69 | 0x71 | 0x79) => {
                let reg = [
                    Reg8::B,
                    Reg8::C,
                    Reg8::D,
                    Reg8::E,
                    Reg8::H,
                    Reg8::L,
                    Reg8::B,
                    Reg8::A,
                ][((op2 >> 3) & 7) as usize];
                let value = if op2 == 0x71 {
                    0
                } else {
                    cpu.reg8(reg).value()
                };
                Some(("OUT (C),r", b, value))
            }
            (0xed, 0xa3 | 0xab | 0xb3 | 0xbb) => {
                // OUTI and friends decrement B before putting it on the bus
                Some(("OUTI/OUTD", b.wrapping_sub(1), mb.read_byte(hl)))
            }
            _ => None,
        };
        let c = mb.crtc().counters();
        // T-states since the start (4 a microsecond)
        let t = mb.t_state_counter().value();
        let _ = op3;
        if let Some((name, port_high, value)) = write {
            if port_high & 0xc0 == 0x40 {
                let what = match value >> 6 {
                    0 if value & 0x10 != 0 => "select border".to_string(),
                    0 => format!("select pen {}", value & 15),
                    1 => format!("ink {}", value & 31),
                    2 => format!("mode/rom {:02X}", value),
                    _ => format!("ram {:02X}", value),
                };
                println!(
                    "frame {frame} C4={:2} C9={} C0={:2} us%64={:2} T%4={} pc={pc:04X} {name:10} {what}",
                    c.vcc, c.vlc, c.hcc, (t / 4) % 64, t % 4
                );
            }
        }
        mb.execute_next_opcode_no_video();
        let vsync = mb.crtc().vsync();
        if vsync && !was_vsync {
            frame += 1;
        }
        was_vsync = vsync;
    }
}
