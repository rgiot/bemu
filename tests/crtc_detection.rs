//! The CRTC detections of demos run on each emulated CRTC: they must find the CRTC emulated
//! (not an emulator, not another CRTC).
//!
//! - Madram's (Amslive): `tests/madram_crtc_test.asm`, the type in A.
//! - Madram's of Amstrad Live (05/2000), from the fanzine's listing:
//!   `tests/fixtures/madram_amstrad_live_2000.asm`, typed in as `CALL &A000` (it prints the
//!   type through the firmware, `'0'` + the type in A; 5 an emulator).
//! - From emu-info's acid tests (Kevin Thacker's collection, in Pasmo's syntax), typed in as
//!   `CALL &xxxx` from BASIC: Batman Begins' (Rhino) and Madram's of Amslive (06/2002), which
//!   print the type, and Longshot's TEST CRTC V1.1 (Logon System, 1991), which leaves it at &BF00.
//! - OffseT's TestCRTC v3.0 (Futurs', 2005, after Longshot's of Logon System; the one Blight
//!   uses): `tests/fixtures/offset_testcrtc_v3.asm`. Each of its 11 tests narrows a mask of the
//!   CRTCs it can be (bit n: CRTC n); none may rule out the CRTC emulated, together they must
//!   leave it alone, and TestCRTC must return it.

use amsrust::crtc::CrtcType;
use amsrust::keyboard::CpcKey;
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

/// Where the hook on TXT OUTPUT (&BB5A) lives, and where it keeps the digit printed and its
/// end mark: free RAM under BASIC, below the listings.
const HOOK: u16 = 0x8f00;
const PRINTED: u16 = 0x8ff0;
const PRINTED_DONE: u16 = 0x8ff1;

fn run_for(motherboard: &mut Motherboard, t_states: usize) {
    let end = motherboard.t_state_counter().value() + t_states;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_no_video();
    }
}

/// Types `text` on the CPC's keyboard, each key held for 3 frames then released for 2.
fn type_text(motherboard: &mut Motherboard, text: &str) {
    for c in text.chars() {
        let keys = if c == '\r' {
            vec![CpcKey::Return]
        } else {
            motherboard.keyboard_layout().keys_of_char(c).unwrap_or_else(|| panic!("no key for {c:?}"))
        };
        for &key in &keys {
            motherboard.keyboard_mut().set_pressed(key, true);
        }
        run_for(motherboard, 3 * 80_000);
        for &key in &keys {
            motherboard.keyboard_mut().set_pressed(key, false);
        }
        run_for(motherboard, 2 * 80_000);
    }
}

/// A machine with `crtc` at BASIC's prompt (a CPC+ leaves its system cartridge's menu with f1).
fn at_basic_prompt(crtc: CrtcType) -> Motherboard {
    let mut motherboard = builder(crtc).build();
    run_until(&mut motherboard, 3, |_| false);
    if crtc == CrtcType::Crtc3 {
        // (0.1 s: held longer, it repeats into BASIC's line)
        motherboard.keyboard_mut().set_pressed(CpcKey::Keypad1, true);
        run_for(&mut motherboard, 400_000);
        motherboard.keyboard_mut().set_pressed(CpcKey::Keypad1, false);
        run_until(&mut motherboard, 4, |_| false);
    }
    motherboard
}

/// Loads `code` at `org` and types `CALL &org` at BASIC's prompt, as their authors run these
/// listings. Then runs until `done`, for at most 10 s.
fn call_from_basic(crtc: CrtcType, code: &[u8], org: u16, before_return: impl Fn(&mut Motherboard), done: impl Fn(&Motherboard) -> bool) -> Motherboard {
    let mut motherboard = at_basic_prompt(crtc);
    motherboard.write_buffer(org, code);
    type_text(&mut motherboard, &format!("CALL &{org:04X}"));
    before_return(&mut motherboard);
    motherboard.keyboard_mut().set_pressed(CpcKey::Return, true);
    let ended = run_until(&mut motherboard, 10, done);
    assert!(ended, "CRTC {}: the listing at &{org:04x} didn't end (PC &{:04x})", crtc as u8, motherboard.pc());
    motherboard
}

/// emu-info's listings are in Pasmo's syntax: its `END entry` directive, which ends the source
/// and names the entry point, is the only part cpclib-asm doesn't take. It is dropped (the
/// harness calls the entry point itself).
fn assemble_pasmo(source: &str) -> Vec<u8> {
    let source: String = source
        .lines()
        .filter(|line| !line.trim_start().to_ascii_lowercase().starts_with("end "))
        .map(|line| format!("{line}\n"))
        .collect();
    cpclib_asm::assemble(&source).unwrap_or_else(|e| panic!("{e}"))
}

/// For the listings that print the type through TXT OUTPUT (`'0'` + the type; 5 an emulator):
/// the type printed. The firmware's jump to TXT OUTPUT at &BB5A, in RAM, goes to a hook that
/// keeps the first digit printed and stops (BASIC's own printing, a new line, passes through it
/// unprinted). The listings aren't changed.
fn printed_type(crtc: CrtcType, source: &str, org: u16) -> u8 {
    let code = assemble_pasmo(source);
    let hook = cpclib_asm::assemble(&format!(
        " org &{HOOK:04x}\n cp '0'\n ret c\n cp '9'+1\n ret nc\n ld (&{PRINTED:04x}),a\n ld a,&aa\n ld (&{PRINTED_DONE:04x}),a\n jr $\n"
    ))
    .unwrap();
    let motherboard = call_from_basic(
        crtc,
        &code,
        org,
        |motherboard| {
            motherboard.write_buffer(HOOK, &hook);
            motherboard.write_buffer(PRINTED, &[0, 0]);
            motherboard.write_buffer(0xbb5a, &[0xc3, HOOK as u8, (HOOK >> 8) as u8]);
        },
        |motherboard| motherboard.read_byte(PRINTED_DONE) == 0xaa
    );
    motherboard.read_byte(PRINTED) - b'0'
}

fn printed_types(source: &str, org: u16) -> Vec<u8> {
    CRTCS.iter().map(|&crtc| printed_type(crtc, source, org)).collect()
}

#[test]
fn madram_amstrad_live_finds_each_crtc() {
    let found = printed_types(include_str!("fixtures/madram_amstrad_live_2000.asm"), 0xa000);
    assert_eq!(found, [0, 1, 2, 3, 4], "Madram's Amstrad Live (05/2000) test on CRTC 0 to 4");
}

/// Batman Begins' (Rhino), from emu-info's acid tests.
#[test]
fn batman_begins_finds_each_crtc() {
    let found = printed_types(include_str!("fixtures/batman_begins_rhino.asm"), 0x4000);
    assert_eq!(found, [0, 1, 2, 3, 4], "Batman Begins' test on CRTC 0 to 4");
}

/// Madram's of Amslive (06/2002), as emu-info transcribed it. `tests/madram_crtc_test.asm` is
/// another transcription of the same article: both have `out (c),a` at `setcrtc` and
/// `t3_ok ld a,a`; this one's test3 selects R0 (`ld bc,&bc00`, its comment says R12; the other
/// selects R12) and its `conclut` has `inc c` (the other `in a,(c)`). As it stands it can't tell
/// any CRTC: R0 is write-only on all five, so &BF00 reads 0 (ACCC §21.2: CRTC 0, 1 and 2 return 0
/// for it, CRTC 3 and 4 read R16, the light pen, 0 without a strobe), `ld a,a` then ANDs the
/// candidates with that 0, none is left, and it answers 5, an emulator. A real CPC does the
/// same whatever its CRTC, so must bemu.
#[test]
fn madram_amslive_2002_as_transcribed_finds_no_crtc() {
    let found = printed_types(include_str!("fixtures/crtc_amstlive_06_2002_madram.asm"), 0x9001);
    assert_eq!(found, [5; 5], "Madram's Amslive (06/2002) test, as transcribed, on CRTC 0 to 4");
}

/// Longshot's TEST CRTC V1.1 (Logon System, 1991), from emu-info's acid tests: the type at
/// &BF00, then it draws it and loops (JR $).
#[test]
fn longshot_finds_each_crtc() {
    let code = assemble_pasmo(include_str!("fixtures/crtc_v1_1_longshot.asm"));
    let found: Vec<u8> = CRTCS
        .iter()
        .map(|&crtc| {
            let motherboard = call_from_basic(crtc, &code, 0xa000, |_| (), |motherboard| {
                let pc = motherboard.pc();
                pc >= 0xa000 && [motherboard.read_byte(pc), motherboard.read_byte(pc.wrapping_add(1))] == [0x18, 0xfe]
            });
            motherboard.read_byte(0xbf00)
        })
        .collect();
    assert_eq!(found, [0, 1, 2, 3, 4], "Longshot's TEST CRTC V1.1 on CRTC 0 to 4");
}
