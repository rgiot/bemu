#![cfg(all(feature = "playcity", feature = "digiblaster"))]
//! The PlayCity and the Digiblaster driven by Z80 code, as a program does, heard through the
//! emulator's sound output.

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::psg::AudioConfig;
use amsrust::z80::{HasValue, Reg16};

const RATE: u32 = 48_000;

fn machine(program: &[u8]) -> Motherboard {
    let mut builder = MotherboardBuilder::default();
    builder
        .disable_lower_rom()
        .disable_upper_rom()
        .enable_interruptions()
        .plug_playcity()
        .plug_digiblaster();
    let mut motherboard = builder.build();
    motherboard.write_buffer(0x100, program);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);
    motherboard.set_audio_output(Some(AudioConfig::stereo(RATE)));
    motherboard
}

/// Run for `seconds` and give the left and right samples.
fn listen(motherboard: &mut Motherboard, seconds: f64) -> (Vec<f32>, Vec<f32>) {
    let end = motherboard.t_state_counter().value() + (seconds * 4_000_000.0) as usize;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_no_video();
    }
    let samples = motherboard.take_audio_samples();
    let left = samples.iter().step_by(2).copied().collect();
    let right = samples.iter().skip(1).step_by(2).copied().collect();
    (left, right)
}

/// The frequency of a square-ish wave, from its rising zero crossings (the DC filter centres it).
fn frequency(samples: &[f32]) -> f64 {
    let settled = &samples[samples.len() / 4..];
    let crossings = settled
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    crossings as f64 * f64::from(RATE) / settled.len() as f64
}

fn energy(samples: &[f32]) -> f32 {
    let settled = &samples[samples.len() / 4..];
    settled.iter().map(|s| s * s).sum::<f32>() / settled.len() as f32
}

#[test]
fn the_left_ay_plays_a_tone_at_2_mhz() {
    // the left AY (select &F988, data &F888): channel A, period &100, tone only, volume 15
    let program = cpclib_macros::assemble!(
        "
        di : ld bc,0xf988 : ld a,0 : out (c),a : ld b,0xf8 : ld a,0x00 : out (c),a
        ld b,0xf9 : ld a,1 : out (c),a : ld b,0xf8 : ld a,0x01 : out (c),a
        ld b,0xf9 : ld a,7 : out (c),a : ld b,0xf8 : ld a,0x3e : out (c),a
        ld b,0xf9 : ld a,8 : out (c),a : ld b,0xf8 : ld a,15 : out (c),a
        loop: jr loop
        "
    );
    let mut motherboard = machine(program);
    let (left, right) = listen(&mut motherboard, 0.5);
    // 2 MHz / 16 / 256
    let expected = 2_000_000.0 / 16.0 / 256.0;
    let measured = frequency(&left);
    assert!(
        (measured - expected).abs() < expected * 0.03,
        "{measured} Hz, not {expected}"
    );
    assert!(
        energy(&left) > 100.0 * energy(&right),
        "the left AY is on the left"
    );
}

#[test]
fn ctc_channel_0_changes_the_ay_clock() {
    // channel 0 as a counter of the 4 MHz, constant 4: 1 MHz, halved: 500 kHz
    let program = cpclib_macros::assemble!(
        "
        di : ld bc,0xf880 : ld a,0x47 : out (c),a : ld a,4 : out (c),a
        ld bc,0xf984 : ld a,0 : out (c),a : ld b,0xf8 : ld a,0x40 : out (c),a
        ld b,0xf9 : ld a,7 : out (c),a : ld b,0xf8 : ld a,0x3e : out (c),a
        ld b,0xf9 : ld a,8 : out (c),a : ld b,0xf8 : ld a,15 : out (c),a
        loop: jr loop
        "
    );
    let mut motherboard = machine(program);
    let (left, right) = listen(&mut motherboard, 0.5);
    let expected = 500_000.0 / 16.0 / 64.0;
    let measured = frequency(&right);
    assert!(
        (measured - expected).abs() < expected * 0.03,
        "{measured} Hz, not {expected}"
    );
    assert!(
        energy(&right) > 100.0 * energy(&left),
        "the right AY is on the right"
    );
}

#[test]
fn the_digiblaster_plays_what_is_written_to_the_printer_port() {
    // a square wave: +&40 then -&40, about every 100 us
    let program = cpclib_macros::assemble!(
        "
        di : ld bc,0xef00
        loop: ld a,0x40 : out (c),a : ld d,20 : w1: dec d : jr nz,w1
              ld a,0xc0 : out (c),a : ld d,20 : w2: dec d : jr nz,w2
              jr loop
        "
    );
    let mut motherboard = machine(program);
    let (left, right) = listen(&mut motherboard, 0.3);
    let measured = frequency(&left);
    assert!(measured > 1000.0 && measured < 10_000.0, "{measured} Hz");
    assert!((energy(&left) - energy(&right)).abs() < 1e-6, "mono");
    assert!(energy(&left) > 0.01);
}

#[test]
fn ctc_channel_1_counts_the_crtc_cursor_to_nmis() {
    // the cursor: on the character at &3000 + 10 rows of 40 (R12/R13 at reset: &3000), its 8
    // lines (R10 = 0, steady, R11 = 7); channel 1 counts its rising edges, an NMI every 4
    let program = cpclib_macros::assemble!(
        "
        ld sp,0xc000 : di
        ld bc,0xbc0e : out (c),c : ld bc,0xbd31 : out (c),c
        ld bc,0xbc0f : out (c),c : ld bc,0xbd90 : out (c),c
        ld bc,0xbc0a : out (c),c : ld bc,0xbd00 : out (c),c
        ld bc,0xbc0b : out (c),c : ld bc,0xbd07 : out (c),c
        ld bc,0xf881 : ld a,0x57 : out (c),a : ld a,4 : out (c),a
        loop: jr loop
        "
    );
    let mut motherboard = machine(program);
    motherboard.write_buffer(
        0x66,
        cpclib_macros::assemble!(
            " push hl : ld hl,(0x8000) : inc hl : ld (0x8000),hl : pop hl : retn"
        ),
    );
    // one second: 50 frames, 8 cursor lines each
    listen(&mut motherboard, 1.0);
    let nmis = motherboard.read_word(0x8000);
    let expected = 50 * 8 / 4;
    assert!(
        (i32::from(nmis) - expected).abs() <= 2,
        "{nmis} NMIs, not {expected}"
    );
}

#[test]
fn ctc_timer_interrupts_through_its_im2_vector() {
    // vector &20; channel 2: timer, prescaler 256, interrupts, constant 250 (62.5 Hz)
    let program = cpclib_macros::assemble!(
        "
        ld sp,0xc000 : ld a,0x90 : ld i,a : im 2
        ld bc,0xf880 : ld a,0x20 : out (c),a
        ld bc,0xf882 : ld a,0xa5 : out (c),a : ld a,250 : out (c),a
        ei
        loop: jr loop
        "
    );
    let mut motherboard = machine(program);
    // channel 2's entry: vector | 2 << 1 = &24; the gate array's interrupts read &FF
    motherboard.write_buffer(0x9024, &[0x00, 0x50]);
    motherboard.write_buffer(0x90ff, &[0x00, 0x60]);
    motherboard.write_buffer(0x6000, cpclib_macros::assemble!(" ei : reti"));
    motherboard.write_buffer(
        0x5000,
        cpclib_macros::assemble!(
            " push hl : ld hl,(0x8000) : inc hl : ld (0x8000),hl : pop hl : ei : reti"
        ),
    );
    listen(&mut motherboard, 1.0);
    let interrupts = motherboard.read_word(0x8000);
    assert!(
        (i32::from(interrupts) - 62).abs() <= 2,
        "{interrupts} interrupts, not 62"
    );
}
