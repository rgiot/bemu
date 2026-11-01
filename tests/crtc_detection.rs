/// Check that the CRTC detection methods DO NOT detect an emulator but the right CRTC emulated
///
///
use amsrust::crtc::CrtcType;
use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16, Reg8};


enum CrtcTest {
    MadramAmslive,
}

impl CrtcTest {
    /// Return the address where to assemble/load/run the test
    fn address(&self) -> u16 {
        match self {
            CrtcTest::MadramAmslive => 0x9001,
        }
    }

    /// Return the assembled test
    fn test(&self) -> (&[u8], u16) {
        match self {
            CrtcTest::MadramAmslive => (
                cpclib_macros::assemble!(fname:"tests/madram_crtc_test.asm"),
                self.address(),
            ),
        }
    }
}

fn test_crtc(crtc_type: CrtcType, crtc_test: CrtcTest) {
    // build the CPC motherboard
    let mut motherboard = MotherboardBuilder::default()
                                        .set_crtc(crtc_type)
                                        .enable_interruptions()
                                        .disable_lower_rom()
                                        .disable_upper_rom()
                                        .build();

    // build the test
    let (test, addr) = crtc_test.test();
    eprintln!("{:x?}", test);

    assert!(!test.is_empty());
    let end_pattern = cpclib_macros::assemble!(" jr $");
    let inter_code = cpclib_macros::assemble!(" ei : ret");
    assert_eq!(
        inter_code,
        &[0xfb, 0xc9]
    );

    // inject it in memory
    motherboard.write_buffer(addr, test);
    motherboard.write_buffer(0x38, inter_code);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(addr);

    // execute it until it is finished
    let mut last_d = 0xffu8;
    loop {
        let pc_before = motherboard.cpu().reg16(Reg16::Pc).value();
        motherboard.execute_next_opcode_no_video();
        let pc = motherboard.cpu().reg16(Reg16::Pc).value();

        let d = motherboard.cpu().reg8(Reg8::D).value();
        if d != last_d {
            eprintln!("pc_before={pc_before:04x} -> D changed to {d:#010b} ({d:#x})");
            last_d = d;
        }

        // detect if it is the end
        let current_pattern = [motherboard.read_byte(pc), motherboard.read_byte(pc + 1)];

        if current_pattern == *end_pattern {
            eprintln!("reached end at pc={pc:04x} a={:#x}", motherboard.cpu().reg8(Reg8::A).value());
            break;
        }
    }

    // retreive the crtc number
    let obtained_crtc = motherboard.cpu().reg8(Reg8::A).value();
    let expected_crtc = crtc_type as u8;

    // and make the comparison
    assert_eq!(obtained_crtc, expected_crtc)
}

#[test]
fn crtc0_madram() {
    test_crtc(CrtcType::Crtc0, CrtcTest::MadramAmslive);
}
