//! The duration (in microseconds) of every opcode matches ACCC chapter 26's
//! table ("duration of instructions on the CPC").
#![feature(exclusive_range_pattern)]
#![feature(half_open_range_patterns)]

use amsrust::{motherboard::MotherboardBuilder, z80::{HasValue, Reg16}};

/// (min, max) expected durations; None when not covered by the checks
fn expected(prefix: u8, op: u8) -> Option<(u8, u8)> {
    let r = op & 7;
    let y = (op >> 3) & 7;
    let x = op >> 6;
    let same = |a: u8| Some((a, a));
    match prefix {
        0x00 => match op {
            0x76 | 0x00 | 0xf3 | 0xfb | 0xd9 | 0xeb | 0x08 | 0x3f | 0x37 | 0x2f | 0x27 | 0x07 | 0x0f | 0x17 | 0x1f | 0xe9 => same(1),
            0x01 | 0x11 | 0x21 | 0x31 => same(3),
            0x02 | 0x12 | 0x0a | 0x1a => same(2),
            0x03 | 0x13 | 0x23 | 0x33 | 0x0b | 0x1b | 0x2b | 0x3b => same(2),
            0x09 | 0x19 | 0x29 | 0x39 => same(3),
            0x04 | 0x0c | 0x14 | 0x1c | 0x24 | 0x2c | 0x3c | 0x05 | 0x0d | 0x15 | 0x1d | 0x25 | 0x2d | 0x3d => same(1),
            0x34 | 0x35 => same(3),
            0x06 | 0x0e | 0x16 | 0x1e | 0x26 | 0x2e | 0x3e => same(2),
            0x36 => same(3),
            0x10 => Some((3, 4)),
            0x18 => same(3),
            0x20 | 0x28 | 0x30 | 0x38 => Some((2, 3)),
            0x22 | 0x2a => same(5),
            0x32 | 0x3a => same(4),
            0x40..=0x7f => if r == 6 || y == 6 { same(2) } else { same(1) },
            0x80..=0xbf => if r == 6 { same(2) } else { same(1) },
            0xc0 | 0xc8 | 0xd0 | 0xd8 | 0xe0 | 0xe8 | 0xf0 | 0xf8 => Some((2, 4)),
            0xc1 | 0xd1 | 0xe1 | 0xf1 => same(3),
            0xc5 | 0xd5 | 0xe5 | 0xf5 => same(4),
            0xc9 => same(3),
            0xc3 => same(3),
            0xcd => same(5),
            0xc2 | 0xca | 0xd2 | 0xda | 0xe2 | 0xea | 0xf2 | 0xfa => same(3),
            0xc4 | 0xcc | 0xd4 | 0xdc | 0xe4 | 0xec | 0xf4 | 0xfc => Some((3, 5)),
            0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => same(2),
            0xc7 | 0xcf | 0xd7 | 0xdf | 0xe7 | 0xef | 0xf7 | 0xff => same(4),
            0xe3 => same(6),
            0xf9 => same(2),
            0xd3 | 0xdb => same(3),
            _ => None,
        },
        0xcb => if r == 6 { if x == 1 { same(3) } else { same(4) } } else { same(2) },
        0xed => match op {
            0x40..=0x7f => match r {
                0 | 1 => same(4),
                2 => same(4),
                3 => same(6),
                4 => same(2),
                5 => same(4),
                6 => same(2),
                7 => if y == 4 || y == 5 { same(5) } else if y < 4 { same(3) } else { same(2) },
                _ => None,
            },
            0xa0 | 0xa8 => same(5),
            0xa1 | 0xa9 => same(4),
            0xa2 | 0xaa | 0xa3 | 0xab => same(5),
            0xb0 | 0xb8 => Some((5, 6)),
            0xb1 | 0xb9 => Some((4, 6)),
            0xb2 | 0xba | 0xb3 | 0xbb => Some((5, 6)),
            _ => same(2),
        },
        0xdd | 0xfd => match op {
            0x21 => same(4),
            0x22 | 0x2a => same(6),
            0x23 | 0x2b => same(3),
            0x09 | 0x19 | 0x29 | 0x39 => same(4),
            0x34 | 0x35 => same(6),
            0x36 => same(6),
            0x24 | 0x25 | 0x2c | 0x2d => same(2),
            0x26 | 0x2e => same(3),
            0xf9 => same(3),
            0xe1 => same(4),
            0xe5 => same(5),
            0xe3 => same(7),
            0xe9 => same(2),
            0x40..=0x7f if op != 0x76 => {
                if r == 6 || y == 6 { same(5) } else if (r >= 4 && r <= 5) || (y >= 4 && y <= 5) { same(2) } else { same(2) }
            }
            0x80..=0xbf => if r == 6 { same(5) } else if r == 4 || r == 5 { same(2) } else { same(1 + 1) },
            _ => None,
        },
        _ => None,
    }
}


#[test]
fn every_opcode_lasts_what_the_accc_table_says() {
    let mut bad = vec![];
    for prefix in [0x00u8, 0xcb, 0xed, 0xdd, 0xfd, 0xdc, 0xfc] {
        for op in 0..=255u8 {
            if prefix == 0 && matches!(op, 0xcb | 0xed | 0xdd | 0xfd) {
                continue;
            }
            if matches!(prefix, 0xdd | 0xfd) && matches!(op, 0xcb | 0xed | 0xdd | 0xfd) {
                continue;
            }
            let mut durs = vec![];
            for f in [0x00u8, 0xff] {
                let mut mb = MotherboardBuilder::default().disable_lower_rom().disable_upper_rom().build();
                let bytes: Vec<u8> = match prefix {
                    0xdc => vec![0xdd, 0xcb, 0x00, op, 0, 0],
                    0xfc => vec![0xfd, 0xcb, 0x00, op, 0, 0],
                    0x00 => vec![op, 0x00, 0x40, 0, 0, 0],
                    p => vec![p, op, 0x00, 0x40, 0, 0],
                };
                mb.write_buffer(0x100, &bytes);
                mb.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);
                mb.cpu_mut().reg16_mut(Reg16::Af).set(u16::from(f));
                mb.cpu_mut().reg16_mut(Reg16::Bc).set(2);
                mb.cpu_mut().reg16_mut(Reg16::Hl).set(0x4000);
                mb.cpu_mut().reg16_mut(Reg16::Ix).set(0x4000);
                mb.cpu_mut().reg16_mut(Reg16::Iy).set(0x4000);
                mb.cpu_mut().reg16_mut(Reg16::Sp).set(0x8000);
                let t0 = mb.nops_counter().value();
                mb.execute_next_opcode();
                durs.push((mb.nops_counter().value() - t0) as u8);
            }
            let exp = match prefix {
                0xdc | 0xfc => Some(if op >> 6 == 1 { (6, 6) } else { (7, 7) }),
                p => expected(p, op),
            };
            if let Some((lo, hi)) = exp {
                if durs.iter().any(|d| *d < lo || *d > hi) {
                    bad.push(format!("prefix {prefix:02X} op {op:02X}: {durs:?}, expected {lo}..{hi}"));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
