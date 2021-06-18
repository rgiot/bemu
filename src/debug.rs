use cpclib_asm::preamble::Token;
use smallvec::SmallVec;

use crate::chip::Chip;
use crate::ga::*;
use crate::motherboard::Motherboard;
use crate::z80::Z80Pin;
use crate::z80::{HasValue, Reg16, Z80};

pub struct Monitor {}

impl Monitor {
    pub fn monitor(motherboard: &Motherboard) {
        print!("z80: ");
        Monitor::monitor_z80(motherboard.cpu());
        Monitor::monitor_z80_pins(motherboard.cpu());

        print!("stack: ");
        Monitor::monitor_stack(motherboard, 2, 8);

        print!("ga: ");
        Monitor::monitor_ga(motherboard.ga());
        Monitor::monitor_ga_pins(motherboard.ga());

        print!("next: ");
        Monitor::monitor_next_instruction(&motherboard)
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

    pub fn monitor_ga(ga: &GateArray) {
        println!(
            "Pen: {:#?}  UR: {:?}  LR: {:?} Palette {:?}",
            ga.pen().value(),
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
