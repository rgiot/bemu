//! Checks amsrust's R52-bit5 interrupt-request polarity (ACCC §27.3.2,
//! see `project_crtc_timing_core_rewrite` memory) against a real reference:
//! AMSpiriT's own Shaker "R52 BIT 5 RESET (R2=46/R3=8/CLOSE END HSYNC)"
//! self-test (`BR` module, SSM code 0x01F1) confirms real hardware
//! requests an interrupt right at the R52=0x1F->0x20 transition (every
//! measured value in that test matches its own "Exp:" expectation - no
//! `#FF=NO INT` anywhere). amsrust's current code does the opposite
//! (fires only when bit 5 is *clear*), so it should visibly miss that
//! transition here if that's still true.
//!
//! Sets CRTC0's R2=46 (0x2E), R3=8 (0x08) - the same "close end hsync"
//! configuration AMSpiriT's test uses - via real `OUT (C),C` register
//! writes (matching `tests/crtc_bus.rs`'s proven-correct sequence), then
//! single-steps, logging the GateArray's `r52()` value every time PC
//! enters the IM1 vector (0x0038) - i.e. every accepted interrupt.
//!
//! `cargo run --release --example debug_r52_bit5`

use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};

fn main() {
    let program: [u8; 20] = [
        0x01, 0x02, 0xbc, // LD BC,&BC02
        0xed, 0x49, // OUT (C),C   select R2
        0x01, 46, 0xbd, // LD BC,&BD2E (46)
        0xed, 0x49, // OUT (C),C   write R2=46
        0x01, 0x03, 0xbc, // LD BC,&BC03
        0xed, 0x49, // OUT (C),C   select R3
        0x01, 8, 0xbd, // LD BC,&BD08 (8)
        0xed, 0x49, // OUT (C),C   write R3=8
    ];

    let mut mb = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .enable_interruptions()
        .build();

    mb.write_buffer(0x4000, &program);
    // IM1 handler at 0x0038: just EI+RET so execution resumes normally.
    mb.write_buffer(0x0038, &[0xfb, 0xc9]); // EI, RET
    mb.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    mb.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);

    for _ in 0..program.len() {
        mb.execute_next_opcode_no_video();
    }
    // Idle loop from here: JP $ (infinite loop at current PC), so the
    // only thing that ever happens is interrupts firing and returning.
    let idle_at = mb.cpu().reg16(Reg16::Pc).value();
    mb.write_buffer(idle_at, &[0xc3, (idle_at & 0xff) as u8, (idle_at >> 8) as u8]); // JP idle_at

    eprintln!("R2={} R3={} - idle loop at {:04X}", mb.crtc().r2(), mb.crtc().r3(), idle_at);

    let mut last_pc = mb.pc();
    let mut last_r52 = mb.ga().r52();
    let mut resets = 0u32;
    let mut interrupts = 0u32;
    const MAX_RESETS: u32 = 40;
    const MAX_STEPS: u64 = 50_000_000;
    let mut steps: u64 = 0;

    loop {
        mb.execute_next_opcode_no_video();
        steps += 1;
        let pc = mb.pc();
        let r52 = mb.ga().r52();

        // Capture the value R52 had the instant before it drops (a reset
        // just happened) - checking it *after* acceptance is too late, the
        // reset has already zeroed it out by the time PC reaches 0x0038.
        if r52 < last_r52 {
            resets += 1;
            eprintln!(
                "[step {steps}] R52 reset: was {:#04x} ({}) just before, bit5={}",
                last_r52, last_r52, last_r52 & 0x20 != 0,
            );
            if resets >= MAX_RESETS {
                break;
            }
        }
        last_r52 = r52;

        if pc == 0x0038 && last_pc != 0x0037 {
            interrupts += 1;
        }
        last_pc = pc;

        if steps >= MAX_STEPS {
            eprintln!("gave up after {MAX_STEPS} steps with only {resets} resets / {interrupts} interrupts seen");
            break;
        }
    }
}
