//! The CRTC detections of demos run on each emulated CRTC: they must find the CRTC emulated
//! (not an emulator, not another CRTC).
//!
//! - Madram's (Amslive): `tests/madram_crtc_test.asm`, the type in A.
//! - OffseT's TestCRTC v3.0 (Futurs', 2005, after Longshot's of Logon System; the one Blight
//!   uses): `tests/fixtures/offset_testcrtc_v3.asm`. Each of its 11 tests narrows a mask of the
//!   CRTCs it can be (bit n: CRTC n); none may rule out the CRTC emulated, together they must
//!   leave it alone, and TestCRTC must return it.

use amsrust::crtc::CrtcType;
use amsrust::motherboard::{CpcModel, Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16, Reg8};

const CRTCS: [CrtcType; 5] = [CrtcType::Crtc0, CrtcType::Crtc1, CrtcType::Crtc2, CrtcType::Crtc3, CrtcType::Crtc4];

/// A machine with `crtc`: a 6128+ for the ASIC's CRTC 3 (its PPI is the ASIC's too), a 6128
/// otherwise.
fn builder(crtc: CrtcType) -> MotherboardBuilder {
    let mut builder = MotherboardBuilder::default();
    if crtc == CrtcType::Crtc3 {
        builder.set_model("6128plus".parse::<CpcModel>().unwrap());
    }
    builder.set_crtc(crtc);
    builder
}

fn run_until(motherboard: &mut Motherboard, seconds: usize, done: impl Fn(&Motherboard) -> bool) -> bool {
    let end = motherboard.t_state_counter().value() + seconds * 4_000_000;
    while motherboard.t_state_counter().value() < end {
        if done(motherboard) {
            return true;
        }
        motherboard.execute_next_opcode_no_video();
    }
    false
}

/// Madram's test, from &9001, the firmware off, EI:RET at &0038: the type it returns.
fn madram(crtc: CrtcType) -> u8 {
    let mut motherboard = builder(crtc).enable_interruptions().disable_lower_rom().disable_upper_rom().build();
    let test = cpclib_macros::assemble!(fname: "tests/madram_crtc_test.asm");
    motherboard.write_buffer(0x9001, test);
    motherboard.write_buffer(0x38, &[0xfb, 0xc9]);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x9001);
    // it ends on a JR $
    let ended = run_until(&mut motherboard, 5, |motherboard| {
        let pc = motherboard.cpu().reg16(Reg16::Pc).value();
        [motherboard.read_byte(pc), motherboard.read_byte(pc.wrapping_add(1))] == [0x18, 0xfe]
    });
    assert!(ended, "CRTC {}: Madram's test didn't end", crtc as u8);
    motherboard.cpu().reg8(Reg8::A).value()
}

/// On CRTC 0 only, as this listing goes: it finds the other ones only with three lines changed
/// (`out (c),c` at `setcrtc`, R12 in `crtclist`, `ld a,e` at `t3_ok`), which aren't made until
/// the original listing is checked.
#[test]
fn madram_finds_crtc0() {
    assert_eq!(madram(CrtcType::Crtc0), 0, "Madram's test on CRTC 0");
}

/// OffseT's tests, in TestCRTC's order.
const OFFSET_TESTS: [&str; 11] = [
    "TestLongueurVBL", "TestBFxx", "TestBExx", "TestFinVBL", "TestR4R5", "TestRegsWO", "TestBloc", "TestBorder",
    "TestRAZPPI", "TestPortBPPI", "TestReg31"
];

/// Where the harness keeps the mask after each test, then TestCRTC's answer and an end mark.
const MASKS: u16 = 0x9000;
const ANSWER: u16 = 0x9020;
const DONE: u16 = 0x9021;

/// OffseT's tests from BASIC, as Blight runs them (EI:RET at &0038, since TestCRTC ends with
/// EI): the mask after each test, and TestCRTC's answer.
fn offset(crtc: CrtcType) -> (Vec<u8>, u8) {
    let source = include_str!("fixtures/offset_testcrtc_v3.asm");
    let mut harness = String::from(" org &4000\n di\n ld hl,&c9fb\n ld (&38),hl\n ld a,&ff\n ld (TypeCRTC),a\n");
    for (index, test) in OFFSET_TESTS.iter().enumerate() {
        harness += &format!(" call {test}\n ld a,(TypeCRTC)\n ld (&{:04x}),a\n", MASKS + index as u16);
    }
    harness += &format!(" call TestCRTC\n ld (&{ANSWER:04x}),a\n ld a,&aa\n ld (&{DONE:04x}),a\nloop jr loop\n");
    harness += source;
    let code = cpclib_asm::assemble(&harness).unwrap_or_else(|e| panic!("{e}"));

    let mut motherboard = builder(crtc).build();
    // BASIC up, its CRTC settings
    run_until(&mut motherboard, 3, |_| false);
    motherboard.tick_until_next_instruction();
    motherboard.write_buffer(MASKS, &[0; 0x40]);
    motherboard.write_buffer(0x4000, &code);
    motherboard.set_pc(0x4000);
    let ended = run_until(&mut motherboard, 10, |motherboard| motherboard.read_byte(DONE) == 0xaa);
    assert!(ended, "CRTC {}: OffseT's TestCRTC didn't end", crtc as u8);
    let masks = (0..OFFSET_TESTS.len() as u16).map(|index| motherboard.read_byte(MASKS + index)).collect();
    (masks, motherboard.read_byte(ANSWER))
}

#[test]
fn offset_testcrtc_finds_each_crtc() {
    for crtc in CRTCS {
        let n = crtc as u8;
        let (masks, answer) = offset(crtc);
        for (test, mask) in OFFSET_TESTS.iter().zip(&masks) {
            assert!(mask & (1 << n) != 0, "CRTC {n}: {test} rules it out (mask {mask:#04x}, masks {masks:02x?})");
        }
        assert_eq!(masks.last(), Some(&(1 << n)), "CRTC {n}: the tests leave it alone ({masks:02x?})");
        assert_eq!(answer, n, "CRTC {n}: TestCRTC's answer");
    }
}
