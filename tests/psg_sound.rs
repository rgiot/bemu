///! PSG sound through the whole machine: a Z80 program programs the PSG
///! via the PPI exactly like the firmware (data on port A, BDIR/BC1 on port
///! C bits 7/6), and the sound must come out at the pitch the datasheet
///! gives for the CPC's 1MHz PSG clock (the gate array's CCLK).

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::psg::AudioConfig;
use amsrust::z80::{HasValue, Reg16};

const PORT_A: u8 = 0xf4;
const PORT_C: u8 = 0xf6;

/// LD BC,B<<8; LD A,val; OUT (C),A
fn emit_out(prog: &mut Vec<u8>, port_high: u8, value: u8) {
    prog.extend_from_slice(&[0x01, 0x00, port_high]); // LD BC,port_high*256
    prog.extend_from_slice(&[0x3e, value]); // LD A,value
    prog.extend_from_slice(&[0xed, 0x79]); // OUT (C),A
}

/// Select a register then write it, as the firmware's MC SOUND REGISTER.
fn emit_psg_write(prog: &mut Vec<u8>, reg: u8, value: u8) {
    emit_out(prog, PORT_A, reg);
    emit_out(prog, PORT_C, 0xc0); // BDIR=1,BC1=1 -> latch address
    emit_out(prog, PORT_C, 0x00); // inactive
    emit_out(prog, PORT_A, value);
    emit_out(prog, PORT_C, 0x80); // BDIR=1,BC1=0 -> write
    emit_out(prog, PORT_C, 0x00); // inactive
}

fn machine_playing(writes: &[(u8, u8)], config: AudioConfig) -> Motherboard {
    let mut prog = Vec::new();
    for &(reg, value) in writes {
        emit_psg_write(&mut prog, reg, value);
    }
    prog.extend_from_slice(&[0x18, 0xfe]); // JR $ - loop forever

    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    motherboard.write_buffer(0x4000, &prog);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
    motherboard.set_audio_output(Some(config));
    motherboard
}

/// Run `seconds` of emulated time, returning the samples produced.
fn record(motherboard: &mut Motherboard, seconds: f64) -> Vec<f32> {
    let end = motherboard.t_state_counter().value() + (seconds * 4_000_000.0) as usize;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_no_video();
    }
    motherboard.take_audio_samples()
}

#[test]
fn a_tone_programmed_through_the_ppi_plays_at_fclock_over_16_tp() {
    // TP = 0x11c = 284: 1MHz / (16 * 284) = 220.07Hz.
    let mut motherboard = machine_playing(
        &[(0, 0x1c), (1, 0x01), (7, 0b0011_1110), (8, 15)],
        AudioConfig::mono(48_000)
    );
    record(&mut motherboard, 0.1); // let the program run
    assert_eq!(motherboard.psg().register(0), 0x1c);
    assert_eq!(motherboard.psg().register(1), 0x01);
    assert_eq!(motherboard.psg().register(7), 0x3e);
    assert_eq!(motherboard.psg().register(8), 15);

    let samples = record(&mut motherboard, 1.0);
    // Exactly the requested rate.
    assert_eq!(samples.len(), 48_000);
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    let crossings = (1..samples.len())
        .filter(|&i| samples[i - 1] < mean && samples[i] >= mean)
        .count();
    // 220.07Hz over 1s.
    assert!((219..=221).contains(&crossings), "{crossings}");
}

#[test]
fn silence_when_nothing_is_programmed() {
    let mut motherboard = machine_playing(&[], AudioConfig::stereo(44_100));
    let samples = record(&mut motherboard, 0.5);
    assert_eq!(samples.len(), 2 * 22_050);
    assert!(samples.iter().all(|&s| s == 0.0));
}

#[test]
fn no_audio_output_unless_requested() {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    for _ in 0..10_000 {
        motherboard.execute_next_opcode_no_video();
    }
    assert!(motherboard.take_audio_samples().is_empty());
}
