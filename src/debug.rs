use cpclib_asm::preamble::Token;
use smallvec::SmallVec;

use crate::chip::Chip;
use crate::crtc::Crtc;
use crate::ga::*;
use crate::motherboard::Motherboard;
use crate::z80::Z80Flags;
use crate::z80::Z80Pin;
use crate::z80::{HasValue, Reg16, Z80};

pub struct Monitor {}

impl Monitor {
    pub fn monitor(motherboard: &Motherboard) {
        print!("z80: ");
        Monitor::monitor_z80(motherboard.cpu());
        Monitor::monitor_z80_flags(motherboard.cpu());
        Monitor::monitor_z80_pins(motherboard.cpu());

        print!("stack: ");
        Monitor::monitor_stack(motherboard, 2, 8);

        print!("ga: ");
        Monitor::monitor_ga(motherboard.ga());
        Monitor::monitor_ga_pins(motherboard.ga());

        print!("crtc: ");
        Monitor::monitor_crtc_registers(motherboard.crtc());
        Monitor::monitor_crtc_counters(motherboard.crtc());

        print!("next: ");
        Monitor::monitor_next_instruction(&motherboard)
    }

    pub fn monitor_crtc_registers(crtc: &Crtc) {
        println!(
            "R0:{} R1:{} R2:{} R3:{} R4:{} R5:{} R6:{} R7:{} R8:{} R9:{} R10:{} R11:{} R12:{} R13:{}",
            crtc.get_r0(),
            crtc.get_r1(),
            crtc.get_r2(),
            crtc.get_r3(),
            crtc.get_r4(),
            crtc.get_r5(),
            crtc.get_r6(),
            crtc.get_r7(),
            crtc.get_r8(),
            crtc.get_r9(),
            crtc.get_r10(),
            crtc.get_r11(),
            crtc.get_r12(),
            crtc.get_r13(),
        )
    }
    pub fn monitor_crtc_counters(crtc: &Crtc) {
        println!(
            "VSC:{}{} VCC:{} VLC:{} VTAC:{} | HSC:{}{} HDC:{} HCC:{} | VMA:0x{:x}", 
            if crtc.vsync(){"*"}else{""},
            crtc.vsc,
            crtc.vcc.value(),
            crtc.vlc.value(),
            crtc.vtac,

            if crtc.hsync(){"*"}else{""},
            crtc.hsc,
            crtc.hdc,
            crtc.hcc,

            crtc.vma()
        );
    }

    pub fn monitor_stack(motherboard: &Motherboard, before: u16, after: u16) {
        let sp_orig = motherboard.cpu().reg16(Reg16::Sp).value();
        let sp = sp_orig.wrapping_sub(after * 2);
        let size = after + before;

        (0..size).rev().for_each(|i| {
            let addr = sp.wrapping_add(2 * i);
            let is_top = addr == sp_orig;
            if is_top {
                print!("<<");
            }
            print!("0x{:04X}=0x{:04X}", addr, motherboard.read_word(addr));
            if is_top {
                print!(">> ");
            } else {
                print!(" ");
            }
        });
        println!();
    }

    pub fn monitor_z80(cpu: &Z80) {
        use Reg16::*;
        for reg in [Af, Hl, De, Bc, Ix, Iy, Af2, Hl2, De2, Bc2, Pc, Sp, Wz].into_iter() {
            print!("{:#?}=0x{:04X} ", reg, cpu.reg16(reg).value())
        }
        println!();
    }

    pub fn monitor_z80_pins(cpu: &Z80) {
        println!(
            "MREQ:{} IOREQ:{} WR:{} RD:{} M1:{} RFSH:{}",
            cpu.get_pin(Z80Pin::Neg_MREQ),
            cpu.get_pin(Z80Pin::Neg_IORQ),
            cpu.get_pin(Z80Pin::Neg_WR),
            cpu.get_pin(Z80Pin::Neg_RD),
            cpu.get_pin(Z80Pin::Neg_M1),
            cpu.get_pin(Z80Pin::Neg_RFSH)
        );
    }

    pub fn monitor_z80_flags(cpu: &Z80) {   
        if cpu.flag(Z80Flags::S) {
            print!("S");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::Z) {
            print!("Z");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::Y) {
            print!("Y");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::H) {
            print!("H");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::X) {
            print!("X");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::P) {
            print!("P");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::N) {
            print!("N");
        } else {
            print!("-");
        }
      

        if cpu.flag(Z80Flags::C) {
            print!("C");
        } else {
            print!("-");
        }
        println!();
    }

    pub fn monitor_ga(ga: &GateArray) {
        println!(
            "Pen: {:#?} Mode:{} R52: {} UR: {:?}  LR: {:?} Palette {:?}",
            ga.pen().value(),
		    ga.current_mode() as u8,
            ga.r52(),
            ga.upper_rom_pagging(),
            ga.lower_rom_pagging(),
            ga.palette()
        );
    }
    pub fn monitor_ga_pins(cpu: &GateArray) {
        println!(
            "A15:{} A14:{} IORQ:{}",
            cpu.get_pin(GateArrayPin::A15),
            cpu.get_pin(GateArrayPin::A14),
            cpu.get_pin(GateArrayPin::IORQ),
        );
    }

    pub fn monitor_next_instruction(motherboard: &Motherboard) {
        println!("{}", motherboard.disassemble_next_instruction().to_string());
    }
}

#[derive(Debug)]
pub struct DisassembledInstruction {
    pub token: Token,
    pub address: u16,
    pub memory: SmallVec<[u8; 3]>,
}

impl ToString for DisassembledInstruction {
    fn to_string(&self) -> String {
        format!("0x{:04x} {} {:02x?}", self.address, self.token, self.memory)
    }
}
