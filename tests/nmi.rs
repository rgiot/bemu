//! Non-maskable interrupts (an expansion's /NMI, e.g. the PlayCity's CTC) and the IM 2 vector
//! an interrupting device puts on the bus.

use amsrust::chip::Chip;
use amsrust::expansion_port::{ExpansionDevice, ExpansionPin, ExpansionPort};
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

/// A card that answers every interrupt acknowledge with a vector, and pulls /NMI on demand.
struct TestCard {
    vector: u8,
    nmi: bool
}

impl ExpansionDevice for TestCard {
    fn tick(&mut self, port: &mut ExpansionPort) {
        if port.interrupt_acknowledge() {
            port.drive_data(self.vector);
        }
        if self.nmi {
            port.assert(ExpansionPin::Neg_NMI);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn im2_reads_the_vector_a_card_puts_on_the_bus() {
    // IM 2 with I = &90; the table entry at &9020 points to &5000, the one at &90FF to &6000
    let program = cpclib_macros::assemble!(" ld sp,0xc000 : ld a,0x90 : ld i,a : im 2 : ei : loop: jr loop");
    let handlers = |motherboard: &mut Motherboard| {
        motherboard.write_buffer(0x9020, &[0x00, 0x50]);
        motherboard.write_buffer(0x90ff, &[0x00, 0x60]);
        motherboard.write_buffer(0x5000, cpclib_macros::assemble!(" ld a,0x55 : ld (0x8000),a : ei : reti"));
        motherboard.write_buffer(0x6000, cpclib_macros::assemble!(" ld a,0x66 : ld (0x8001),a : ei : reti"));
    };
    let mut motherboard = machine(program, 0x100);
    handlers(&mut motherboard);
    motherboard.plug_expansion(Box::new(TestCard { vector: 0x20, nmi: false }));
    // a frame: the gate array interrupts several times
    run(&mut motherboard, 20_000);
    assert_eq!(motherboard.read_byte(0x8000), 0x55, "through I:vector");
    assert_eq!(motherboard.read_byte(0x8001), 0x00);

    // no card: the floating bus, &FF
    let mut motherboard = machine(program, 0x100);
    handlers(&mut motherboard);
    run(&mut motherboard, 20_000);
    assert_eq!(motherboard.read_byte(0x8000), 0x00);
    assert_eq!(motherboard.read_byte(0x8001), 0x66);
}

#[test]
fn a_card_pulling_nmi_calls_0066_once() {
    let mut motherboard = machine(cpclib_macros::assemble!(" ld sp,0xc000 : di : loop: jr loop"), 0x100);
    motherboard.write_buffer(
        0x66,
        cpclib_macros::assemble!(" push hl : ld hl,(0x8000) : inc hl : ld (0x8000),hl : pop hl : retn")
    );
    motherboard.plug_expansion(Box::new(TestCard { vector: 0xff, nmi: false }));
    run(&mut motherboard, 100);
    fn card(motherboard: &mut Motherboard) -> &mut TestCard {
        motherboard.expansion_port_mut().unwrap().device_mut::<TestCard>().unwrap()
    }
    card(&mut motherboard).nmi = true;
    run(&mut motherboard, 100);
    assert_eq!(motherboard.read_word(0x8000), 1, "edge-triggered: held low, one NMI");
    card(&mut motherboard).nmi = false;
    run(&mut motherboard, 10);
    card(&mut motherboard).nmi = true;
    run(&mut motherboard, 100);
    assert_eq!(motherboard.read_word(0x8000), 2);
}
