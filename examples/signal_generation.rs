use std::pin::Pin;

use amsrust::{chip::{Chip, ReadAddressChip, ReadDataChip}, crtc::CrtcPin, debug::Monitor, ga::GateArrayPin, motherboard::*, z80::{Z80, Z80Pin}};
use cpclib_macros::*;
use itertools::Itertools;

enum PinEnum {
	Cpu(Z80Pin),
	Ga(GateArrayPin),
	Crtc(CrtcPin),

	CpuAddr,
	CpuData,
	CrtcAddr,

	CpuState,
}

enum PinData {
	Bool(bool),
	Value(u16),
	String(String)
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

impl From<String> for PinData {
	fn from(s: String) -> Self {
		Self::String(s)
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
			},
			Self::String(s)=> s.clone()
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

			(PinEnum::CpuState, "STATE"),
			(PinEnum::Cpu(Z80Pin::Neg_M1), "/M1"),
			(PinEnum::Cpu(Z80Pin::Neg_WAIT), "/WAIT"),
			(PinEnum::Cpu(Z80Pin::Neg_RFSH), "/RFSH"),
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

				PinEnum::CpuState => {
					motherboard.cpu().next_state().to_string().into()
				}
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

fn execute_code(motherboard: &mut Motherboard, init: &[u8], code: &[u8]) {
	let mut collector = TickCollector::default();

	motherboard.write_buffer(0x4000, init);
	motherboard.set_pc(0x4000);
	motherboard.tick_until_next_instruction();
	while motherboard.pc() != 0x4000 + init.len() as u16 {
		motherboard.execute_next_opcode();
		Monitor::monitor(motherboard);
	}


	motherboard.write_buffer(0x4000, dbg!(code));
	motherboard.set_pc(0x4000);


//	Monitor::monitor(motherboard);

	while  motherboard.pc() != 0x4000 + code.len() as u16 || !motherboard.is_aligned_with_new_instruction() {
//	Monitor::monitor(motherboard);
		collector.collect(&motherboard);
		motherboard.tick();
	}

	collector.render();
}

fn test_out(motherboard: &mut Motherboard) {
	execute_code(
			board,
		assemble!("
		org 0x4000
		ld bc, 0xbc00 + 6
		"),
		assemble!("
		org 0x4000
		out (c), c
		nop
		")
	)
}

fn test_outd(motherboard: &mut Motherboard) {
	motherboard.write_byte(0x5000, 0x0f);
	execute_code(
		 motherboard,
		assemble!("
		org 0x4000
		ld b, 0x80
		ld hl, 0x5000
		"),
		assemble!("
		org 0x4000
		outd
		")
	)
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


	// TODO make a menu
	//thirty_two_nops(&mut motherboard);a
	//test_out(&mut motherboard);
	test_outd(&mut motherboard);



}	