//! Non-maskable interrupts (an expansion's /NMI, e.g. the PlayCity's CTC) and the IM 2 vector
//! an interrupting device puts on the bus.

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16};

fn machine(program: &[u8], at: u16) -> Motherboard {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .enable_interruptions()
        .build();
    motherboard.write_buffer(at, program);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(at);
    motherboard
}

fn run(motherboard: &mut Motherboard, instructions: usize) {
    for _ in 0..instructions {
        motherboard.execute_next_opcode();
    }
}

#[test]
fn an_nmi_calls_0066_and_retn_restores_the_interrupts() {
    // EI then a loop; the NMI handler marks memory and returns
    let mut motherboard = machine(cpclib_macros::assemble!(" ld sp,0xc000 : im 1 : ei : loop: jr loop"), 0x100);
    motherboard.write_buffer(0x66, cpclib_macros::assemble!(" ld a,0x42 : ld (0x8000),a : retn"));
    // no maskable interrupt in the way: the gate array's handler would be at &0038
    motherboard.write_buffer(0x38, cpclib_macros::assemble!(" ei : ret"));
    run(&mut motherboard, 10);
    assert!(motherboard.cpu().iff1());

    motherboard.cpu_mut().trigger_nmi();
    motherboard.execute_next_opcode();
    // the next instruction is the handler's first, interrupts off but remembered
    assert_eq!(motherboard.cpu().reg16(Reg16::Pc).value(), 0x66);
    assert!(!motherboard.cpu().iff1());
    assert!(motherboard.cpu().iff2());
    let sp = motherboard.cpu().reg16(Reg16::Sp).value();
    let pushed = motherboard.read_word(sp);
    assert!((0x105..=0x108).contains(&pushed), "returns into the loop, not {pushed:#06x}");

    run(&mut motherboard, 3);
    assert_eq!(motherboard.read_byte(0x8000), 0x42);
    assert!((0x105..=0x108).contains(&motherboard.cpu().reg16(Reg16::Pc).value()));
    assert!(motherboard.cpu().iff1(), "RETN: IFF1 back from IFF2");
}

#[test]
fn an_nmi_wakes_a_halt_and_returns_after_it() {
    let mut motherboard = machine(cpclib_macros::assemble!(" ld sp,0xc000 : di : halt : ld a,7 : loop: jr loop"), 0x100);
    motherboard.write_buffer(0x66, cpclib_macros::assemble!(" retn"));
    run(&mut motherboard, 20);
    let halt = 0x104;
    assert_eq!(motherboard.cpu().reg16(Reg16::Pc).value(), halt, "halted");
    motherboard.cpu_mut().trigger_nmi();
    run(&mut motherboard, 4);
    assert!(motherboard.cpu().reg16(Reg16::Pc).value() > halt, "out of the HALT");
}

#[test]
fn im2_reads_the_vector_of_the_device() {
    // IM 2 with I = &90; the table entry at &9020 points to &5000
    let mut motherboard =
        machine(cpclib_macros::assemble!(" ld sp,0xc000 : ld a,0x90 : ld i,a : im 2 : ei : loop: jr loop"), 0x100);
    motherboard.write_buffer(0x9020, &[0x00, 0x50]);
    motherboard.write_buffer(0x90ff, &[0x00, 0x60]);
    motherboard.write_buffer(0x5000, cpclib_macros::assemble!(" ld a,0x55 : ld (0x8000),a : ei : reti"));
    motherboard.write_buffer(0x6000, cpclib_macros::assemble!(" ld a,0x66 : ld (0x8001),a : ei : reti"));
    motherboard.cpu_mut().set_interrupt_vector(Some(0x20));
    // a frame: the gate array interrupts several times
    run(&mut motherboard, 20_000);
    assert_eq!(motherboard.read_byte(0x8000), 0x55, "through I:vector");
    assert_eq!(motherboard.read_byte(0x8001), 0x00);
    assert!(motherboard.cpu_mut().take_interrupt_acknowledge());

    // no device vector: the floating bus, &FF
    motherboard.cpu_mut().set_interrupt_vector(None);
    run(&mut motherboard, 20_000);
    assert_eq!(motherboard.read_byte(0x8001), 0x66);
}
