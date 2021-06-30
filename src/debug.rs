use cpclib_asm::preamble::Token;
use smallvec::SmallVec;

use crate::chip::{Chip, ReadAddressChip, ReadDataChip};
use crate::crtc::{Crtc, CrtcPin};
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
        print!("\t");
        Monitor::monitor_z80_flags(motherboard.cpu());
        print!("\t");
        Monitor::monitor_z80_pins(motherboard.cpu());

        print!("stack:\t");
        Monitor::monitor_stack(motherboard, 2, 8);

        print!("ga:\t");
        Monitor::monitor_ga(motherboard.ga());
        print!("\t");
        Monitor::monitor_ga_pins(motherboard.ga());

        print!("crtc:\t");
        Monitor::monitor_crtc_registers(motherboard.crtc());
        print!("\t");
        Monitor::monitor_crtc_counters(motherboard.crtc());
        print!("\t");
        Monitor::monitor_crtc_pins(motherboard.crtc());

        print!("next:\t");
        Monitor::monitor_next_instruction(&motherboard)
    }

    pub fn monitor_crtc_registers(crtc: &Crtc) {
        println!(
            "AR:{} R0:{} R1:{} R2:{} R3:{} R4:{} R5:{} R6:{} R7:{} R8:{} R9:{} R10:{} R11:{} R12:{} R13:{}",
            crtc.ar(),
            crtc.r0(),
            crtc.r1(),
            crtc.r2(),
            crtc.r3(),
            crtc.r4(),
            crtc.r5(),
            crtc.r6(),
            crtc.r7(),
            crtc.r8(),
            crtc.r9(),
            crtc.r10(),
            crtc.r11(),
            crtc.r12(),
            crtc.r13(),
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

    pub fn monitor_crtc_pins(crtc: &Crtc) {
        println!("CCLK:{} EN:{} A8/RS:{} A9/RW:{} A14/CS:{}, data: {}",
            crtc.get_pin(CrtcPin::CCLK),
            crtc.get_pin(CrtcPin::EN),
            crtc.get_pin(CrtcPin::RS),
            crtc.get_pin(CrtcPin::RW),
            crtc.get_pin(CrtcPin::CS),
            crtc.get_data_from_pins(),
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
            "MREQ:{} IOREQ:{} WR:{} RD:{} M1:{} RFSH:{} ADR:0x{:x} DATA:{}",
            cpu.get_pin(Z80Pin::Neg_MREQ),
            cpu.get_pin(Z80Pin::Neg_IORQ),
            cpu.get_pin(Z80Pin::Neg_WR),
            cpu.get_pin(Z80Pin::Neg_RD),
            cpu.get_pin(Z80Pin::Neg_M1),
            cpu.get_pin(Z80Pin::Neg_RFSH),
            cpu.get_address_from_pins(),
            cpu.get_data_from_pins()
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
            cpu.get_pin(GateArrayPin::Neg_IORQ),
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
    pub memory: SmallVec<[u8; 4]>,
}

impl ToString for DisassembledInstruction {
    fn to_string(&self) -> String {
        format!("0x{:04x} {} {:02x?}", self.address, self.token, self.memory)
    }
}
