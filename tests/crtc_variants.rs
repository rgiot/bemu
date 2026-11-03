//! The five CRTC types. Expected values come from Longshot's *Amstrad CPC
//! CRTC Compendium* (`doc/ACCC1.9-EN.pdf`, "ACCC"): §4.3 register table,
//! §14 sync widths, §19 skew, §21 register/status reads, §28.1.9.
//!
//! Also checks the chips through the two CRTC identification programs the
//! repository ships, which is how the difference shows on a real machine:
//! RHINO (Longshot) and the MadRam/Amslive test (`tests/madram_crtc_test.asm`).

use amsrust::chip::ReadDataChip;
use amsrust::crtc::{Crtc, CrtcType, DisplaySkew, ReadContext, Registers, NB_REGISTERS};
use amsrust::disc_image::load_dsk;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16, Reg8};
use CrtcType::*;

// ------------------------------------------------------------ registers

/// ACCC §4.3, register by register.
fn expected_write_mask(crtc: CrtcType, register: u8) -> u8 {
    let asic_or_hd = matches!(crtc, Crtc0 | Crtc3 | Crtc4);
    match register {
        0 | 1 | 2 | 13 | 15 => 0xff,
        3 if asic_or_hd => 0xff, // R3h (VSYNC width) and R3l (HSYNC width)
        3 => 0x0f,               // R3l only
        4 | 6 | 7 | 10 => 0x7f,
        5 | 9 | 11 => 0x1f,
        8 if asic_or_hd => 0xf3, // cursor skew, display skew, interlace
        8 => 0x03,               // interlace only
        12 | 14 => 0x3f,
        _ => 0x00
    }
}

#[test]
fn each_chip_stores_the_register_bits_the_compendium_gives_it() {
    for crtc in CrtcType::ALL {
        let model = crtc.new_chip();
        for register in 0..=17u8 {
            assert_eq!(
                model.write_mask(register),
                expected_write_mask(crtc, register),
                "{crtc:?} ({}) R{register}",
                model.name()
            );
        }
    }
}

#[test]
fn writing_through_the_chip_applies_the_mask() {
    for crtc in CrtcType::ALL {
        let mut chip = Crtc::from(crtc);
        chip.do_write_register(3, 0xff);
        chip.do_write_register(8, 0xff);
        chip.do_write_register(12, 0xff);
        assert_eq!(chip.r3(), expected_write_mask(crtc, 3), "{crtc:?} R3");
        assert_eq!(chip.r8(), expected_write_mask(crtc, 8), "{crtc:?} R8");
        assert_eq!(chip.r12(), 0x3f, "{crtc:?} R12");
        // registers that don't exist are ignored, not a panic
        chip.do_write_register(17, 0xff);
        chip.do_write_register(200, 0xff);
    }
}

// ---------------------------------------------------------------- reads

fn registers() -> Registers {
    let mut registers = [0u8; NB_REGISTERS];
    registers[12] = 0x3f;
    registers[13] = 0xa5;
    registers[14] = 0x2b;
    registers[15] = 0xc3;
    registers[16] = 0x15;
    registers[17] = 0x99;
    registers[10] = 0x11;
    registers[11] = 0x12;
    registers
}

fn read_register(crtc: CrtcType, selected: u8) -> u8 {
    let registers = registers();
    crtc.new_chip().read_register(&ReadContext {
        registers: &registers,
        selected,
        vertical_border: false,
        noise: false,
        c0: 0,
        c4: 0,
        c9: 0,
        vsync_line: None,
        vma: 0,
        vma_prime: 0,
        fields: 0
    })
}

#[test]
fn crtc0_reads_r12_to_r17_and_zero_elsewhere() {
    // ACCC §21.2.1: 5 significant bits of the register number
    assert_eq!(read_register(Crtc0, 12), 0x3f);
    assert_eq!(read_register(Crtc0, 13), 0xa5);
    assert_eq!(read_register(Crtc0, 14), 0x2b);
    assert_eq!(read_register(Crtc0, 15), 0xc3);
    assert_eq!(read_register(Crtc0, 16), 0x15);
    assert_eq!(read_register(Crtc0, 17), 0x99);
    for unreadable in [0, 1, 4, 10, 11, 18, 31] {
        assert_eq!(read_register(Crtc0, unreadable), 0, "R{unreadable}");
    }
    // "selecting register 108 is equivalent to selecting register 12"
    assert_eq!(read_register(Crtc0, 108), 0x3f);
}

#[test]
fn pointer_registers_read_back_six_or_eight_bits() {
    // R12/R14/R16 are 6 bits wide, R13/R15/R17 8 (ACCC §21.2)
    let mut registers = registers();
    for number in [12usize, 14, 16] {
        registers[number] = 0xff;
    }
    for (crtc, selected) in [(Crtc0, 12), (Crtc0, 14), (Crtc0, 16), (Crtc1, 14), (Crtc2, 16)] {
        let value = crtc.new_chip().read_register(&ReadContext {
            registers: &registers,
            selected,
            vertical_border: false,
            noise: false,
            c0: 0,
            c4: 0,
            c9: 0,
            vsync_line: None,
            vma: 0,
            vma_prime: 0,
            fields: 0
        });
        assert_eq!(value, 0x3f, "{crtc:?} R{selected}");
    }
}

#[test]
fn crtc1_and_2_read_only_r14_to_r17() {
    // ACCC §21.2.2
    for crtc in [Crtc1, Crtc2] {
        assert_eq!(read_register(crtc, 14), 0x2b);
        assert_eq!(read_register(crtc, 15), 0xc3);
        assert_eq!(read_register(crtc, 16), 0x15);
        assert_eq!(read_register(crtc, 17), 0x99);
        for unreadable in [0, 4, 10, 11, 12, 13, 18] {
            assert_eq!(read_register(crtc, unreadable), 0, "{crtc:?} R{unreadable}");
        }
    }
}

#[test]
fn crtc1_alone_returns_something_for_register_31() {
    assert!(matches!(read_register(Crtc1, 31), 0x7f | 0xff));
    assert_eq!(read_register(Crtc2, 31), 0);
    assert_eq!(read_register(Crtc0, 31), 0);
    // "and all values whose bits 0-4 are 1"
    assert!(read_register(Crtc1, 0xff) != 0);
}

#[test]
fn asics_read_through_a_three_bit_table() {
    // ACCC §21.2.3: 0 R16, 1 R17, 2 R10 (status 1), 3 R11 (status 2),
    // 4 R12, 5 R13, 6 R14, 7 R15; only the 3 low bits of the number count
    for crtc in [Crtc3, Crtc4] {
        assert_eq!(read_register(crtc, 0), 0x15);
        assert_eq!(read_register(crtc, 1), 0x99);
        assert_eq!(read_register(crtc, 4), 0x3f, "register 4 reads R12");
        assert_eq!(read_register(crtc, 12), 0x3f, "12 = 8+4 reads R12 too");
        assert_eq!(read_register(crtc, 20), 0x3f, "and 20 = 16+4");
        assert_eq!(read_register(crtc, 5), 0xa5);
        assert_eq!(read_register(crtc, 6), 0x2b);
        assert_eq!(read_register(crtc, 7), 0xc3);
    }
}

#[test]
fn register_4_reads_zero_on_the_6845s() {
    // what RHINO/MadRam's "read register 4" test relies on
    for crtc in [Crtc0, Crtc1, Crtc2] {
        assert_eq!(read_register(crtc, 4), 0, "{crtc:?}");
    }
}

// --------------------------------------------------------------- status

fn read_status(crtc: CrtcType, vertical_border: bool, noise: bool) -> u8 {
    let registers = registers();
    crtc.new_chip().read_status(&ReadContext {
        registers: &registers,
        selected: 12,
        vertical_border,
        noise,
        c0: 0,
        c4: 0,
        c9: 0,
        vsync_line: None,
        vma: 0,
        vma_prime: 0,
        fields: 0
    })
}

#[test]
fn only_crtc1_has_a_status_register() {
    // ACCC §21.3.1/§21.3.3: x L V x x x x x, V = BORDER by R6, L = 0 (R16/17 readable)
    assert_eq!(read_status(Crtc1, true, false), 1 << 5);
    assert_eq!(read_status(Crtc1, false, false), 0);
}

#[test]
fn crtc0_and_2_have_no_status_register() {
    // ACCC §21.3.2: CRTC 2 always returns 255, CRTC 0 randomly 255 or 127
    assert_eq!(read_status(Crtc2, false, false), 0xff);
    assert_eq!(read_status(Crtc2, true, true), 0xff);
    assert_eq!(read_status(Crtc0, false, true), 0xff);
    assert_eq!(read_status(Crtc0, false, false), 0x7f);
}

#[test]
fn asics_status_port_mirrors_the_read_port() {
    // ACCC §21.3.1
    for crtc in [Crtc3, Crtc4] {
        assert_eq!(read_status(crtc, false, false), read_register(crtc, 12));
    }
}

#[test]
fn crtc0_status_is_random_but_only_ever_ff_or_7f_and_reproducible() {
    let mut seen = std::collections::HashSet::new();
    let mut chip = Crtc::from(Crtc0);
    // BE00 read: A9=1, A8=0 -> RW=1, RS=0 through the chip's own bus logic
    for _ in 0..64 {
        chip.status_register();
        seen.insert(chip.get_data_from_pins());
    }
    assert!(seen.iter().all(|v| matches!(v, 0xff | 0x7f)), "{seen:?}");
    assert_eq!(seen.len(), 2, "both values show up");

    // the noise is a fixed sequence: another chip gives the same values
    let mut other = Crtc::from(Crtc0);
    let mut chip = Crtc::from(Crtc0);
    for _ in 0..16 {
        chip.status_register();
        other.status_register();
        assert_eq!(chip.get_data_from_pins(), other.get_data_from_pins());
    }
}

// ------------------------------------------------------------- sync/skew

#[test]
fn hsync_width_follows_r3l_with_chip_specific_zero() {
    // ACCC §14.6: R3l = 0 gives no HSYNC on CRTC 0 and 1, 16 characters on 2, 3, 4
    for crtc in [Crtc0, Crtc1] {
        assert_eq!(crtc.new_chip().hsync_width(0x00), None, "{crtc:?}");
        assert_eq!(crtc.new_chip().hsync_width(0xf0), None, "{crtc:?}");
        assert_eq!(crtc.new_chip().hsync_width(0x05), Some(5));
    }
    for crtc in [Crtc2, Crtc3, Crtc4] {
        assert_eq!(crtc.new_chip().hsync_width(0x00), Some(16), "{crtc:?}");
        assert_eq!(crtc.new_chip().hsync_width(0x0e), Some(14));
    }
}

#[test]
fn vsync_width_is_r3h_only_on_crtc_0_3_4() {
    // ACCC §14.1: 1 and 2 are fixed at 16; 0 means 16 on the others
    for crtc in [Crtc1, Crtc2] {
        assert_eq!(crtc.new_chip().vsync_width(0x80), 16, "{crtc:?}");
        assert_eq!(crtc.new_chip().vsync_width(0x30), 16, "{crtc:?}");
    }
    for crtc in [Crtc0, Crtc3, Crtc4] {
        assert_eq!(crtc.new_chip().vsync_width(0x80), 8, "{crtc:?}");
        assert_eq!(crtc.new_chip().vsync_width(0x3e), 3, "{crtc:?}");
        assert_eq!(crtc.new_chip().vsync_width(0x0e), 16, "{crtc:?}: 0 means 16");
    }
}

#[test]
fn display_skew_exists_on_crtc_0_3_4_only() {
    // ACCC §19.2: R8 bits 5-4; a skew of 3 blanks the display
    for crtc in [Crtc0, Crtc3, Crtc4] {
        assert_eq!(crtc.new_chip().display_skew(0x00), DisplaySkew::Delayed(0));
        assert_eq!(crtc.new_chip().display_skew(0x10), DisplaySkew::Delayed(1));
        assert_eq!(crtc.new_chip().display_skew(0x20), DisplaySkew::Delayed(2));
        assert_eq!(crtc.new_chip().display_skew(0x30), DisplaySkew::Blanked);
    }
    for crtc in [Crtc1, Crtc2] {
        assert_eq!(crtc.new_chip().display_skew(0x30), DisplaySkew::Delayed(0), "{crtc:?}");
    }
}

#[test]
fn a_zero_width_hsync_is_never_started_on_crtc_0_and_1() {
    // R3l = 0: no HSYNC pulse at all (it used to start and never end)
    for crtc in [Crtc0, Crtc1] {
        let mut motherboard = MotherboardBuilder::default()
            .disable_lower_rom()
            .disable_upper_rom()
            .set_crtc(crtc)
            .build();
        motherboard.crtc_mut().do_write_register(3, 0x00);
        for _ in 0..200_000 {
            motherboard.tick();
            assert!(!motherboard.crtc().hsync(), "{crtc:?} started an HSYNC with R3l=0");
        }
    }
}

#[test]
fn the_type_can_be_read_back_and_converted_from_a_number() {
    for (number, crtc) in CrtcType::ALL.into_iter().enumerate() {
        assert_eq!(Crtc::from(crtc).crtc_type(), crtc);
        assert_eq!(CrtcType::try_from(number as u8), Ok(crtc));
        assert_eq!(crtc as usize, number);
    }
    assert!(CrtcType::try_from(5).is_err());
}

// ------------------------------------------------ the detection programs

/// RHINO's own detection routine (Longshot's `RHINO.DSK`): file bytes after
/// the AMSDOS header are loaded at &0170 by BASIC and the routine at &0195
/// returns the CRTC number in A.
fn rhino_routine_result(crtc: CrtcType) -> u8 {
    let disc = load_dsk(&std::fs::read("validation/RHINO.DSK").unwrap()).unwrap();
    let track = disc.track(0, 0).unwrap();
    // the file is in allocation block 2: sectors &C5 and &C6
    let mut file = Vec::new();
    for id in [0xc5u8, 0xc6] {
        file.extend_from_slice(&track.sectors.iter().find(|sector| sector.r == id).unwrap().data);
    }
    let body = &file[128..128 + 256];

    let mut motherboard = MotherboardBuilder::default()
        .set_crtc(crtc)
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    // what the firmware programs: 64 µs lines, 38+1 rows, 25 displayed
    for (register, value) in [(0, 63), (1, 40), (2, 46), (3, 0x8e), (4, 38), (6, 25), (7, 30), (9, 7)] {
        motherboard.crtc_mut().do_write_register(register, value);
    }
    motherboard.write_buffer(0x0170, body);
    // CALL &0195 ; JR $
    motherboard.write_buffer(0x4000, &[0xcd, 0x95, 0x01, 0x18, 0xfe]);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);

    run_until_parked(&mut motherboard, 0x4003);
    motherboard.cpu().reg8(Reg8::A).value()
}

fn run_until_parked(motherboard: &mut Motherboard, park_at: u16) {
    for _ in 0..3_000_000 {
        if motherboard.cpu().reg16(Reg16::Pc).value() == park_at {
            return;
        }
        motherboard.execute_next_opcode_no_video();
    }
    panic!("the routine never returned");
}

#[test]
fn rhino_identifies_every_crtc_type() {
    for crtc in CrtcType::ALL {
        assert_eq!(
            rhino_routine_result(crtc),
            crtc as u8,
            "RHINO took {crtc:?} for another chip"
        );
    }
}

/// The ASICs' status registers (ACCC §21.3.4) at chosen counters: STATUS 1 (R10, read as
/// register 2 or 10) and STATUS 2 (R11).
#[test]
fn asics_status_registers_report_the_counters_events() {
    let mut registers = registers();
    // R0 63, R1 40, R2 46, R3 &8E (VSYNC 8 lines), R4 38, R6 25, R7 30, R9 7
    for (register, value) in [(0, 63), (1, 40), (2, 46), (3, 0x8e), (4, 38), (6, 25), (7, 30), (9, 7)] {
        registers[register] = value;
    }
    let read = |crtc: CrtcType, selected: u8, c0: u8, c4: u8, c9: u8, vsync_line: Option<u8>| {
        crtc.new_chip().read_register(&ReadContext {
            registers: &registers,
            selected,
            vertical_border: false,
            noise: false,
            c0,
            c4,
            c9,
            vsync_line,
            vma: 0,
            vma_prime: 0,
            fields: 0
        })
    };
    for crtc in [Crtc3, Crtc4] {
        // STATUS 1: bit 6 always set; bit 0 set at C0=R0; bit 3 cleared at C0=R2; bit 5 cleared on
        // the VSYNC's last line (OffseT's TestCRTC: the end of the VBL)
        assert_eq!(read(crtc, 10, 10, 0, 0, None) & 0x41, 0x40, "{crtc:?}");
        assert_eq!(read(crtc, 10, 63, 0, 0, None) & 0x01, 0x01, "{crtc:?} C0=R0");
        assert_eq!(read(crtc, 10, 46, 0, 0, None) & 0x08, 0, "{crtc:?} C0=R2");
        assert_eq!(read(crtc, 2, 10, 30, 0, Some(6)) & 0x20, 0x20, "{crtc:?} during the VSYNC (R2 reads R10)");
        assert_eq!(read(crtc, 10, 10, 30, 7, Some(7)) & 0x20, 0, "{crtc:?} its last line");
        // STATUS 2: bit 4 set, bit 6 clear; bit 5 cleared on C9=R9; bit 0 cleared on the screen's
        // last character
        assert_eq!(read(crtc, 11, 10, 0, 0, None) & 0x50, 0x10, "{crtc:?}");
        assert_eq!(read(crtc, 11, 10, 0, 7, None) & 0x20, 0, "{crtc:?} C9=R9");
        assert_eq!(read(crtc, 11, 63, 38, 7, None) & 0x01, 0, "{crtc:?} the last character");
    }
}
