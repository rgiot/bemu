//! When, within an instruction, do I/O read/write cycles happen? Runs a few
//! instructions from RAM and prints the tick (16 per microsecond) of IORQ+RD /
//! IORQ+WR relative to the instruction start.
//!
//! `cargo run --release --example debug_io_timing`

use amsrust::chip::{Activable, Chip};
use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{Reg16, Z80Pin};

fn main() {
    let programs: &[(&str, &[u8])] = &[
        ("IN A,(C)    ED 78", &[0xed, 0x78]),
        ("IN A,(n)       DB f5", &[0xdb, 0xf5]),
        ("OUT (C),A   ED 79", &[0xed, 0x79]),
        ("OUT (C),D   ED 51", &[0xed, 0x51]),
        ("OUT (n),A      D3 7f", &[0xd3, 0x7f]),
        ("OUTI        ED A3", &[0xed, 0xa3]),
        ("INI         ED A2", &[0xed, 0xa2]),
    ];
    for (name, bytes) in programs {
        let mut mb = MotherboardBuilder::default().disable_lower_rom().disable_upper_rom().build();
        mb.write_buffer(0x4000, bytes);
        mb.write_buffer(0x4004, &[0, 0, 0, 0, 0, 0, 0, 0]);
        mb.cpu_mut().reg16_mut(amsrust::z80::Reg16::Pc).set(0x4000);
        mb.cpu_mut().reg16_mut(Reg16::Bc).set(0xbc00 | 0x00f5);
        mb.cpu_mut().reg16_mut(Reg16::Hl).set(0x5000);
        use amsrust::z80::HasValue;
        // align on an instruction boundary then count ticks until PC moves
        let mut t = 0u32;
        let mut events = Vec::new();
        // run until the CPU is back on an instruction boundary
        loop {
            mb.tick();
            t += 1;
            if t > 400 || (t > 4 && mb.cpu().is_activated() && mb.is_aligned_with_new_instruction()) {
                break;
            }
            let iorq = !mb.cpu().get_pin(Z80Pin::Neg_IORQ);
            let rd = !mb.cpu().get_pin(Z80Pin::Neg_RD);
            let wr = !mb.cpu().get_pin(Z80Pin::Neg_WR);
            let m1 = !mb.cpu().get_pin(Z80Pin::Neg_M1);
            if iorq && !m1 && (rd || wr) {
                events.push((t, if rd { "RD" } else { "WR" }));
            }
        }
        let first = events.first().map(|e| e.0);
        let last = events.last().map(|e| e.0);
        println!("{name:28} total {t:3} ticks ({:.2} us); IO {}: first tick {:?} ({:.2} us) last {:?}", t as f32 / 16.0, events.first().map(|e| e.1).unwrap_or("-"), first, first.unwrap_or(0) as f32 / 16.0, last);
    }
}
