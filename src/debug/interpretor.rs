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

	/// Force the display of some information before each command
	Display(InfoKind),

	/// List some kind of information
	Info(InfoKind),
	/// Read byte at the requested address
	Peek(u16),
	Help,
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

fn get_info(word: &str) -> Result<InfoKind, WrongInput>{
	match word {
		"b" | "breakpoints" => {
			Ok(InfoKind::BreakPoint)
		},
		"z" | "z80" | "cpu"=> {
			Ok(InfoKind::Z80)
		},
		"c" | "crtc" => {
			Ok(InfoKind::Crtc)
		},
		"s" | "stack" => {
			Ok(InfoKind::Stack)
		},
		"g" | "ga" => {
			Ok(InfoKind::Ga)
		}
		_ => Err(WrongInput{})
	}	
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
			"c" | "cont" | "continue"=> Ok(Command::Continue),
			"h" | "help" => Ok(Command::Help),
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

			"disp" | "display" => {
					Ok(
						Command::Display (
							get_info(words[1])?
						)
					)
								
			},

			"i" | "info" => {
				Ok(
					Command::Info(
						get_info(words[1])?
					)
				)
			},
			"peek" => {
				let address = parse_hexa(words[1])?;
				Ok(Command::Peek(address))
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