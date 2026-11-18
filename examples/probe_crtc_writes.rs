//! `probe_crtc_writes <script.csl> [frames]`: plays the script, then lists every write to the CRTC
//! (ports &BCxx select, &BDxx data) of the next frames: the register, the value, the CRTC position
//! (C4/C9/C0) and the microsecond of the instruction's start. To find out what a test program does
//! to the CRTC (Shaker's tests, a demo's raster trick).
use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16, Reg8};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("script");
    let frames: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut runner = CslRunner::new("/tmp/probe_crtc_writes_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut frame = 0;
    let mut was_vsync = mb.crtc().vsync();
    let mut selected = 0u8;
    while frame < frames {
        let pc = mb.pc();
        let (op, op2) = (mb.read_byte(pc), mb.read_byte(pc.wrapping_add(1)));
        let cpu = mb.cpu();
        let b = cpu.reg8(Reg8::B).value();
        let a = cpu.reg8(Reg8::A).value();
        let hl = cpu.reg16(Reg16::Hl).value();
        let write = match (op, op2) {
            (0xd3, _) => Some((a, a)),
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
                Some((b, value))
            }
            (0xed, 0xa3 | 0xab | 0xb3 | 0xbb) => Some((b.wrapping_sub(1), mb.read_byte(hl))),
            _ => None,
        };
        let c = mb.crtc().counters();
        let t = mb.t_state_counter().value();
        if let Some((port_high, value)) = write {
            // A9=A8=0 -> select (&BC), A8=1 -> data (&BD); A14 low selects the CRTC
            if port_high & 0x40 == 0 && port_high & 0x80 != 0 {
                if port_high & 0x03 == 0 {
                    selected = value & 31;
                } else if port_high & 0x03 == 1 {
                    println!(
                        "frame {frame} C4={:3} C9={:2} C0={:3} us%64={:2} pc={pc:04X} R{selected} <- {value}",
                        c.vcc, c.vlc, c.hcc, (t / 4) % 64
                    );
                }
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
