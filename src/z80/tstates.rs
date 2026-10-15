//! Where a Z80 instruction really ends, in T-states, once the Gate Array has
//! aligned every bus cycle to a microsecond (ACCC 4.4.2).
//!
//! The CPC lengthens an instruction to a whole number of microseconds, but the
//! Z80A's own last T-state comes earlier when the instruction ends in internal
//! cycles (DEC HL: 6 T-states in a 2 us slot). The Gate Array sees the end of
//! the instruction - the start of an interrupt acknowledge - on that T-state,
//! which decides on which side of the R52 step the acknowledge falls
//! (SHAKER BR/A).

use super::states::OpcodePrefix;

/// One piece of an instruction: a bus cycle (which the Gate Array aligns on a
/// microsecond when it starts) or internal T-states.
#[derive(Clone, Copy)]
enum Cycle {
    Bus(u8),
    Internal(u8),
}
use Cycle::{Bus as B, Internal as I};

fn unprefixed(op: u8, taken: bool) -> Vec<Cycle> {
    let r = op & 7;
    let y = (op >> 3) & 7;
    match op {
        0x01 | 0x11 | 0x21 | 0x31 => vec![B(4), B(3), B(3)],
        0x02 | 0x12 | 0x0a | 0x1a => vec![B(4), B(3)],
        0x03 | 0x13 | 0x23 | 0x33 | 0x0b | 0x1b | 0x2b | 0x3b => vec![B(4), I(2)],
        0x34 | 0x35 => vec![B(4), B(3), I(1), B(3)],
        0x36 => vec![B(4), B(3), B(3)],
        0x09 | 0x19 | 0x29 | 0x39 => vec![B(4), I(7)],
        0x10 => if taken { vec![B(4), I(1), B(3), I(5)] } else { vec![B(4), I(1), B(3)] },
        0x18 => vec![B(4), B(3), I(5)],
        0x20 | 0x28 | 0x30 | 0x38 => if taken { vec![B(4), B(3), I(5)] } else { vec![B(4), B(3)] },
        0x22 | 0x2a => vec![B(4), B(3), B(3), B(3), B(3)],
        0x32 | 0x3a => vec![B(4), B(3), B(3), B(3)],
        0x76 => vec![B(4)],
        0x40..=0x7f => if r == 6 || y == 6 { vec![B(4), B(3)] } else { vec![B(4)] },
        0x80..=0xbf => if r == 6 { vec![B(4), B(3)] } else { vec![B(4)] },
        0xc0 | 0xc8 | 0xd0 | 0xd8 | 0xe0 | 0xe8 | 0xf0 | 0xf8 =>
            if taken { vec![B(4), I(1), B(3), B(3)] } else { vec![B(4), I(1)] },
        0xc1 | 0xd1 | 0xe1 | 0xf1 | 0xc9 => vec![B(4), B(3), B(3)],
        0xc3 => vec![B(4), B(3), B(3)],
        0xcd => vec![B(4), B(3), B(3), I(1), B(3), B(3)],
        0xc5 | 0xd5 | 0xe5 | 0xf5 => vec![B(4), I(1), B(3), B(3)],
        0xd3 | 0xdb => vec![B(4), B(3), B(4)],
        0xe3 => vec![B(4), B(3), B(3), I(1), B(3), B(3), I(2)],
        0xf9 => vec![B(4), I(2)],
        _ if op >= 0xc0 && r == 2 => vec![B(4), B(3), B(3)],
        _ if op >= 0xc0 && r == 4 => if taken { vec![B(4), B(3), B(3), I(1), B(3), B(3)] } else { vec![B(4), B(3), B(3)] },
        _ if op >= 0xc0 && r == 6 => vec![B(4), B(3)],
        _ if op >= 0xc0 && r == 7 => vec![B(4), I(1), B(3), B(3)],
        _ if op < 0x40 && r == 6 => vec![B(4), B(3)],
        _ => vec![B(4)],
    }
}

fn cb(op: u8) -> Vec<Cycle> {
    if op & 7 == 6 {
        if op >> 6 == 1 { vec![B(4), B(4), B(3), I(1)] } else { vec![B(4), B(4), B(3), I(1), B(3)] }
    } else {
        vec![B(4), B(4)]
    }
}

fn ed(op: u8, repeat: bool) -> Vec<Cycle> {
    let r = op & 7;
    match op {
        0x40..=0x7f => match r {
            0 | 1 => vec![B(4), B(4), B(4)],
            2 => vec![B(4), B(4), I(7)],
            3 => vec![B(4), B(4), B(3), B(3), B(3), B(3)],
            5 => vec![B(4), B(4), B(3), B(3)],
            7 if matches!(op, 0x47 | 0x4f | 0x57 | 0x5f) => vec![B(4), B(4), I(1)],
            7 if matches!(op, 0x67 | 0x6f) => vec![B(4), B(4), B(3), I(4), B(3)],
            _ => vec![B(4), B(4)],
        },
        0xa0 | 0xa8 | 0xb0 | 0xb8 => {
            let mut v = vec![B(4), B(4), B(3), B(3), I(2)];
            if repeat { v.push(I(5)); }
            v
        }
        0xa1 | 0xa9 | 0xb1 | 0xb9 => {
            let mut v = vec![B(4), B(4), B(3), I(5)];
            if repeat { v.push(I(5)); }
            v
        }
        0xa2 | 0xaa | 0xa3 | 0xab | 0xb2 | 0xba | 0xb3 | 0xbb => {
            let mut v = vec![B(4), B(4), I(1), B(3), B(4)];
            if repeat { v.push(I(5)); }
            v
        }
        _ => vec![B(4), B(4)],
    }
}

/// DD/FD prefixed instruction
fn indexed(op: u8, taken: bool, repeat: bool) -> Vec<Cycle> {
    let r = op & 7;
    let y = (op >> 3) & 7;
    match op {
        0x21 => vec![B(4), B(4), B(3), B(3)],
        0x22 | 0x2a => vec![B(4), B(4), B(3), B(3), B(3), B(3)],
        0x23 | 0x2b | 0xf9 => vec![B(4), B(4), I(2)],
        0x09 | 0x19 | 0x29 | 0x39 => vec![B(4), B(4), I(7)],
        0x34 | 0x35 => vec![B(4), B(4), B(3), I(5), B(3), I(1), B(3)],
        0x36 => vec![B(4), B(4), B(3), B(3), I(2), B(3)],
        0x26 | 0x2e => vec![B(4), B(4), B(3)],
        0x40..=0x7f if op != 0x76 && (r == 6 || y == 6) => vec![B(4), B(4), B(3), I(5), B(3)],
        0x80..=0xbf if r == 6 => vec![B(4), B(4), B(3), I(5), B(3)],
        0xe1 => vec![B(4), B(4), B(3), B(3)],
        0xe5 => vec![B(4), B(4), I(1), B(3), B(3)],
        0xe3 => vec![B(4), B(4), B(3), B(3), I(1), B(3), B(3), I(2)],
        0xe9 => vec![B(4), B(4)],
        0x24 | 0x25 | 0x2c | 0x2d | 0x40..=0xbf => vec![B(4), B(4)],
        _ => {
            // not touching HL: the prefix is an extra M1, then the plain instruction
            let mut v = vec![B(4)];
            v.extend(match op {
                0xed => ed(op, repeat),
                _ => unprefixed(op, taken),
            });
            v
        }
    }
}

fn indexed_cb(op: u8) -> Vec<Cycle> {
    if op >> 6 == 1 {
        vec![B(4), B(4), B(3), B(3), I(2), B(3), I(1)]
    } else {
        vec![B(4), B(4), B(3), B(3), I(2), B(3), I(1), B(3)]
    }
}

/// T-states left idle at the end of the last microsecond of the instruction:
/// 0 to 3. `taken` is for conditional branches, `repeat` for the block
/// instructions that go round again.
pub(crate) fn idle_tail(prefix: OpcodePrefix, op: u8, taken: bool, repeat: bool) -> u8 {
    let cycles = match prefix {
        OpcodePrefix::None => unprefixed(op, taken),
        OpcodePrefix::Cb => cb(op),
        OpcodePrefix::Ed => ed(op, repeat),
        OpcodePrefix::Dd | OpcodePrefix::Fd => indexed(op, taken, repeat),
        OpcodePrefix::DdCb | OpcodePrefix::FdCb => indexed_cb(op),
    };
    let mut offset = 0u32;
    for c in cycles {
        match c {
            B(n) => offset = ((offset + 3) & !3) + u32::from(n),
            I(n) => offset += u32::from(n),
        }
    }
    (((offset + 3) & !3) - offset) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle(prefix: OpcodePrefix, op: u8) -> u8 {
        idle_tail(prefix, op, false, false)
    }

    #[test]
    fn instructions_ending_with_two_or_more_idle_t_states() {
        // SHAKER BR/A on a real CPC: DEC HL, DEC IY, LD A,I and LD A,R are
        // acknowledged in their last microsecond (#C4)...
        assert_eq!(idle(OpcodePrefix::None, 0x2b), 2);
        assert_eq!(idle(OpcodePrefix::Fd, 0x2b), 2);
        assert_eq!(idle(OpcodePrefix::Ed, 0x57), 3);
        assert_eq!(idle(OpcodePrefix::Ed, 0x5f), 3);
    }

    #[test]
    fn instructions_ending_with_at_most_one_idle_t_state() {
        // ...while these, #CC, are acknowledged one microsecond later
        assert_eq!(idle(OpcodePrefix::None, 0x00), 0);
        assert_eq!(idle(OpcodePrefix::None, 0x83), 0);
        assert_eq!(idle(OpcodePrefix::Ed, 0x62), 1); // SBC HL,HL
        assert_eq!(idle(OpcodePrefix::Cb, 0x86), 1); // RES 0,(HL)
        assert_eq!(idle(OpcodePrefix::Fd, 0x70), 1); // LD (IY+d),B
        assert_eq!(idle(OpcodePrefix::Ed, 0x44), 0); // NEG
    }

    #[test]
    fn indexed_cb_is_longer_than_the_plain_count() {
        // ACCC 29: SET n,(IX+n') officially 23 T-states, 27 or 28 on a CPC
        assert_eq!(idle(OpcodePrefix::DdCb, 0xc6), 1);
    }
}
