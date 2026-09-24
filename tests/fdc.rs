///! FDC (NEC uPD765A/B) protocol tests, cross-checked against the datasheet
///! (doc/fdc.upd765ab.pdf): command/result byte layout, status register
///! bits, and the RQM/DIO handshake for a Read Data command end to end.

use amsrust::fdc::{Fdc, FdcDrive};
use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};
use cpclib_disc::builder::single_head_data_dsk;
use cpclib_disc::disc::Disc;
use cpclib_disc::edsk::Head;
use cpclib_disc::AnyDisc;

const RQM: u8 = 1 << 7;
const DIO: u8 = 1 << 6;

fn disk_with_known_sector() -> (AnyDisc, u8, u16) {
    let mut dsk = single_head_data_dsk();
    let sector_id = dsk.track_min_sector(Head::A, 0).expect("track 0 must be formatted");
    let bytes = dsk
        .sector_read_bytes(Head::A, 0, sector_id)
        .expect("sector must exist");
    let mut content = vec![0u8; bytes.len()];
    for (i, b) in content.iter_mut().enumerate() {
        *b = (i % 256) as u8;
    }
    dsk.sector_write_bytes(Head::A, 0, sector_id, &content)
        .unwrap();
    (AnyDisc::Edsk(dsk), sector_id, content.len() as u16)
}

fn size_code_for_len(len: usize) -> u8 {
    let mut n = 0u8;
    let mut size = 128usize;
    while size < len {
        size <<= 1;
        n += 1;
    }
    n
}

#[test]
fn read_data_full_roundtrip() {
    let (disk, sector_id, sector_len) = disk_with_known_sector();
    let n = size_code_for_len(sector_len as usize);

    let mut fdc = Fdc::default();
    fdc.insert_disk(FdcDrive::A, Some(disk));
    fdc.set_motor(true);

    // Command phase: READ DATA (0x06), unit=0/head=0, C=0,H=0,R=sector_id,
    // N=size code, EOT=sector_id (stop after this one sector), GPL, DTL.
    let command = [0x06, 0x00, 0x00, 0x00, sector_id, n, sector_id, 0x2a, 0xff];
    for &byte in &command {
        assert!(
            fdc.read_status() & RQM != 0,
            "FDC should be ready to accept a command byte"
        );
        fdc.write_data(byte);
    }

    // Execution phase: FDC -> CPU (DIO=1), sector_len bytes to read.
    assert_eq!(fdc.read_status() & DIO, DIO);
    let mut received = Vec::new();
    for _ in 0..sector_len {
        assert!(fdc.read_status() & RQM != 0, "byte should be ready");
        received.push(fdc.read_data());
    }
    let expected: Vec<u8> = (0..sector_len).map(|i| (i as usize % 256) as u8).collect();
    assert_eq!(received, expected);

    // Result phase: ST0, ST1, ST2, C, H, R, N.
    let mut result = Vec::new();
    for _ in 0..7 {
        result.push(fdc.read_data());
    }
    assert_eq!(result[0], 0x00, "ST0 should report normal termination");
    assert_eq!(result[1], 0x00, "ST1 should report no error");
    assert_eq!(result[2], 0x00, "ST2 should report no error");
    assert_eq!(result[3], 0x00, "result C");
    assert_eq!(result[4], 0x00, "result H");
    assert_eq!(result[5], sector_id, "result R");
    assert_eq!(result[6], n, "result N");

    // Back to command phase, ready for a new command.
    assert_eq!(fdc.read_status(), RQM);
}

#[test]
fn read_data_sector_not_found_reports_no_data() {
    let (disk, sector_id, _len) = disk_with_known_sector();
    let missing_sector = sector_id.wrapping_add(200); // not present on the disk

    let mut fdc = Fdc::default();
    fdc.insert_disk(FdcDrive::A, Some(disk));
    fdc.set_motor(true);

    let command = [0x06, 0x00, 0x00, 0x00, missing_sector, 0x02, missing_sector, 0x2a, 0xff];
    for &byte in &command {
        fdc.write_data(byte);
    }

    let mut result = Vec::new();
    for _ in 0..7 {
        result.push(fdc.read_data());
    }
    assert_eq!(result[0] & 0xc0, 0x40, "ST0 should report abnormal termination");
    assert_eq!(result[1] & 0x04, 0x04, "ST1 ND (no data) bit should be set");
}

#[test]
fn read_data_without_disk_reports_not_ready() {
    let mut fdc = Fdc::default();
    fdc.set_motor(true);
    // No disk inserted at all.

    let command = [0x06, 0x00, 0x00, 0x00, 0x01, 0x02, 0x01, 0x2a, 0xff];
    for &byte in &command {
        fdc.write_data(byte);
    }

    let mut result = Vec::new();
    for _ in 0..7 {
        result.push(fdc.read_data());
    }
    assert_eq!(result[0] & 0x08, 0x08, "ST0 NR (not ready) bit should be set");
}

#[test]
fn seek_and_sense_interrupt_status() {
    let mut fdc = Fdc::default();
    fdc.set_motor(true);

    // SEEK to track 5, unit 0.
    for &byte in &[0x0f, 0x00, 0x05] {
        fdc.write_data(byte);
    }
    // Seek has no result phase - FDC goes straight back to command phase,
    // but unit 0's "busy" bit stays set until Sense Interrupt Status
    // clears it (Table 2 of the datasheet: DnB busy while a seek's
    // completion interrupt is still pending).
    assert_eq!(fdc.read_status(), RQM | 0x01);

    // SENSE INTERRUPT STATUS clears the pending seek-end and reports it.
    fdc.write_data(0x08);
    let st0 = fdc.read_data();
    let pcn = fdc.read_data();
    assert_eq!(st0 & 0x20, 0x20, "ST0 SE (seek end) bit should be set");
    assert_eq!(pcn, 5, "PCN should report the new track");

    // A second SIS with nothing pending reports invalid command.
    fdc.write_data(0x08);
    let st0_again = fdc.read_data();
    let _ = fdc.read_data();
    assert_eq!(st0_again, 0x80, "ST0 should report invalid command");
}

#[test]
fn recalibrate_resets_track_to_zero() {
    let mut fdc = Fdc::default();
    fdc.set_motor(true);

    for &byte in &[0x0f, 0x00, 0x1f] {
        fdc.write_data(byte);
    }
    fdc.write_data(0x08); // clear the seek-end interrupt
    let _ = fdc.read_data();
    let _ = fdc.read_data();

    for &byte in &[0x07, 0x00] {
        fdc.write_data(byte);
    }
    assert_eq!(fdc.read_status(), RQM | 0x01);

    fdc.write_data(0x08);
    let st0 = fdc.read_data();
    let pcn = fdc.read_data();
    assert_eq!(st0 & 0x20, 0x20, "ST0 SE bit should be set");
    assert_eq!(pcn, 0, "recalibrate should bring the head back to track 0");
}

#[test]
fn invalid_command_reports_st0_80() {
    let mut fdc = Fdc::default();
    fdc.set_motor(true);

    // 0x0D is FORMAT TRACK, deliberately unsupported.
    fdc.write_data(0x0d);
    assert_eq!(fdc.read_data(), 0x80);
    // Back to command phase, ready for the next command.
    assert_eq!(fdc.read_status(), RQM);
}

/// Same Read Data command as `read_data_full_roundtrip`, but driven
/// entirely through real `OUT`/`IN` Z80 instructions against the Motherboard,
/// to check the &FA7E/&FB7E/&FB7F address decode wired up in motherboard.rs
/// (not just the `Fdc` struct in isolation).
#[test]
fn motherboard_reads_disk_through_real_io_ports() {
    let (disk, sector_id, sector_len) = disk_with_known_sector();
    let n = size_code_for_len(sector_len as usize);

    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    motherboard.fdc_mut().insert_disk(FdcDrive::A, Some(disk));

    let mut prog = Vec::new();
    let mut nb_instructions = 0usize;

    let mut emit_out = |prog: &mut Vec<u8>, port: u16, value: u8| {
        prog.extend_from_slice(&[0x01, (port & 0xff) as u8, (port >> 8) as u8]); // LD BC,port
        prog.extend_from_slice(&[0x3e, value]); // LD A,value
        prog.extend_from_slice(&[0xed, 0x79]); // OUT (C),A
        nb_instructions += 3;
    };
    emit_out(&mut prog, 0xfa7e, 1); // motor on
    for &b in &[0x06, 0x00, 0x00, 0x00, sector_id, n, sector_id, 0x2a, 0xff] {
        emit_out(&mut prog, 0xfb7f, b);
    }

    let result_buffer: u16 = 0x8000;
    let total = sector_len + 7;
    for i in 0..total {
        let dest = result_buffer + i;
        prog.extend_from_slice(&[0x01, 0x7f, 0xfb]); // LD BC,0xfb7f
        prog.extend_from_slice(&[0xed, 0x78]); // IN A,(C)
        prog.extend_from_slice(&[0x32, (dest & 0xff) as u8, (dest >> 8) as u8]); // LD (dest),A
        nb_instructions += 3;
    }
    prog.extend_from_slice(&[0x18, 0xfe]); // jr $ (never reached)

    motherboard.write_buffer(0x4000, &prog);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);

    for _ in 0..nb_instructions {
        motherboard.execute_next_opcode_no_video();
    }

    for i in 0..sector_len {
        assert_eq!(
            motherboard.read_byte(result_buffer + i),
            (i % 256) as u8,
            "data byte {i}"
        );
    }
    assert_eq!(motherboard.read_byte(result_buffer + sector_len), 0x00, "ST0");
    assert_eq!(motherboard.read_byte(result_buffer + sector_len + 5), sector_id, "result R");
}
