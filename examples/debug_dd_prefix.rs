//! Quick, direct verification of the DD/FD-prefix fallback fix
//! (`states.rs::build_opcode_information`): confirms `DD 41` (undocumented
//! "LD B,C" with an ignored DD prefix - doesn't touch H/L at all) really
//! transfers C into B, and that a multi-byte form (`DD 01 nnnn`, i.e.
//! `LD BC,nnnn`) correctly consumes its operand bytes and advances PC past
//! them, rather than treating the whole sequence as a single 1-byte NOP.
//! Found via zexall's "ld <bcdexya>,<bcdexya>" CRC failure - see
//! `project_crtc_timing_core_rewrite` memory.
//!
//! `cargo run --release --example debug_dd_prefix`

use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16, Reg8};

fn main() {
    // Case 1: DD 41 = "LD B,C" (undocumented, DD prefix ignored since
    // neither operand is H/L) - must transfer C into B.
    {
        let mut mb = MotherboardBuilder::default()
            .disable_lower_rom()
            .disable_upper_rom()
            .build();
        mb.write_buffer(0, &[0xdd, 0x41]);
        mb.cpu_mut().reg16_mut(Reg16::Pc).set(0);
        mb.cpu_mut().reg8_mut(Reg8::B).set(0x00);
        mb.cpu_mut().reg8_mut(Reg8::C).set(0x42);
        mb.execute_next_opcode_no_video();
        let b = mb.cpu().reg8(Reg8::B).value();
        let pc = mb.pc();
        eprintln!("DD 41 (LD B,C): B={b:02X} (expect 42), PC={pc:04X} (expect 0002)");
        assert_eq!(
            b, 0x42,
            "DD 41 should transfer C into B exactly like unprefixed LD B,C"
        );
        assert_eq!(
            pc, 2,
            "DD 41 is a 2-byte instruction (DD + opcode), nothing more"
        );
    }

    // Case 2: DD 01 nnnn = "LD BC,nnnn" (undocumented, DD prefix ignored)
    // - must consume both immediate bytes and load them into BC.
    {
        let mut mb = MotherboardBuilder::default()
            .disable_lower_rom()
            .disable_upper_rom()
            .build();
        mb.write_buffer(0, &[0xdd, 0x01, 0x34, 0x12]);
        mb.cpu_mut().reg16_mut(Reg16::Pc).set(0);
        mb.cpu_mut().reg16_mut(Reg16::Bc).set(0);
        mb.execute_next_opcode_no_video();
        let bc = mb.cpu().reg16(Reg16::Bc).value();
        let pc = mb.pc();
        eprintln!("DD 01 34 12 (LD BC,1234): BC={bc:04X} (expect 1234), PC={pc:04X} (expect 0004)");
        assert_eq!(bc, 0x1234, "DD 01 nnnn should load BC from the two immediate bytes exactly like unprefixed LD BC,nnnn");
        assert_eq!(
            pc, 4,
            "DD 01 nnnn is a 4-byte instruction (DD + opcode + 2 operand bytes)"
        );
    }

    eprintln!("\nAll checks passed.");
}
