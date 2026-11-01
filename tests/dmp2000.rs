use amsrust::{dmp2000::Dmp2000, motherboard::MotherboardBuilder};

/// A Z80 program prints through the printer port the way the firmware does: waits for BUSY to
/// fall (PPI port B, bit 6), puts the byte on the latch (&EFxx), then pulses /STROBE (bit 7).
#[test]
fn a_program_prints_through_the_printer_port() {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .plug_dmp2000()
        .build();
    // the RAM powers up with RST &38s: the Z80 only runs the program
    motherboard.write_buffer(0, &[0u8; 0x4000]);

    let program = cpclib_macros::assemble!(
        "
        org 0x4000
        ld hl, message
    again:
        ld a, (hl)
        or a
        jr z, stop
        ld d, a
    busy_loop:
        ld b, 0xf5
        in a, (c)
        and 0x40
        jr nz, busy_loop
        ld bc, 0xef00
        out (c), d
        ld a, d
        or 0x80
        out (c), a
        out (c), d
        inc hl
        jr again
    stop:
        jr stop
    message:
        db 72, 105, 13, 10, 0
    "
    );
    motherboard.write_buffer(0x4000, program);
    motherboard.set_pc(0x4000);
    motherboard.tick_until_next_instruction();

    for _ in 0..3_000_000 {
        motherboard.tick();
    }

    let printer = motherboard
        .printer_port()
        .and_then(|port| port.device())
        .and_then(|device| device.as_any().downcast_ref::<Dmp2000>())
        .expect("a DMP2000 on the printer port");
    assert_eq!(printer.raw(), b"Hi\r\n");
    assert_eq!(printer.text(), "Hi\n");
    assert_eq!(printer.inked_pages().count(), 1);
}
