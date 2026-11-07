//! Advanced breakpoints - execution, memory or IO; read, write or both;
//! stopping or only watching - as the rasm and basm `BREAKPOINT` directives
//! and ACE describe them, and as the REMU snapshot chunk stores them
//! (`acebreak` entries, see `cpclib-sna`'s `AdvancedRemuBreakPoint`).
//!
//! Matching: an access hits when its address, compared through `mask`, is
//! within `address..address + size` (also through `mask`), its direction is
//! one `access` covers and - when `value_mask` is not 0 - its value, through
//! `value_mask`, equals `value`. Memory breakpoints watch data accesses
//! (see [`crate::motherboard::BusAccess`]); execution ones the instruction
//! fetched at PC.
//!
//! A breakpoint may carry a [`Condition`] (an expression on the registers,
//! memory and the access): it then only fires while the condition holds.
//! Step counts are not supported yet: a breakpoint carrying one is refused
//! rather than made to fire on every hit.

use crate::motherboard::{BusAccess, BusAccessKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointKind {
    Exec,
    Memory,
    Io,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointAccess {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Pause the emulation.
    Stop,
    /// Only report the hit, keep running.
    Watch,
    /// Neither stop nor report: only act on the counters (see
    /// [`AdvancedBreakpoint::counters`]) and count the hit.
    Count,
}

/// What a hit does to one of the debugger's named NOP counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterAction {
    /// Start counting (creating the counter if needed).
    Start,
    /// Stop counting.
    Stop,
    /// Back to 0 (creating the counter if needed), running or not as it was.
    Reset,
}

impl CounterAction {
    pub fn name(&self) -> &'static str {
        match self {
            CounterAction::Start => "start",
            CounterAction::Stop => "stop",
            CounterAction::Reset => "reset",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdvancedBreakpoint {
    pub kind: BreakpointKind,
    pub access: BreakpointAccess,
    pub run: RunMode,
    pub address: u16,
    pub mask: u16,
    pub size: u16,
    pub value: u8,
    pub value_mask: u8,
    pub name: Option<String>,
    pub condition: Option<Condition>,
    /// Counters to act on at each hit (debugger extension), in order.
    pub counters: Vec<(CounterAction, String)>,
    /// Hits ignored before it acts (a pass count): 0 acts on the first.
    pub after: u64,
    /// Hits so far (its condition holding), counted by the debugger.
    pub hits: u64,
}

impl AdvancedBreakpoint {
    /// An execution breakpoint stopping on one address.
    pub fn exec(address: u16) -> Self {
        Self {
            kind: BreakpointKind::Exec,
            access: BreakpointAccess::ReadWrite,
            run: RunMode::Stop,
            address,
            mask: 0xffff,
            size: 1,
            value: 0,
            value_mask: 0,
            name: None,
            condition: None,
            counters: Vec::new(),
            after: 0,
            hits: 0,
        }
    }

    fn covers(&self, address: u16) -> bool {
        (address & self.mask).wrapping_sub(self.address & self.mask) < self.size.max(1)
    }

    /// Whether executing the instruction at `pc` hits.
    pub fn hit_by_execution(&self, pc: u16) -> bool {
        self.kind == BreakpointKind::Exec && self.covers(pc)
    }

    /// Whether the bus access hits.
    pub fn hit_by_access(&self, access: &BusAccess) -> bool {
        let (kind, write) = match access.kind {
            BusAccessKind::MemoryRead => (BreakpointKind::Memory, false),
            BusAccessKind::MemoryWrite => (BreakpointKind::Memory, true),
            BusAccessKind::IoRead => (BreakpointKind::Io, false),
            BusAccessKind::IoWrite => (BreakpointKind::Io, true),
        };
        let direction = match self.access {
            BreakpointAccess::Read => !write,
            BreakpointAccess::Write => write,
            BreakpointAccess::ReadWrite => true,
        };
        self.kind == kind
            && direction
            && self.covers(access.address)
            && (access.value & self.value_mask) == (self.value & self.value_mask)
    }

    /// Parse the description of a REMU `acebreak` entry, as rasm and basm
    /// write it: `MEM,W,STOP,addr=49152,mask=65535,size=1,value=0,
    /// valmask=0,name="imported",condition=,step=`. Positional fields and
    /// `key=value` ones may come in any order; missing ones take their
    /// defaults (EXEC, RW, STOP, mask 0xFFFF, size 1).
    pub fn parse_remu(text: &str) -> Result<Self, String> {
        let mut brk = Self::exec(0);
        for field in split_fields(text) {
            let field = field.trim();
            if field.is_empty() {
                continue;
            }
            let Some((key, value)) = field.split_once('=') else {
                match field.to_ascii_uppercase().as_str() {
                    "EXEC" | "EXECUTE" => brk.kind = BreakpointKind::Exec,
                    "MEM" | "MEMORY" => brk.kind = BreakpointKind::Memory,
                    "IO" => brk.kind = BreakpointKind::Io,
                    "R" | "READ" => brk.access = BreakpointAccess::Read,
                    "W" | "WRITE" => brk.access = BreakpointAccess::Write,
                    "RW" | "READWRITE" => brk.access = BreakpointAccess::ReadWrite,
                    "STOP" | "STOPPER" => brk.run = RunMode::Stop,
                    "WATCH" | "WATCHER" => brk.run = RunMode::Watch,
                    other => return Err(format!("unknown breakpoint field {other:?}")),
                }
                continue;
            };
            let value = value.trim();
            let number = |max: u32| -> Result<u32, String> {
                parse_number(value)
                    .filter(|v| *v <= max)
                    .ok_or_else(|| format!("{key}={value}: expected a number up to {max}"))
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "addr" | "address" => brk.address = number(0xffff)? as u16,
                "mask" => brk.mask = number(0xffff)? as u16,
                "size" => brk.size = number(0x10000)?.clamp(1, 0xffff) as u16,
                "value" => brk.value = number(0xff)? as u8,
                "valmask" | "value_mask" => brk.value_mask = number(0xff)? as u8,
                "name" => {
                    let name = value.trim_matches('"');
                    brk.name = (!name.is_empty() && name != "imported").then(|| name.to_owned());
                }
                "condition" if !value.trim_matches('"').is_empty() => {
                    brk.condition = Some(Condition::parse(value.trim_matches('"'))?);
                }
                "step" if !value.is_empty() => {
                    return Err(format!("step={value}: step counts are not supported yet"))
                }
                "condition" | "step" => {}
                other => return Err(format!("unknown breakpoint field {other:?}")),
            }
        }
        Ok(brk)
    }
}

/// Split on commas outside double quotes (names and conditions can hold
/// commas).
fn split_fields(text: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                fields.last_mut().unwrap().push(c);
            }
            ',' if !quoted => fields.push(String::new()),
            _ => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

/// Decimal, or hexadecimal with 0x, #, & or $.
pub fn parse_number(text: &str) -> Option<u32> {
    let text = text.trim();
    let hex = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix('#'))
        .or_else(|| text.strip_prefix('&'))
        .or_else(|| text.strip_prefix('$'));
    match hex {
        Some(digits) => u32::from_str_radix(digits, 16).ok(),
        None => text.parse().ok(),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn access(kind: BusAccessKind, address: u16, value: u8) -> BusAccess {
        BusAccess {
            kind,
            address,
            value,
        }
    }

    #[test]
    fn parses_what_cpclib_writes() {
        // The text of `AdvancedRemuBreakPoint`'s Display, as stored in REMU.
        let brk = AdvancedBreakpoint::parse_remu(
            r#"MEM,W,WATCH,addr=49152,mask=65535,size=16,value=0,valmask=0,name="screen",condition=,step="#
        )
        .unwrap();
        assert_eq!(brk.kind, BreakpointKind::Memory);
        assert_eq!(brk.access, BreakpointAccess::Write);
        assert_eq!(brk.run, RunMode::Watch);
        assert_eq!(
            (brk.address, brk.size, brk.name.as_deref()),
            (0xc000, 16, Some("screen"))
        );
        let conditional =
            AdvancedBreakpoint::parse_remu(r#"EXEC,RW,STOP,addr=1,condition="A==2,B==3""#);
        assert!(
            conditional.is_err(),
            "commas inside the quoted condition are not separators: A==2,B==3 is invalid"
        );
        let conditional =
            AdvancedBreakpoint::parse_remu(r#"EXEC,RW,STOP,addr=1,condition="A==2 && B==3""#)
                .unwrap();
        assert_eq!(conditional.condition.unwrap().source(), "A==2 && B==3");
        assert!(AdvancedBreakpoint::parse_remu("EXEC,addr=1,step=3").is_err());
        assert!(AdvancedBreakpoint::parse_remu("BOGUS").is_err());
    }

    #[test]
    fn memory_accesses_through_mask_size_and_value() {
        let brk =
            AdvancedBreakpoint::parse_remu("MEM,W,STOP,addr=#C000,size=0x10,value=#80,valmask=#80")
                .unwrap();
        assert!(brk.hit_by_access(&access(BusAccessKind::MemoryWrite, 0xc00f, 0xff)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::MemoryWrite, 0xc010, 0xff)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::MemoryWrite, 0xc000, 0x7f)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::MemoryRead, 0xc000, 0xff)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::IoWrite, 0xc000, 0xff)));
        // A mask ignoring the high bits: every 16K window.
        let mirrored =
            AdvancedBreakpoint::parse_remu("MEM,RW,STOP,addr=0x0100,mask=0x3fff").unwrap();
        assert!(mirrored.hit_by_access(&access(BusAccessKind::MemoryRead, 0x4100, 0)));
    }

    #[test]
    fn io_ports_with_the_usual_partial_decoding() {
        // The gate array answers whenever A15=0, A14=1.
        let brk = AdvancedBreakpoint::parse_remu("IO,W,STOP,addr=0x7f00,mask=0xc000").unwrap();
        assert!(brk.hit_by_access(&access(BusAccessKind::IoWrite, 0x7f8d, 0x8d)));
        assert!(brk.hit_by_access(&access(BusAccessKind::IoWrite, 0x7fff, 0)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::IoWrite, 0xbc00, 0)));
    }

    #[test]
    fn execution_ranges() {
        let brk = AdvancedBreakpoint::parse_remu("EXEC,addr=0x4000,size=3").unwrap();
        assert!(
            brk.hit_by_execution(0x4002)
                && !brk.hit_by_execution(0x4003)
                && !brk.hit_by_execution(0x3fff)
        );
    }
}

/// A breakpoint condition: an expression on the machine state, true when
/// not 0.
///
/// The language is ACE's (see <https://roudoudou.com/ACE-DL/breakpoints.html>),
/// which basm, rasm and ACE write in snapshots, written like BASIC; the C
/// spellings of the same operators are accepted too. Case-insensitive.
///
/// - numbers: decimal, hexadecimal with `&`, `#`, `$`, `0x` or a trailing
///   `h`, binary with `%` or `&x`;
/// - the registers (`A`..`L`, `F`, `I`, `R`, `IXh`/`IXl`/`IYh`/`IYl`, `AF`,
///   `BC`, `DE`, `HL`, `IX`, `IY`, `SP`, `PC`, the alternate set `A'`..`HL'`),
///   `IFF1`, `IFF2`, `IM`, the flags `FlagZ`, `FlagNZ`, `FlagC`, `FlagNC`,
///   `FlagP`, `FlagM`, `FlagPO`, `FlagPE` (1 or 0);
/// - the CRTC: its counters `C0` (character), `C3H` (HSYNC width), `C3V`
///   (VSYNC height), `C4` (character row), `C5` (adjust), `C9` (raster line
///   in the row), its registers `CrtcR0`..`CrtcR17` and `CrtcSel`;
/// - the beam: `LineCount` (the raster line) and `LineCycle` (the
///   microsecond in it, 0-63);
/// - the memory: `MemPage` (the 64K expansion page), `MemBankMode` (the 16K
///   bank mapping mode, 0-7), `LowerROMOn`, `UpperROMOn`, `UpperROMNum`,
///   and for the Plus `LowerROMNum`, `LowerROMAdr`, `ASICPageOn` (0 here)
///   and `ASICPageOff` (1: no ASIC page is ever mapped);
/// - for memory/IO breakpoints the access's `ADDRESS` (`ADDR`) and `VALUE`;
/// - `peek(x)` / `deek(x)`: the byte / little-endian word at `x`, as the CPU
///   sees memory.
///
/// Operators, loosest first (BASIC's order, the C ones slotted in):
/// `||`; `&&`; `XOR`; `OR`; `AND`; `NOT`; `|`; `&`; `=` `==` `<>` `!=`;
/// `<` `<=` `>` `>=`; `<<` `>>`; `+` `-`; `MOD` `%`; `\` (integer
/// division); `*` `/`; the unary `-` `+` `!` `~`; `^` (power); parentheses.
///
/// As in BASIC, a comparison is -1 when true and 0 when false, and `AND`,
/// `OR`, `XOR` and `NOT` work on the bits - so `NOT (A = 1)` and
/// `A = 1 AND B = 2` mean what they say. `NOT` binds looser than the
/// comparisons (`NOT A = 1` is `NOT (A = 1)`); `^` is a power, not an
/// exclusive or (that is `XOR`). As in C, `&` and `|` bind looser than the
/// comparisons: `value & 0xC0 == 0x80` is `value & (0xC0 == 0x80)`, write
/// `(value & 0xC0) == 0x80`.
///
/// Arithmetic is on 64-bit integers; a division by 0 gives 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    source: String,
    expr: CondExpr,
}

#[derive(Debug, Clone, PartialEq)]
enum CondExpr {
    Number(i64),
    Variable(&'static str),
    Unary(char, Box<CondExpr>),
    Binary(&'static str, Box<CondExpr>, Box<CondExpr>),
    Peek(Box<CondExpr>, bool),
}

/// What a condition reads.
pub trait ConditionContext {
    /// A variable among [`VARIABLES`] (upper case); `None` when it has no
    /// value here (`VALUE` outside a memory/IO access).
    fn variable(&self, name: &str) -> Option<i64>;
    fn peek(&self, address: u16) -> u8;
}

/// The variables a condition can name.
pub const VARIABLES: &[&str] = &[
    "A",
    "F",
    "B",
    "C",
    "D",
    "E",
    "H",
    "L",
    "I",
    "R",
    "IXH",
    "IXL",
    "IYH",
    "IYL",
    "AF",
    "BC",
    "DE",
    "HL",
    "IX",
    "IY",
    "SP",
    "PC",
    "A'",
    "F'",
    "B'",
    "C'",
    "D'",
    "E'",
    "H'",
    "L'",
    "AF'",
    "BC'",
    "DE'",
    "HL'",
    "IFF1",
    "IFF2",
    "IM",
    "ADDRESS",
    "VALUE",
    "FLAGZ",
    "FLAGNZ",
    "FLAGC",
    "FLAGNC",
    "FLAGP",
    "FLAGM",
    "FLAGPO",
    "FLAGPE",
    "C0",
    "C3H",
    "C3V",
    "C4",
    "C5",
    "C9",
    "CRTCR0",
    "CRTCR1",
    "CRTCR2",
    "CRTCR3",
    "CRTCR4",
    "CRTCR5",
    "CRTCR6",
    "CRTCR7",
    "CRTCR8",
    "CRTCR9",
    "CRTCR10",
    "CRTCR11",
    "CRTCR12",
    "CRTCR13",
    "CRTCR14",
    "CRTCR15",
    "CRTCR16",
    "CRTCR17",
    "CRTCSEL",
    "LINECOUNT",
    "LINECYCLE",
    "MEMPAGE",
    "MEMBANKMODE",
    "LOWERROMON",
    "UPPERROMON",
    "LOWERROMNUM",
    "UPPERROMNUM",
    "LOWERROMADR",
    "ASICPAGEON",
    "ASICPAGEOFF",
];

impl Condition {
    pub fn parse(source: &str) -> Result<Self, String> {
        let tokens = tokenize(source)?;
        let mut parser = Parser {
            tokens,
            position: 0,
        };
        let expr = parser.expression(0)?;
        if let Some(token) = parser.tokens.get(parser.position) {
            return Err(format!("condition {source:?}: unexpected {token:?}"));
        }
        Ok(Self {
            source: source.to_owned(),
            expr,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether the condition holds (a variable without a value is 0).
    pub fn holds(&self, context: &dyn ConditionContext) -> bool {
        evaluate(&self.expr, context) != 0
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(i64),
    Name(String),
    Operator(&'static str),
    Open,
    Close,
}

/// Operators, longest first so that `<=` wins over `<`.
const OPERATORS: &[&str] = &[
    "||", "&&", "==", "!=", "<>", "<=", ">=", "<<", ">>", "|", "^", "&", "=", "<", ">", "+", "-",
    "*", "/", "\\", "%", "!", "~",
];

/// The operators written as words, as BASIC writes them.
const WORD_OPERATORS: &[&str] = &["AND", "OR", "XOR", "NOT", "MOD"];

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    while i < chars.len() {
        let c = chars[i];
        let value_expected = !matches!(
            tokens.last(),
            Some(Token::Number(_) | Token::Name(_) | Token::Close)
        );
        if c.is_whitespace() {
            i += 1;
        } else if c == '(' || c == ')' {
            tokens.push(if c == '(' { Token::Open } else { Token::Close });
            i += 1;
        } else if value_expected
            && c == '&'
            && matches!(chars.get(i + 1), Some('x' | 'X'))
            && chars.get(i + 2).is_some_and(|d| matches!(d, '0' | '1'))
        {
            let start = i + 2;
            i = start;
            while i < chars.len() && matches!(chars[i], '0' | '1') {
                i += 1;
            }
            let digits: String = chars[start..i].iter().collect();
            tokens.push(Token::Number(i64::from_str_radix(&digits, 2).map_err(
                |_| format!("condition {source:?}: bad number &x{digits}"),
            )?));
        } else if value_expected
            && (c == '#' || c == '&' || c == '$' || c == '%')
            && chars.get(i + 1).is_some_and(|d| d.is_ascii_hexdigit())
        {
            let radix = if c == '%' { 2 } else { 16 };
            let start = i + 1;
            i = start;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            let digits: String = chars[start..i].iter().collect();
            tokens.push(Token::Number(i64::from_str_radix(&digits, radix).map_err(
                |_| format!("condition {source:?}: bad number {c}{digits}"),
            )?));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && is_name_char(chars[i]) {
                i += 1;
            }
            let text: String = chars[start..i]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase();
            let value = if let Some(hex) = text.strip_prefix("0x") {
                i64::from_str_radix(hex, 16).ok()
            } else if let Some(hex) = text.strip_suffix('h') {
                i64::from_str_radix(hex, 16).ok()
            } else {
                text.parse().ok()
            };
            tokens.push(Token::Number(value.ok_or_else(|| {
                format!("condition {source:?}: bad number {text}")
            })?));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && is_name_char(chars[i]) {
                i += 1;
            }
            if chars.get(i) == Some(&'\'') {
                i += 1;
            }
            let name = chars[start..i]
                .iter()
                .collect::<String>()
                .to_ascii_uppercase();
            match WORD_OPERATORS.iter().find(|word| **word == name) {
                Some(word) => tokens.push(Token::Operator(word)),
                None => tokens.push(Token::Name(name)),
            }
        } else {
            let rest: String = chars[i..].iter().collect();
            let operator = OPERATORS
                .iter()
                .find(|op| rest.starts_with(**op))
                .ok_or_else(|| format!("condition {source:?}: unexpected {c:?}"))?;
            tokens.push(Token::Operator(operator));
            i += operator.chars().count();
        }
    }
    Ok(tokens)
}

/// Binary operators by binding strength (index = level, loosest first).
/// `NOT` (a prefix) sits at its own level, [`NOT_LEVEL`].
const LEVELS: &[&[&str]] = &[
    &["||"],
    &["&&"],
    &["XOR"],
    &["OR"],
    &["AND"],
    &["NOT"],
    &["|"],
    &["&"],
    &["==", "=", "!=", "<>"],
    &["<", "<=", ">", ">="],
    &["<<", ">>"],
    &["+", "-"],
    &["MOD", "%"],
    &["\\"],
    &["*", "/"],
];

/// BASIC's `NOT`: looser than the comparisons, tighter than `AND`.
const NOT_LEVEL: usize = 5;

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn expression(&mut self, level: usize) -> Result<CondExpr, String> {
        if level == LEVELS.len() {
            return self.unary();
        }
        if level == NOT_LEVEL {
            if self.tokens.get(self.position) == Some(&Token::Operator("NOT")) {
                self.position += 1;
                let operand = self.expression(NOT_LEVEL)?;
                return Ok(CondExpr::Unary('N', Box::new(operand)));
            }
            return self.expression(level + 1);
        }
        let mut left = self.expression(level + 1)?;
        while let Some(Token::Operator(op)) = self.tokens.get(self.position) {
            let Some(op) = LEVELS[level].iter().find(|o| *o == op) else {
                break;
            };
            self.position += 1;
            let right = self.expression(level + 1)?;
            left = CondExpr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// The unary operators, then `^`: tighter than them, as in BASIC
    /// (`-2^2` is -4).
    fn unary(&mut self) -> Result<CondExpr, String> {
        if let Some(Token::Operator(op @ ("!" | "~" | "-" | "+"))) =
            self.tokens.get(self.position).cloned()
        {
            self.position += 1;
            let operand = self.unary()?;
            return Ok(if op == "+" {
                operand
            } else {
                CondExpr::Unary(op.chars().next().unwrap(), Box::new(operand))
            });
        }
        let mut left = self.primary()?;
        while self.tokens.get(self.position) == Some(&Token::Operator("^")) {
            self.position += 1;
            let right = self.primary()?;
            left = CondExpr::Binary("^", Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn primary(&mut self) -> Result<CondExpr, String> {
        match self.next() {
            Some(Token::Number(n)) => Ok(CondExpr::Number(n)),
            Some(Token::Open) => {
                let inner = self.expression(0)?;
                match self.next() {
                    Some(Token::Close) => Ok(inner),
                    _ => Err("missing ')'".to_owned()),
                }
            }
            Some(Token::Name(name)) if name == "PEEK" || name == "DEEK" => {
                if self.next() != Some(Token::Open) {
                    return Err(format!("{name} needs '('"));
                }
                let address = self.expression(0)?;
                if self.next() != Some(Token::Close) {
                    return Err(format!("{name}( needs ')'"));
                }
                Ok(CondExpr::Peek(Box::new(address), name == "DEEK"))
            }
            Some(Token::Name(name)) => {
                let name = match name.as_str() {
                    "ADDR" => "ADDRESS".to_owned(),
                    // As ACE's own documentation spells it.
                    "LOWERRROMON" => "LOWERROMON".to_owned(),
                    _ => name,
                };
                VARIABLES
                    .iter()
                    .find(|v| **v == name)
                    .map(|v| CondExpr::Variable(v))
                    .ok_or_else(|| {
                        format!("unknown name {name} (registers, flags, CRTC, LineCount, MemPage..., ADDRESS, VALUE, peek, deek)")
                    })
            }
            other => Err(format!("expected a value, found {other:?}")),
        }
    }
}

/// BASIC's truth: -1, every bit set, so that `AND`, `OR` and `NOT` on bits
/// are also the logical ones.
fn truth(holds: bool) -> i64 {
    if holds {
        -1
    } else {
        0
    }
}

/// `l ^ r` on integers; a negative power is 0 (or 1 / -1 for a base of 1 / -1).
fn power(l: i64, r: i64) -> i64 {
    match (l, r) {
        (1, _) => 1,
        (-1, _) => {
            if r % 2 == 0 {
                1
            } else {
                -1
            }
        }
        (_, r) if r < 0 => 0,
        _ => l.wrapping_pow(r.min(u32::MAX as i64) as u32),
    }
}

fn evaluate(expr: &CondExpr, context: &dyn ConditionContext) -> i64 {
    match expr {
        CondExpr::Number(n) => *n,
        CondExpr::Variable(name) => context.variable(name).unwrap_or(0),
        CondExpr::Peek(address, word) => {
            let address = evaluate(address, context) as u16;
            let low = context.peek(address) as i64;
            if *word {
                low | (context.peek(address.wrapping_add(1)) as i64) << 8
            } else {
                low
            }
        }
        CondExpr::Unary(op, operand) => {
            let v = evaluate(operand, context);
            match op {
                '!' => truth(v == 0),
                '~' | 'N' => !v,
                _ => v.wrapping_neg(),
            }
        }
        CondExpr::Binary(op, left, right) => {
            let (l, r) = (evaluate(left, context), evaluate(right, context));
            match *op {
                "||" => truth(l != 0 || r != 0),
                "&&" => truth(l != 0 && r != 0),
                "|" | "OR" => l | r,
                "XOR" => l ^ r,
                "&" | "AND" => l & r,
                "^" => power(l, r),
                "==" | "=" => truth(l == r),
                "!=" | "<>" => truth(l != r),
                "<" => truth(l < r),
                "<=" => truth(l <= r),
                ">" => truth(l > r),
                ">=" => truth(l >= r),
                "<<" => l.wrapping_shl(r as u32),
                ">>" => l.wrapping_shr(r as u32),
                "+" => l.wrapping_add(r),
                "-" => l.wrapping_sub(r),
                "*" => l.wrapping_mul(r),
                "/" | "\\" => {
                    if r == 0 {
                        0
                    } else {
                        l.wrapping_div(r)
                    }
                }
                _ => {
                    if r == 0 {
                        0
                    } else {
                        l.wrapping_rem(r)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod condition_test {
    use super::*;

    struct Fake;

    impl ConditionContext for Fake {
        fn variable(&self, name: &str) -> Option<i64> {
            match name {
                "A" => Some(0x12),
                "HL" => Some(0xc000),
                "A'" => Some(7),
                "VALUE" => Some(0x80),
                _ => None,
            }
        }

        fn peek(&self, address: u16) -> u8 {
            (address & 0xff) as u8
        }
    }

    fn holds(source: &str) -> bool {
        Condition::parse(source)
            .unwrap_or_else(|e| panic!("{e}"))
            .holds(&Fake)
    }

    #[test]
    fn syntaxes_and_precedence() {
        assert!(holds("A == #12"));
        assert!(holds("a=&12 && hl >= 0xC000"));
        assert!(holds("HL = 0C000h"));
        assert!(holds("A' = 7"));
        assert!(holds("1 + 2 * 3 == 7"));
        assert!(holds("(1 + 2) * 3 == 9"));
        assert!(holds("value & %10000000"));
        assert!(holds("peek(HL + 5) == 5 && deek(0x0102) == 0x0302"));
        assert!(holds("!(A <> $12)"));
        assert!(holds("A > 1 || 1 / 0"));
        assert!(!holds("~0 == 0"));
        assert!(holds("-1 < 0 && 7 % 4 == 3 && 1 << 4 == 16"));
    }

    fn value(source: &str) -> i64 {
        evaluate(
            &Condition::parse(source)
                .unwrap_or_else(|e| panic!("{e}"))
                .expr,
            &Fake,
        )
    }

    /// ACE's language (roudoudou.com/ACE-DL/breakpoints.html): BASIC-like.
    #[test]
    fn ace_conditions() {
        // Numbers: &, #, 0x hexadecimal; %, &x binary.
        assert_eq!(value("&x101 + %11 + &10 + #10 + 0x10"), 5 + 3 + 48);
        // BASIC truth and word operators, on the bits.
        assert_eq!(value("A = &12"), -1);
        assert_eq!(value("A = 1"), 0);
        assert!(holds("A = &12 AND HL = &C000"));
        assert!(holds("A = 1 OR HL = &C000"));
        assert!(holds("NOT A = 1"), "NOT is looser than the comparison");
        assert!(holds("NOT (A = 1) AND NOT (HL <> &C000)"));
        assert!(!holds("A = &12 XOR HL = &C000"));
        assert_eq!(value("NOT 0"), -1);
        assert_eq!(value("6 AND 3"), 2);
        assert_eq!(value("6 OR 3"), 7);
        assert_eq!(value("6 XOR 3"), 5);
        // Arithmetic: ^ is a power, \ integer division, MOD.
        assert_eq!(value("2 ^ 3 ^ 2"), 64, "left to right, as in BASIC");
        assert_eq!(value("-2 ^ 2"), -4, "^ before the unary minus");
        assert_eq!(value("2 * 3 ^ 2"), 18);
        assert_eq!(value("7 \\ 2 + 7 MOD 4"), 6);
        assert_eq!(value("17 MOD 5 * 2"), 7, "MOD looser than *: 17 MOD 10");
        assert_eq!(value("1 + 2 = 3 AND 2 < 3"), -1);
        // The C spellings still work, with C's ^ gone to XOR.
        assert!(holds("a == #12 && !(hl != 0xc000)"));
        // Names are case-insensitive, ACE's misspelling included.
        for name in [
            "FlagZ",
            "flagnz",
            "C0",
            "c3h",
            "C3V",
            "CrtcR12",
            "CRTCSEL",
            "LineCount",
            "linecycle",
            "MemPage",
            "MemBankMode",
            "LowerROMOn",
            "LowerrROMOn",
            "UpperROMNum",
            "ASICPageOn",
            "ASICPageOff",
            "IXl",
            "iyh",
        ] {
            assert!(Condition::parse(&format!("{name} >= 0")).is_ok(), "{name}");
        }
    }

    #[test]
    fn errors() {
        assert!(Condition::parse("A ==").is_err());
        assert!(Condition::parse("BOGUS == 1").is_err());
        assert!(Condition::parse("(A == 1").is_err());
        assert!(Condition::parse("A == 1 )").is_err());
        assert!(Condition::parse("A @ 1").is_err());
    }
}
