use crate::motherboard::Motherboard;
use crate::z80::{Z80, Reg16, HasValue};
use crate::ga::*;

pub struct Monitor {
	
}

impl Monitor {
	pub fn monitor(motherboard: &Motherboard) {
		print!("z80: ");
		Monitor::monitor_z80(motherboard.cpu());

		print!("stack: ");
		Monitor::monitor_stack(motherboard, 10);

		print!("ga: ");
		Monitor::monitor_ga(motherboard.ga());

	}

	pub fn monitor_stack(motherboard: &Motherboard, size: u16) {
		let sp = motherboard.cpu().reg16(Reg16::Sp).value();
		(0..size).for_each(|i|{
			let addr = sp.wrapping_add(2*i);
			print!(
				"0x{:04X}=0x{:04X} ",
				addr,
				motherboard.read_word(addr)
			)
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
}