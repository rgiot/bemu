use std::pin::Pin;

use amsrust::{chip::{Chip, ReadAddressChip, ReadDataChip}, crtc::CrtcPin, ga::GateArrayPin, motherboard::*, z80::{Z80, Z80Pin}};
use cpclib_macros::*;
use itertools::Itertools;

enum PinEnum {
	Cpu(Z80Pin),
	Ga(GateArrayPin),
	Crtc(CrtcPin),

	CpuAddr,
	CpuData,
	CrtcAddr
}

enum PinData {
	Bool(bool),
	Value(u16)
}

impl From<bool> for PinData {
	fn from(d: bool) -> Self {
		Self::Bool(d)
	}
}
impl From<u16> for PinData {
	fn from(d: u16) -> Self {
		Self::Value(d)
	}
}

impl ToString for PinData {
	fn to_string(&self) -> String {
		match self {
			Self::Bool(b) => {
				if *b {
					"1".to_owned()
				}
				else {
					"0".to_owned()
				}
			},
			Self::Value(v) => {
				v.to_string()
			}
		}
	}
}


struct  TickCollector {
	/// List of collected bits
	collected: Vec<Vec<PinData>>,
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

			(PinEnum::CpuAddr, "CPU ADDR"),
			(PinEnum::CpuData, "CPU DATA"),

			(PinEnum::Crtc(CrtcPin::CS), "CS"),
			(PinEnum::Crtc(CrtcPin::EN), "EN"),

			(PinEnum::CrtcAddr, "SCREEN"),
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

			let data: PinData = match pin_data.0 {
				PinEnum::Ga(pin) => motherboard.ga().get_pin(pin).into(),
				PinEnum::Cpu(pin) => motherboard.cpu().get_pin(pin).into(),
				PinEnum::Crtc(pin) => motherboard.crtc().get_pin(pin).into(),

				PinEnum::CpuAddr => motherboard.cpu().get_address_from_pins().into(),
				PinEnum::CpuData => (motherboard.cpu().get_data_from_pins() as u16).into(),

				PinEnum::CrtcAddr => {motherboard.crtc().get_address_from_pins().into()}
			};
			self.collected[idx].push(data);
		}
	}

	fn render(&self) {
		let header = self.pins.iter()
			.map(|d| d.1)
			.join(",");

		println!("16MHz,{}", header);

		for row in 0..self.len() {
			let line = self.collected.iter()
				.map(|d| &d[row])
				.map(|b| b.to_string())
				.join(",");
			println!("0,{}\n1,{}", line,line)
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