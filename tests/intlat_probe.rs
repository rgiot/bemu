//! INTLAT (tools/intlat of CLAUDESTRAD, tests/fixtures/claudestrad/intlat/INTLAT.BIN): the loops
//! of 16 times one instruction run while 300 IM 2 interrupts come, and are counted. Run on a bare
//! machine, in seconds, instead of through the disc and BASIC (tests/claudestrad.rs, intlat).
//!
//! The real 6128's counts (photo of CRTC 1, tests/fixtures/claudestrad/truth/INTLAT_crtc1.txt):
//! NOP 30656, INC HL 20455, LD A,R 15342, LD (HL),A 20452, LD BC,&F600 15339, OUT (C),C 12271,
//! IN A,(C) 12271, EXX 30678, LDI 10228, EX (SP),HL 8767, PUSH BC/POP BC 13635, LD A,(nn) 12271.

use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};

const NAMES: [&str; 12] = [
    "NOP",
    "INC HL",
    "LD A,R",
    "LD (HL),A",
    "LD BC,&F600",
    "OUT (C),C",
    "IN A,(C)",
    "EXX",
    "LDI",
    "EX (SP),HL",
    "PUSH BC/POP BC",
    "LD A,(nn)",
];

/// The loop counts of the 12 measurements on a bare machine.
fn counts() -> [u16; 12] {
    let bin = include_bytes!("fixtures/claudestrad/intlat/INTLAT.BIN");
    let code = &bin[128..128 + 0x45a];
    let mut mb = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .enable_interruptions()
        .build();
    mb.write_buffer(0x8000, code);
    // where the program returns to
    mb.write_buffer(0x7000, &[0x18, 0xfe]);
    mb.write_buffer(0xbefe, &[0x00, 0x70]);
    mb.cpu_mut().reg16_mut(Reg16::Sp).set(0xbefe);
    mb.set_pc(0x8000);
    mb.set_video(false);
    let limit = 4_000_000usize * 40;
    let start = mb.t_state_counter().value();
    while mb.cpu().reg16(Reg16::Pc).value() != 0x7000
        && mb.t_state_counter().value() - start < limit
    {
        mb.execute_next_opcode_no_video();
    }
    let mut out = [0u16; 12];
    for (k, v) in out.iter_mut().enumerate() {
        let lo = mb.read_word(0x9000 + 2 * k as u16);
        *v = lo;
    }
    out
}

/// The real 6128's counts (CRTC 1), but for NOP, which depends on the CRTC type: 30656 on the
/// real CRTC 1, 30642 on the real CRTC 2, both of which a bare machine's loop meets within 16.
const REAL: [u16; 12] = [
    30656, 20455, 15342, 20452, 15339, 12271, 12271, 30678, 10228, 8767, 13635, 12271,
];

#[test]
fn the_loops_count_as_on_the_real_machine() {
    let c = counts();
    for ((name, real), got) in NAMES.iter().zip(REAL).zip(c) {
        // NOP's loop meets the 300th interrupt at another phase on each CRTC type
        let tolerance = if *name == "NOP" { 16 } else { 0 };
        assert!(
            got.abs_diff(real) <= tolerance,
            "{name}: {got} loops, the real machine's {real}"
        );
    }
}

#[test]
#[ignore = "prints the counts: cargo test --release --test intlat_probe -- --ignored --nocapture"]
fn print_the_counts() {
    let c = counts();
    for (name, v) in NAMES.iter().zip(c) {
        println!("{name:16} {v}");
    }
}
