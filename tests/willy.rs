#![cfg(feature = "willy")]
//! Willy on the expansion port, driven by Z80 programs as the CPC would: the OPL3LPT's tone, the
//! S2P's detection (status &80, &3F answered by &FE) and a note through a small SoundFont made
//! here (one looped sine), so no big file is needed.

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::expansion_port::ExpansionDevice;
use amsrust::willy::{Opl3Lpt, S2p, OPL3_BASE, S2P_BASE};

/// A machine running `code` from &4000, interrupts off.
fn machine(code: &[u8]) -> Motherboard {
    let mut motherboard = MotherboardBuilder::default().build();
    for (i, byte) in code.iter().enumerate() {
        motherboard.write_byte(0x4000 + i as u16, *byte);
    }
    motherboard.cpu_mut().set_pc(0x4000);
    motherboard
}

fn run(motherboard: &mut Motherboard, microseconds: usize, mut each: impl FnMut(&Motherboard)) {
    let end = motherboard.t_state_counter().value() + microseconds * 4;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_no_video();
        each(motherboard);
    }
}

/// `LD BC,port : LD A,value : OUT (C),A` for each write, then `DI : HALT`... a loop on itself.
fn writes(pairs: &[(u16, u8)]) -> Vec<u8> {
    let mut code = vec![0xf3];
    for &(port, value) in pairs {
        code.extend([0x01, port as u8, (port >> 8) as u8, 0x3e, value, 0xed, 0x79]);
    }
    code.extend([0x18, 0xfe]);
    code
}

#[test]
fn a_z80_program_plays_the_opl3() {
    // shipbrook.net's tone: channel 1 at 440 Hz
    let tone = [(0x20, 0x01), (0x40, 0x10), (0x60, 0xf0), (0x80, 0x77), (0xa0, 0x98), (0x23, 0x01), (0x63, 0xf0), (0x83, 0x77), (0xb0, 0x31)];
    let pairs: Vec<(u16, u8)> = tone.iter().flat_map(|&(register, value)| [(OPL3_BASE, register), (OPL3_BASE + 1, value)]).collect();
    let mut motherboard = machine(&writes(&pairs));
    motherboard.plug_expansion(Box::new(Opl3Lpt::new(OPL3_BASE)));
    let mut loudest = 0.0f32;
    run(&mut motherboard, 30_000, |motherboard| {
        let sound = motherboard.expansion_port().unwrap().device::<Opl3Lpt>().unwrap().sound();
        loudest = loudest.max(sound[0].abs());
    });
    let card = motherboard.expansion_port().unwrap().device::<Opl3Lpt>().unwrap();
    assert_eq!(card.register_writes(), 9);
    assert!(loudest > 0.01, "the tone: {loudest}");
}

/// A SoundFont 2 with one preset (0), one instrument, one looped 441 Hz sine.
fn sine_soundfont() -> Vec<u8> {
    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }
    fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut data = kind.to_vec();
        for c in chunks {
            data.extend(c);
        }
        chunk(b"LIST", &data)
    }
    fn name(text: &str) -> Vec<u8> {
        let mut out = text.as_bytes().to_vec();
        out.resize(20, 0);
        out
    }
    let le16 = |v: u16| v.to_le_bytes().to_vec();
    let le32 = |v: u32| v.to_le_bytes().to_vec();
    const LENGTH: u32 = 1000;
    let mut samples = Vec::new();
    for i in 0..LENGTH + 46 {
        let value = if i < LENGTH { ((i as f32 / 100.0 * std::f32::consts::TAU).sin() * 16000.0) as i16 } else { 0 };
        samples.extend(value.to_le_bytes());
    }
    let info = list(b"INFO", &[chunk(b"ifil", &[2, 0, 1, 0]), chunk(b"isng", b"EMU8000\0"), chunk(b"INAM", b"bemu test\0")]);
    let sdta = list(b"sdta", &[chunk(b"smpl", &samples)]);
    let phdr = [name("Sine"), le16(0), le16(0), le16(0), le32(0), le32(0), le32(0), name("EOP"), le16(0), le16(0), le16(1), le32(0), le32(0), le32(0)].concat();
    let pbag = [le16(0), le16(0), le16(1), le16(0)].concat();
    let pmod = vec![0u8; 10];
    let pgen = [le16(41), le16(0), le16(0), le16(0)].concat();
    let inst = [name("Sine"), le16(0), name("EOI"), le16(1)].concat();
    let ibag = [le16(0), le16(0), le16(2), le16(0)].concat();
    let imod = vec![0u8; 10];
    let igen = [le16(54), le16(1), le16(53), le16(0), le16(0), le16(0)].concat();
    let shdr = [
        name("Sine"), le32(0), le32(LENGTH), le32(0), le32(LENGTH), le32(44_100), vec![69, 0], le16(0), le16(1),
        name("EOS"), le32(0), le32(0), le32(0), le32(0), le32(0), vec![0, 0], le16(0), le16(0)
    ]
    .concat();
    let pdta = list(b"pdta", &[
        chunk(b"phdr", &phdr), chunk(b"pbag", &pbag), chunk(b"pmod", &pmod), chunk(b"pgen", &pgen),
        chunk(b"inst", &inst), chunk(b"ibag", &ibag), chunk(b"imod", &imod), chunk(b"igen", &igen), chunk(b"shdr", &shdr)
    ]);
    let mut body = b"sfbk".to_vec();
    body.extend(info);
    body.extend(sdta);
    body.extend(pdta);
    chunk(b"RIFF", &body)
}

#[test]
fn a_z80_program_detects_the_s2p_and_plays_a_note() {
    // the docs' detection: the status (&80 idle), &3F to the control, then the status (&C0, a
    // byte to read) and the byte (&FE); stored at &5000-&5002
    let mut code = vec![0xf3];
    code.extend([0x01, (S2P_BASE + 1) as u8, (S2P_BASE >> 8) as u8, 0xed, 0x78, 0x32, 0x00, 0x50]); // IN A,(C) : LD (&5000),A
    code.extend([0x3e, 0x3f, 0xed, 0x79]); // OUT (C),&3F
    code.extend([0xed, 0x78, 0x32, 0x01, 0x50]); // the status again
    code.extend([0x01, S2P_BASE as u8, (S2P_BASE >> 8) as u8, 0xed, 0x78, 0x32, 0x02, 0x50]); // the byte
    // note on: &90 &3C &7F
    for value in [0x90, 0x3c, 0x7f] {
        code.extend([0x3e, value, 0xed, 0x79]);
    }
    code.extend([0x18, 0xfe]);
    let mut motherboard = machine(&code);
    motherboard.plug_expansion(Box::new(S2p::new(S2P_BASE, &sine_soundfont()).unwrap()));
    let mut loudest = 0.0f32;
    run(&mut motherboard, 100_000, |motherboard| {
        let sound = motherboard.expansion_port().unwrap().device::<S2p>().unwrap().sound();
        loudest = loudest.max(sound[0].abs());
    });
    assert_eq!(motherboard.read_byte(0x5000), 0x80, "idle");
    assert_eq!(motherboard.read_byte(0x5001), 0xc0, "a byte to read");
    assert_eq!(motherboard.read_byte(0x5002), 0xfe, "UART mode");
    assert_eq!(motherboard.expansion_port().unwrap().device::<S2p>().unwrap().messages(), 1);
    assert!(loudest > 0.01, "the note: {loudest}");
}
