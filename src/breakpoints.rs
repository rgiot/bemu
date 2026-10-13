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
//! Conditions (expressions on the registers) and step counts are not
//! evaluated yet: a breakpoint carrying one is refused rather than made
//! unconditional.

use crate::motherboard::{BusAccess, BusAccessKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointKind {
    Exec,
    Memory,
    Io
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointAccess {
    Read,
    Write,
    ReadWrite
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Pause the emulation.
    Stop,
    /// Only report the hit, keep running.
    Watch
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
    pub name: Option<String>
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
            name: None
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
            BusAccessKind::IoWrite => (BreakpointKind::Io, true)
        };
        let direction = match self.access {
            BreakpointAccess::Read => !write,
            BreakpointAccess::Write => write,
            BreakpointAccess::ReadWrite => true
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
            let Some((key, value)) = field.split_once('=')
            else {
                match field.to_ascii_uppercase().as_str() {
                    "EXEC" | "EXECUTE" => brk.kind = BreakpointKind::Exec,
                    "MEM" | "MEMORY" => brk.kind = BreakpointKind::Memory,
                    "IO" => brk.kind = BreakpointKind::Io,
                    "R" | "READ" => brk.access = BreakpointAccess::Read,
                    "W" | "WRITE" => brk.access = BreakpointAccess::Write,
                    "RW" | "READWRITE" => brk.access = BreakpointAccess::ReadWrite,
                    "STOP" | "STOPPER" => brk.run = RunMode::Stop,
                    "WATCH" | "WATCHER" => brk.run = RunMode::Watch,
                    other => return Err(format!("unknown breakpoint field {other:?}"))
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
                },
                "condition" if !value.trim_matches('"').is_empty() => {
                    return Err(format!("condition {value}: conditional breakpoints are not supported yet"))
                },
                "step" if !value.is_empty() => {
                    return Err(format!("step={value}: step counts are not supported yet"))
                },
                "condition" | "step" => {},
                other => return Err(format!("unknown breakpoint field {other:?}"))
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
            },
            ',' if !quoted => fields.push(String::new()),
            _ => fields.last_mut().unwrap().push(c)
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
        None => text.parse().ok()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn access(kind: BusAccessKind, address: u16, value: u8) -> BusAccess {
        BusAccess { kind, address, value }
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
        assert_eq!((brk.address, brk.size, brk.name.as_deref()), (0xc000, 16, Some("screen")));
        assert!(AdvancedBreakpoint::parse_remu(r#"EXEC,RW,STOP,addr=1,condition="A==2""#).is_err());
        assert!(AdvancedBreakpoint::parse_remu("BOGUS").is_err());
    }

    #[test]
    fn memory_accesses_through_mask_size_and_value() {
        let brk = AdvancedBreakpoint::parse_remu("MEM,W,STOP,addr=#C000,size=0x10,value=#80,valmask=#80").unwrap();
        assert!(brk.hit_by_access(&access(BusAccessKind::MemoryWrite, 0xc00f, 0xff)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::MemoryWrite, 0xc010, 0xff)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::MemoryWrite, 0xc000, 0x7f)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::MemoryRead, 0xc000, 0xff)));
        assert!(!brk.hit_by_access(&access(BusAccessKind::IoWrite, 0xc000, 0xff)));
        // A mask ignoring the high bits: every 16K window.
        let mirrored = AdvancedBreakpoint::parse_remu("MEM,RW,STOP,addr=0x0100,mask=0x3fff").unwrap();
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
        assert!(brk.hit_by_execution(0x4002) && !brk.hit_by_execution(0x4003) && !brk.hit_by_execution(0x3fff));
    }
}
