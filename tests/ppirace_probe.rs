//! PPIRACE (tools/ppirace of CLAUDESTRAD, tests/fixtures/claudestrad/ppirace/PPIRACE.BIN): the main
//! program reads the keyboard through the PPI and the PSG in a loop while the interrupt routine
//! does the same: an interrupt in the middle of the main program's read leaves the PPI in the
//! state of the interrupt's read. Run on a bare machine instead of through the disc and BASIC.
//!
//! The real 6128 (photos): about 41340 reads of the main program, 3000 interrupts, about 300
//! false reads (the space key seen, bit 7 of the byte read clear), their values 00.

use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};

struct Result {
    reads: u32,
    interrupts: u16,
    false_reads: u16,
    values: Vec<u8>,
}

fn run() -> Result {
    let bin = include_bytes!("fixtures/claudestrad/ppirace/PPIRACE.BIN");
    let code = &bin[128..128 + 0xc6];
    let mut mb = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .enable_interruptions()
        .build();
    mb.write_buffer(0x8000, code);
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
    let n = mb.read_word(0x9000) as u32 | (mb.read_word(0x9002) as u32) << 16;
    let count = usize::from(mb.read_byte(0x9010));
    Result {
        reads: n,
        interrupts: mb.read_word(0x9006),
        false_reads: mb.read_word(0x9004),
        values: (0..count).map(|k| mb.read_byte(0x9008 + k as u16)).collect(),
    }
}

#[test]
fn the_races_happen_as_on_the_real_machine() {
    let r = run();
    assert_eq!(r.interrupts, 3000);
    assert!(r.reads.abs_diff(41339) <= 60, "{} reads", r.reads);
    assert!((290..=320).contains(&r.false_reads), "{} false reads", r.false_reads);
    // an interrupt between the main program's switch of the PPI to input and its read leaves port A an
    // output, the latch of which a mode set cleared: 00, never a key
    assert!(!r.values.is_empty() && r.values.iter().all(|&v| v == 0x00), "{:02X?}", r.values);
}

#[test]
#[ignore = "prints the result: cargo test --release --test ppirace_probe -- --ignored --nocapture"]
fn print_the_result() {
    let r = run();
    println!(
        "reads {} interrupts {} false reads {} values {:02X?}",
        r.reads, r.interrupts, r.false_reads, r.values
    );
}
