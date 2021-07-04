use crate::debug::breakpoint::BreakPoint;

#[derive(Debug)]
pub enum Command {
	/// Start the program
	Run(u16),
	/// Execute next instruction
	Next,
	/// Exacute next instraction inluding all the implied ones (call/ldir/...)
	Step,
	/// Continue until next breakpoint
	Continue,

	/// Disassemble
	Disassemble(Option<u16>),

	/// Add a breakpoint
	Break(BreakPoint),
	/// List some kind of information
	Info(InfoKind),
	/// Quit the interpretor

	Quit
}


#[derive(Debug)]
pub enum InfoKind {
	BreakPoint,
	Z80,
	Crtc,
	Ga,
	Stack
}

pub struct WrongInput {

}


/// Simple command parser. To be improved later
pub fn parse_command(line: &str) -> Result<Command, WrongInput> {
	let line = line.to_ascii_lowercase();
	let words = line.split(" ");
	let words = words.collect::<Vec<_>>();

	let parse_hexa = |s: &str| {
		u16::from_str_radix(
			s,
			16)
			.map_err(|e| WrongInput{})
	};

	if words.len() == 1 {
		match words[0] {
			"n" | "next"=> Ok(Command::Next),
			"s" | "step"=> Ok(Command::Step),
			"c" | "Continue"=> Ok(Command::Continue),
			"n" | "next"=> Ok(Command::Next),
			"q" | "quit" => Ok(Command::Quit),
			"d" | "disass" | "disassemble" => Ok(Command::Disassemble(None)),
			_ => Err(WrongInput{})
		}
	} else if words.len() == 2 {
		match words[0] {
			"b" | "break" => {
				let address = parse_hexa(words[1])?;
				Ok(Command::Break(BreakPoint::Address(address)))
			},

			"d" | "disass" | "disassemble" => {
				let address = parse_hexa(words[1])?;
				Ok(Command::Disassemble(Some(address)))
			},

			"i" | "info" => {
				match words[1] {
					"b" | "breakpoints" => {
						Ok(Command::Info(InfoKind::BreakPoint))
					},
					"z" | "z80" | "cpu"=> {
						Ok(Command::Info(InfoKind::Z80))
					},
					"c" | "crtc" => {
						Ok(Command::Info(InfoKind::Crtc))
					},
					"s" | "stack" => {
						Ok(Command::Info(InfoKind::Stack))
					},
					"g" | "ga" => {
						Ok(Command::Info(InfoKind::Ga))
					}
					_ => Err(WrongInput{})
				}
			},
			"r" | "run" => {
				 let address = parse_hexa(words[1])?;
				Ok(Command::Run(address))
			},
			_ => Err(WrongInput{})
		}
	}
	else {
		Err(WrongInput{})
	}
}