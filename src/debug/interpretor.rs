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
	/// Add a breakpoint
	Break(BreakPoint),
	/// List Breakpoints
	InfoBreakpoints,
	/// Quit the interpretor
	Quit

}

pub struct WrongInput {

}


/// Simple command parser. To be improved later
pub fn parse_command(line: &str) -> Result<Command, WrongInput> {
	let line = line.to_ascii_lowercase();
	let words = line.split(" ");
	let words = words.collect::<Vec<_>>();

	if words.len() == 1 {
		match words[0] {
			"n" | "next"=> Ok(Command::Next),
			"s" | "step"=> Ok(Command::Step),
			"c" | "Continue"=> Ok(Command::Continue),
			"n" | "next"=> Ok(Command::Next),
			"q" | "quit" => Ok(Command::Quit),
			_ => Err(WrongInput{})
		}
	} else if words.len() == 2 {
		match words[0] {
			"b" | "break" => {
				let address = u16::from_str_radix(
					words[1],
					16)
					.map_err(|e| WrongInput{})?;
				Ok(Command::Break(BreakPoint::Address(address)))
			},
			"i" | "info" => {
				match words[1] {
					"b" | "breakpoints" => {
						Ok(Command::InfoBreakpoints)
					},
					_ => Err(WrongInput{})
				}
			},
			"r" | "run" => {
				let address = u16::from_str_radix(
					words[1],
					16)
					.map_err(|e| WrongInput{})?;
				Ok(Command::Run(address))
			},
			_ => Err(WrongInput{})
		}
	}
	else {
		Err(WrongInput{})
	}
}