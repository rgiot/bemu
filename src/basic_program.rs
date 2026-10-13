//! The Locomotive BASIC 1.1 program and interpreter state in RAM, for
//! debuggers: where the program and its variables are, the line and
//! statement being run, the program as lines of statements, and loading a
//! program in place of the one in memory.
//!
//! Everything is read in the base 64K, whatever the CPU's mapping (the
//! program at `0x0170` sits under the lower ROM). The addresses are those
//! of the 6128's BASIC 1.1, as used by AMSpiriT and cpclib-dap.

use cpclib::basic::tokens::{BasicToken, BasicTokenNoPrefix};
use cpclib::basic::BasicProgram;

use crate::motherboard::Motherboard;

/// The byte before the statement being run (`0` in direct mode).
pub const CURRENT_STATEMENT: u16 = 0xAE1B;
/// The line-number field of the line being run (`0` in direct mode).
pub const CURRENT_LINE_NUMBER_FIELD: u16 = 0xAE1D;
/// The byte before the program.
pub const END_OF_RESERVED_AREA: u16 = 0xAE64;
/// End of the program, start of the variables.
pub const PROGRAM_END: u16 = 0xAE66;
pub const VARIABLES_START: u16 = 0xAE68;
pub const ARRAYS_START: u16 = 0xAE6A;
pub const ARRAYS_END: u16 = 0xAE6C;
/// 27 chain heads of the scalar variables (A-Z and `DEF FN`).
pub const VARIABLE_CHAIN_HEADS: u16 = 0xADB7;
const VARIABLE_CHAIN_HEADS_SIZE: u16 = 27 * 2;
/// `AUTO`, `ON ERROR GOTO` and `PROTECT` flags, cleared by a load.
const AUTO_FLAG: u16 = 0xAC01;
const ERROR_FLAG: u16 = 0xAD90;
const PROTECT_FLAG: u16 = 0xAE2C;
/// Where a program is loaded.
pub const PROGRAM_START: u16 = 0x0170;
/// HIMEM of a 6128 with AMSDOS: a program must end below.
const HIGHEST_PROGRAM_END: u16 = 0xA67B;

/// The ROM's per-statement entry point, just after it stored the
/// statement's address in [`CURRENT_STATEMENT`] (`LD (&AE1B),HL` at
/// `0xDE60`): reached once per statement run, with both pointers fresh.
pub const STATEMENT_HOOK: u16 = 0xDE63;
const STATEMENT_HOOK_CODE: [u8; 3] = [0x22, 0x1b, 0xae];

fn peek(motherboard: &Motherboard, address: u16) -> u8 {
    motherboard.read_video_ram_byte(address)
}

fn deek(motherboard: &Motherboard, address: u16) -> u16 {
    u16::from_le_bytes([peek(motherboard, address), peek(motherboard, address.wrapping_add(1))])
}

fn doke(motherboard: &mut Motherboard, address: u16, value: u16) {
    let [low, high] = value.to_le_bytes();
    motherboard.write_base_ram(address, low);
    motherboard.write_base_ram(address.wrapping_add(1), high);
}

/// What `GET /api/basic_state` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicState {
    pub prog_start: u16,
    pub txttop: u16,
    pub vartop: u16,
    pub arrend: u16,
    /// The line being run; `None` in direct mode.
    pub line: Option<u16>,
    /// The byte before the statement being run (as `/api/basic_listing`
    /// gives them).
    pub stmt_addr: u16,
    /// Whether that statement is the first of its line.
    pub first_statement: bool
}

impl BasicState {
    pub fn read(motherboard: &Motherboard) -> Self {
        let line_field = deek(motherboard, CURRENT_LINE_NUMBER_FIELD);
        let stmt_addr = deek(motherboard, CURRENT_STATEMENT);
        let line = (line_field != 0).then(|| deek(motherboard, line_field));
        Self {
            prog_start: deek(motherboard, END_OF_RESERVED_AREA).wrapping_add(1),
            txttop: deek(motherboard, PROGRAM_END),
            vartop: deek(motherboard, ARRAYS_START),
            arrend: deek(motherboard, ARRAYS_END),
            line,
            stmt_addr,
            // The first statement follows the line number's high byte.
            first_statement: line.is_some() && stmt_addr == line_field.wrapping_add(1)
        }
    }
}

/// Whether the CPU is at the BASIC ROM's per-statement point (see
/// [`STATEMENT_HOOK`]).
pub fn at_statement(motherboard: &Motherboard) -> bool {
    motherboard.pc() == STATEMENT_HOOK
        && (0..3).all(|i| motherboard.read_byte(STATEMENT_HOOK - 3 + i) == STATEMENT_HOOK_CODE[i as usize])
}

/// One statement of a [`ListedLine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedStatement {
    /// What [`CURRENT_STATEMENT`] holds while it runs: the byte before it
    /// (the line number's high byte, or the `:` before it).
    pub addr: u16,
    /// The `:` or end of line after it (exclusive end).
    pub end: u16,
    /// Preceded by a `:`.
    pub colon: bool,
    pub text: String,
    /// The variables it names, as written (`A%`, `MSG$`).
    pub vars: Vec<String>
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedLine {
    /// The line's length field.
    pub addr: u16,
    pub number: u16,
    pub statements: Vec<ListedStatement>
}

/// The program's bytes (with the end marker) and its address.
fn program_bytes(motherboard: &Motherboard) -> (u16, Vec<u8>) {
    let state = BasicState::read(motherboard);
    let end = state.txttop.max(state.prog_start);
    (state.prog_start, (state.prog_start..end).map(|a| peek(motherboard, a)).collect())
}

/// The program in RAM, decoded.
pub fn program(motherboard: &Motherboard) -> Result<BasicProgram, String> {
    let (_, bytes) = program_bytes(motherboard);
    BasicProgram::decode(&bytes).map_err(|e| format!("{e:?}"))
}

/// The program in RAM as lines of statements, with their addresses.
pub fn listing(motherboard: &Motherboard) -> Result<Vec<ListedLine>, String> {
    let (start, bytes) = program_bytes(motherboard);
    let program = BasicProgram::decode(&bytes).map_err(|e| format!("{e:?}"))?;
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for line in program.lines() {
        let Some(&[low, high]) = bytes.get(offset..offset + 2).map(|b| <&[u8; 2]>::try_from(b).unwrap())
        else {
            break;
        };
        let length = u16::from_le_bytes([low, high]) as usize;
        if length < 5 {
            break;
        }
        let addr = start.wrapping_add(offset as u16);
        lines.push(ListedLine {
            addr,
            number: line.line_number(),
            statements: statements(addr, length, line.tokens())
        });
        offset += length;
    }
    Ok(lines)
}

/// Split a line (at `addr`, `length` bytes) at its `:`.
fn statements(addr: u16, length: usize, tokens: &[BasicToken]) -> Vec<ListedStatement> {
    let first = addr.wrapping_add(3);
    let last = addr.wrapping_add(length as u16 - 1);
    let mut statements = Vec::new();
    let mut current = ListedStatement { addr: first, end: last, colon: false, text: String::new(), vars: Vec::new() };
    let mut position = addr.wrapping_add(4);
    for token in tokens {
        if token.is_statement_separator() {
            current.end = position;
            let next = ListedStatement { addr: position, end: last, colon: true, text: String::new(), vars: Vec::new() };
            statements.push(std::mem::replace(&mut current, next));
        }
        else if matches!(token, BasicToken::SimpleToken(BasicTokenNoPrefix::EndOfTokenisedLine)) {
            break;
        }
        else {
            current.text.push_str(&token.to_string());
            if let BasicToken::Variable(..) = token {
                let name = token.to_string().trim().to_owned();
                if !current.vars.contains(&name) {
                    current.vars.push(name);
                }
            }
        }
        position = position.wrapping_add(token.as_bytes().len() as u16);
    }
    statements.push(current);
    // Token lengths not adding up to the line's: one statement.
    if position != last {
        let text = statements.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(":");
        let vars = statements.iter().flat_map(|s| s.vars.clone()).collect();
        return vec![ListedStatement { addr: first, end: last, colon: false, text, vars }];
    }
    for statement in &mut statements {
        statement.text = statement.text.trim().to_owned();
    }
    statements
}

/// Replace the program in RAM by `source` (lines of Locomotive BASIC, as
/// typed), as `LOAD` would: no variables, no `AUTO`, no `ON ERROR`, not
/// protected. Returns the program's size.
pub fn load(motherboard: &mut Motherboard, source: &str) -> Result<usize, String> {
    let program = BasicProgram::parse(source).map_err(|e| format!("{e:?}"))?;
    let bytes = program.as_bytes();
    let start = BasicState::read(motherboard).prog_start;
    let end = start as usize + bytes.len();
    if end > HIGHEST_PROGRAM_END as usize {
        return Err(format!("program too large: {} bytes, {} available", bytes.len(), HIGHEST_PROGRAM_END - start));
    }
    for (i, &byte) in bytes.iter().enumerate() {
        motherboard.write_base_ram(start + i as u16, byte);
    }
    let end = end as u16;
    for pointer in [PROGRAM_END, VARIABLES_START, ARRAYS_START, ARRAYS_END] {
        doke(motherboard, pointer, end);
    }
    for i in 0..VARIABLE_CHAIN_HEADS_SIZE {
        motherboard.write_base_ram(VARIABLE_CHAIN_HEADS + i, 0);
    }
    for flag in [AUTO_FLAG, ERROR_FLAG, PROTECT_FLAG] {
        motherboard.write_base_ram(flag, 0);
    }
    Ok(bytes.len())
}

