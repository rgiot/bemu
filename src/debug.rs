use cpclib_asm::preamble::Token;
use smallvec::SmallVec;

use crate::ga::*;
use crate::motherboard::Motherboard;
use crate::z80::{HasValue, Reg16, Z80};

pub struct Monitor {}

impl Monitor {
    pub fn monitor(motherboard: &Motherboard) {
        print!("z80: ");
        Monitor::monitor_z80(motherboard.cpu());

        print!("stack: ");
        Monitor::monitor_stack(motherboard, 10);

        print!("ga: ");
        Monitor::monitor_ga(motherboard.ga());

        print!("next: ");
        Monitor::monitor_next_instruction(&motherboard)
    }

    pub fn monitor_stack(motherboard: &Motherboard, size: u16) {
        let sp = motherboard.cpu().reg16(Reg16::Sp).value();
        (0..size).for_each(|i| {
            let addr = sp.wrapping_add(2 * i);
            print!("0x{:04X}=0x{:04X} ", addr, motherboard.read_word(addr))
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

    pub fn monitor_ga(ga: &GateArray) {
        println!(
            "Pen: {:#?}  UR: {:?}  LR: {:?} Palette {:?}",
            ga.pen().value(),
            ga.upper_rom_pagging(),
            ga.lower_rom_pagging(),
            ga.palette()
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
    pub memory: SmallVec<[u8;3]>
}

impl ToString for DisassembledInstruction {
    fn to_string(&self) -> String {
        format!(
            "0x{:04x} {} {:02x?}", 
            self.address,
            self.token, 
            self.memory
        )
    }
}