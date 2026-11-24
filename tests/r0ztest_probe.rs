//! R0ZTEST (tools/r0ztest of CLAUDESTRAD, tests/fixtures/claudestrad/r0ztest/R0ZTEST.BIN): R0=0 and
//! R0 shortened at the end of a line, measured as loop counts (12 us) on the booted machine, in
//! seconds instead of through the disc and BASIC.
//!
//! The real CRTC 0's counts (two photos, a GT65 and a CTM 644): 1451, then R0=0 with C4=32 C9=3:
//! 1557 and 1557, C4=32 C9=7: 1536 and 1536, C4=38 C9=7: 1280 and 1280, C4=0 C9=0: 1274 and 1274.

use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16};

fn values(crtc: u8) -> Vec<u16> {
    let bin = include_bytes!("fixtures/claudestrad/r0ztest/R0ZTEST.BIN");
    let code = &bin[128..128 + 0x1029];
    let mut runner = CslRunner::new("/tmp/r0ztest_probe");
    runner
        .run_text(
            &format!("csl_version 1.0\ncrtc_select {crtc}\nreset\nwait 4000000\n"),
            ".",
        )
        .unwrap();
    let mb = runner.motherboard_mut();
    mb.write_buffer(0x4000, code);
    mb.write_buffer(0x7f00, &[0x18, 0xfe]);
    // a CALL &4000 from BASIC: the return address is a trap
    let sp = mb.cpu().reg16(Reg16::Sp).value().wrapping_sub(2);
    mb.write_buffer(sp, &[0x00, 0x7f]);
    mb.cpu_mut().reg16_mut(Reg16::Sp).set(sp);
    mb.set_pc(0x4000);
    let limit = 4_000_000usize * 60;
    let start = mb.t_state_counter().value();
    while mb.cpu().reg16(Reg16::Pc).value() != 0x7f00
        && mb.t_state_counter().value() - start < limit
    {
        mb.execute_next_opcode_with_video(|_| {});
    }
    (0..33).map(|k| mb.read_word(0x7000 + 2 * k)).collect()
}

#[test]
fn r0_zero_with_c9_on_r9_ends_the_row_as_on_the_real_crtc_0() {
    let c = values(0);
    // C4=32 C9=7 and C4=38 C9=7 (the frame's end), 512 and 1024 us
    assert_eq!(c[3..7], [1536, 1536, 1280, 1280]);
    // C4=32 C9=3, C4=0 C9=0
    assert_eq!([c[1], c[2], c[7], c[8]], [1557, 1557, 1274, 1274]);
}

#[test]
#[ignore = "prints the counts: cargo test --release --test r0ztest_probe -- --ignored --nocapture"]
fn print_the_counts() {
    for crtc in [0u8, 1, 2] {
        println!("CRTC {crtc}: {:?}", values(crtc));
    }
}
