use std::pin::Pin;

use amsrust::{chip::Chip, crtc::CrtcPin, ga::GateArrayPin, motherboard::*, z80::{Z80, Z80Pin}};
use cpclib_macros::*;
use itertools::Itertools;

enum PinEnum {
	Cpu(Z80Pin),
	Ga(GateArrayPin),
	Crtc(CrtcPin)
}


struct  TickCollector {
	/// List of collected bits
	collected: Vec<Vec<bool>>,
	/// List of pins to collect
	pins: Vec<(PinEnum, &'static str)>
}

impl Default for TickCollector {
	fn default() -> Self {
		let pins = vec![
			(PinEnum::Ga(GateArrayPin::CCLK), "CCLK"),
			(PinEnum::Ga(GateArrayPin::NCPU), "NCPU"),

			(PinEnum::Cpu(Z80Pin::Neg_M1), "/M1"),
			(PinEnum::Cpu(Z80Pin::Neg_RD), "/RD"),
			(PinEnum::Cpu(Z80Pin::Neg_WR), "/WR"),
			(PinEnum::Cpu(Z80Pin::Neg_IORQ), "/IORQ"),
			(PinEnum::Cpu(Z80Pin::Neg_MREQ), "/MREQ"),

			(PinEnum::Crtc(CrtcPin::CS), "CS"),
			(PinEnum::Crtc(CrtcPin::EN), "EN")
		];

		let collected = pins.iter()
							.map(|_| Vec::new())
							.collect_vec();
		
		TickCollector{
			collected,
			pins
		}
	}
}

impl TickCollector {
	fn collect(&mut self, motherboard: &Motherboard) {

		for (idx, pin_data) in self.pins.iter().enumerate() {
			let bit = match pin_data.0 {
				PinEnum::Ga(pin) => motherboard.ga().get_pin(pin),
				PinEnum::Cpu(pin) => motherboard.cpu().get_pin(pin),
				PinEnum::Crtc(pin) => motherboard.crtc().get_pin(pin),
			};
			self.collected[idx].push(bit);
		}
	}

	fn render(&self) {
		let header = self.pins.iter()
			.map(|d| d.1)
			.join(",");

		println!("{}", header);

		for row in 0..self.len() {
			let line = self.collected.iter()
				.map(|d| d[row])
				.map(|b| if b {"1"} else{"0"})
				.join(",");
			println!("{}", line)
		}
	}

	fn len(&self) -> usize {
		self.collected[0].len()
	}
}


fn follow_nticks(motherboard: &mut Motherboard, mut count: usize ) {
	let mut collector = TickCollector::default();
	loop {
		motherboard.tick();
		collector.collect(&motherboard);
		count -= 1;
		if count == 0 {
			break;
		}
	}
	collector.render();
}

fn thirty_two_nops(motherboard: &mut Motherboard) {
	// no need to fill memory of a prog as there is only nops there
	follow_nticks(motherboard, 32);
}


fn main() {
	let mut motherboard = MotherboardBuilder::default()
										.disable_lower_rom()					.disable_upper_rom()
										.build();

	motherboard.set_sp(0x8000);
	motherboard.set_pc(0x1000);
	motherboard.write_buffer(0x38, assemble!("
		ei
		ret
	"));

	thirty_two_nops(&mut motherboard);


}	