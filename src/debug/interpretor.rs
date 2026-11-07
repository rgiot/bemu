use crate::debug::breakpoint::BreakPoint;
use crate::symbols::SymbolTable;

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
    /// Clear a Breakpoint
    Clear(BreakPoint),

    /// Force the display of some information before each command
    Display(InfoKind),

    /// List some kind of information
    Info(InfoKind),
    /// Read byte at the requested address
    Peek(u16),
    Help,
    /// Quit the interpretor
    Quit,
}

#[derive(Debug)]
pub enum InfoKind {
    BreakPoint,
    Z80,
    Crtc,
    Ga,
    Stack,
}

pub struct WrongInput {}
/// An address: a symbol, hexadecimal digits, or a symbol plus or minus a number.
fn parse_address(s: &str, symbols: &SymbolTable) -> Result<u16, WrongInput> {
    symbols
        .address_of(s)
        .or_else(|| u16::from_str_radix(s, 16).ok())
        .or_else(|| symbols.resolve(s))
        .ok_or(WrongInput {})
}

fn get_info(word: &str) -> Result<InfoKind, WrongInput> {
    match word.to_ascii_lowercase().as_str() {
        "b" | "breakpoints" => Ok(InfoKind::BreakPoint),
        "z" | "z80" | "cpu" => Ok(InfoKind::Z80),
        "c" | "crtc" => Ok(InfoKind::Crtc),
        "s" | "stack" => Ok(InfoKind::Stack),
        "g" | "ga" => Ok(InfoKind::Ga),
        _ => Err(WrongInput {}),
    }
}

fn get_breakpoint(word: &str, symbols: &SymbolTable) -> Result<BreakPoint, WrongInput> {
    let address = parse_address(word, symbols)?;
    Ok(BreakPoint::Address(address))
}

/// Simple command parser. To be improved later. The addresses can be named by `symbols`.
pub fn parse_command(line: &str, symbols: &SymbolTable) -> Result<Command, WrongInput> {
    let mut words = line
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if let Some(command) = words.first_mut() {
        command.make_ascii_lowercase();
    }
    let words = words.iter().map(String::as_str).collect::<Vec<_>>();

    if words.len() == 1 {
        match words[0] {
            "n" | "next" => Ok(Command::Next),
            "s" | "step" => Ok(Command::Step),
            "c" | "cont" | "continue" => Ok(Command::Continue),
            "h" | "help" => Ok(Command::Help),
            "q" | "quit" => Ok(Command::Quit),
            "d" | "disass" | "disassemble" => Ok(Command::Disassemble(None)),
            _ => Err(WrongInput {}),
        }
    } else if words.len() == 2 {
        match words[0] {
            "b" | "break" => Ok(Command::Break(get_breakpoint(words[1], symbols)?)),
            "clear" => Ok(Command::Clear(get_breakpoint(words[1], symbols)?)),

            "d" | "disass" | "disassemble" => {
                let address = parse_address(words[1], symbols)?;
                Ok(Command::Disassemble(Some(address)))
            }

            "disp" | "display" => Ok(Command::Display(get_info(words[1])?)),

            "i" | "info" => Ok(Command::Info(get_info(words[1])?)),
            "peek" => {
                let address = parse_address(words[1], symbols)?;
                Ok(Command::Peek(address))
            }

            "r" | "run" => {
                let address = parse_address(words[1], symbols)?;
                Ok(Command::Run(address))
            }
            _ => Err(WrongInput {}),
        }
    } else {
        Err(WrongInput {})
    }
}
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn addresses_by_name_or_in_hex() {
        let mut symbols = SymbolTable::default();
        symbols.insert("Loop", 0x4010);
        let address = |line: &str| match parse_command(line, &symbols) {
            Ok(Command::Break(BreakPoint::Address(address)))
            | Ok(Command::Disassemble(Some(address))) => Some(address),
            _ => None,
        };
        assert_eq!(address("b Loop"), Some(0x4010));
        assert_eq!(
            address("B loop+2"),
            Some(0x4012),
            "the command in any case, the name too"
        );
        assert_eq!(
            address("d 4000"),
            Some(0x4000),
            "bare numbers are hexadecimal"
        );
        assert_eq!(address("b nowhere"), None);
    }
}
