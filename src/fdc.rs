//! NEC uPD765A/B (aka "FDC") emulation - the floppy disk controller used by
//! the CPC's disk interface.
//!
//! This is *not* a pin-level `Chip` like the CRTC/PPI/GA: the real uPD765's
//! interesting behaviour (command/execution/result phases, status
//! registers) is a byte-level protocol over its two registers, and the CPC
//! never uses DMA - the Z80 always transfers data with plain `IN`/`OUT`, one
//! byte per port access. There's nothing to gain from ticking it every
//! CPU cycle, so it's driven directly by `Motherboard`'s I/O dispatch via
//! `read_status`/`read_data`/`write_data`/`set_motor`.
//!
//! Command/status semantics below are cross-checked against the NEC
//! uPD765A/B datasheet (`doc/fdc.upd765ab.pdf`), not just against reference
//! emulator source - the datasheet is the authority when they'd disagree.
//!
//! Unlike a real drive, seeks and rotational latency are instantaneous:
//! firmware only depends on the eventual status/result bytes being correct,
//! not on how many microseconds they took, and modelling gap/index timing
//! would only matter for copy-protection schemes this emulator doesn't aim
//! to support (yet).

use cpclib_disc::disc::Disc;
use cpclib_disc::edsk::Head;
use cpclib_disc::AnyDisc;

/// Main status register bit positions (datasheet Table 2).
mod st_bit {
    pub const RQM: u8 = 7; // Request for Master: data register ready
    pub const DIO: u8 = 6; // Data I/O direction: 1 = FDC -> CPU
    pub const EXM: u8 = 5; // Execution mode (non-DMA)
    pub const CB: u8 = 4; // FDC busy
}

/// ST0 bit positions (datasheet Table 3).
mod st0_bit {
    pub const SE: u8 = 5; // Seek End
    pub const EC: u8 = 4; // Equipment Check
    pub const NR: u8 = 3; // Not Ready
}
/// ST0 D7-D6 "interrupt code" field values.
mod st0_ic {
    pub const NORMAL: u8 = 0x00;
    pub const ABNORMAL: u8 = 0x40;
    pub const INVALID: u8 = 0x80;
}

/// ST1 bit positions.
mod st1_bit {
    pub const ND: u8 = 2; // No Data (sector not found)
}

/// ST3 bit positions.
mod st3_bit {
    pub const WP: u8 = 6; // Write protected
    pub const RY: u8 = 5; // Ready
    pub const T0: u8 = 4; // Track 0
    pub const TS: u8 = 3; // Two-sided
}

/// Which drive unit a CSL `disk_insert`/an FDC command targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FdcDrive {
    #[default]
    A,
    B
}

impl FdcDrive {
    fn index(self) -> usize {
        match self {
            FdcDrive::A => 0,
            FdcDrive::B => 1
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Command,
    Execution,
    Result
}

/// The 8 commands this FDC understands - everything AMSDOS/the firmware
/// disk ROM actually issues to read/run files from a disk. Anything else
/// (Format, Read Track, Scan*, Read/Write Deleted Data) reports "invalid
/// command", exactly like real hardware would for a command it didn't
/// recognize - not a crash, since firmware occasionally probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmd {
    ReadData,
    WriteData,
    ReadId,
    Recalibrate,
    SenseInterruptStatus,
    Specify,
    SenseDriveStatus,
    Seek,
    Invalid
}

impl Cmd {
    /// `byte` is the very first command-phase byte (the command code,
    /// still carrying its MT/MF/SK flag bits in D7-D5 for the commands
    /// that have them).
    fn from_byte(byte: u8) -> Self {
        match byte & 0x1f {
            0x06 => Cmd::ReadData,
            0x05 => Cmd::WriteData,
            0x0a => Cmd::ReadId,
            0x07 => Cmd::Recalibrate,
            0x08 => Cmd::SenseInterruptStatus,
            0x03 => Cmd::Specify,
            0x04 => Cmd::SenseDriveStatus,
            0x0f => Cmd::Seek,
            _ => Cmd::Invalid
        }
    }

    /// Total command-phase length (command code byte included).
    fn command_len(self) -> usize {
        match self {
            Cmd::ReadData | Cmd::WriteData => 9,
            Cmd::ReadId => 2,
            Cmd::Recalibrate => 2,
            Cmd::SenseInterruptStatus => 1,
            Cmd::Specify => 3,
            Cmd::SenseDriveStatus => 2,
            Cmd::Seek => 3,
            Cmd::Invalid => 1
        }
    }

    /// Result-phase length. 0 means the command goes straight back to the
    /// command phase with no result bytes at all (Recalibrate/Seek/Specify).
    fn result_len(self) -> usize {
        match self {
            Cmd::ReadData | Cmd::WriteData => 7,
            Cmd::ReadId => 7,
            Cmd::Recalibrate | Cmd::Seek | Cmd::Specify => 0,
            Cmd::SenseInterruptStatus => 2,
            Cmd::SenseDriveStatus => 1,
            Cmd::Invalid => 1
        }
    }

    /// Whether this command has a data-transfer execution phase at all
    /// (Read/Write Data); everything else jumps straight from command
    /// phase to its result (or back to command phase).
    fn has_data_execution_phase(self) -> bool {
        matches!(self, Cmd::ReadData | Cmd::WriteData)
    }

    /// Execution-phase transfer direction: true = FDC drives the data bus
    /// (a read), false = the CPU does (a write).
    fn to_cpu(self) -> bool {
        !matches!(self, Cmd::WriteData)
    }
}

// Command-phase byte indices (datasheet Table 4), valid for Read/Write Data.
const CMD_CODE: usize = 0;
const CMD_UNIT: usize = 1;
const CMD_TRACK: usize = 2;
const CMD_HEAD: usize = 3;
const CMD_SECTOR: usize = 4;
const CMD_SIZE: usize = 5;
const CMD_EOT: usize = 6;
const CMD_DTL: usize = 8;

#[derive(Default)]
struct DriveState {
    disk: Option<AnyDisc>,
    current_track: u8
}

pub struct Fdc {
    motor: bool,
    drives: [DriveState; 2],
    active_drive: usize,

    st: u8,
    st0: u8,
    st1: u8,
    st2: u8,

    phase: Phase,
    current_cmd: Cmd,
    command: Vec<u8>,
    command_len: usize,

    result: Vec<u8>,
    result_pos: usize,

    /// Bytes of the sector currently being transferred in the execution
    /// phase (the *whole* sector is fetched from the disk image up front;
    /// `read_data`/`write_data` just walk through it one byte at a time,
    /// matching what the real chip does one bit-cell at a time).
    exec_data: Vec<u8>,
    exec_pos: usize
}

impl Default for Fdc {
    fn default() -> Self {
        let mut fdc = Self {
            motor: false,
            drives: Default::default(),
            active_drive: 0,
            st: 0,
            st0: 0,
            st1: 0,
            st2: 0,
            phase: Phase::Command,
            current_cmd: Cmd::Invalid,
            command: Vec::new(),
            command_len: 1,
            result: Vec::new(),
            result_pos: 0,
            exec_data: Vec::new(),
            exec_pos: 0
        };
        fdc.reset();
        fdc
    }
}

impl Fdc {
    pub fn reset(&mut self) {
        self.motor = false;
        self.active_drive = 0;
        self.st0 = 0;
        self.st1 = 0;
        self.st2 = 0;
        self.command.clear();
        self.command_len = 1;
        self.current_cmd = Cmd::Invalid;
        self.result.clear();
        self.result_pos = 0;
        self.exec_data.clear();
        self.exec_pos = 0;
        for drive in &mut self.drives {
            drive.current_track = 0;
        }
        self.go_to_command_phase();
    }

    pub fn set_motor(&mut self, on: bool) {
        self.motor = on;
    }

    pub fn motor(&self) -> bool {
        self.motor
    }

    /// Put a disk image in a drive (an empty drive - `None` - ejects it).
    pub fn insert_disk(&mut self, drive: FdcDrive, disk: Option<AnyDisc>) {
        self.drives[drive.index()].disk = disk;
    }

    pub fn has_disk(&self, drive: FdcDrive) -> bool {
        self.drives[drive.index()].disk.is_some()
    }

    // ---- Register access (called directly from Motherboard's I/O dispatch) ----

    pub fn read_status(&self) -> u8 {
        self.st
    }

    pub fn write_data(&mut self, val: u8) {
        if !self.get_bit(st_bit::RQM) || self.get_bit(st_bit::DIO) {
            // Not our turn to accept a byte from the CPU.
            return;
        }

        match self.phase {
            Phase::Command => self.write_command_byte(val),
            Phase::Execution => {
                if !self.current_cmd.to_cpu() {
                    if self.exec_pos < self.exec_data.len() {
                        self.exec_data[self.exec_pos] = val;
                    }
                    self.exec_pos += 1;
                    self.clear_rqm();
                    self.advance_write_execution();
                }
            },
            Phase::Result => {
                // The CPU has nothing to write during the result phase.
            }
        }
    }

    pub fn read_data(&mut self) -> u8 {
        match self.phase {
            Phase::Execution => {
                if self.get_bit(st_bit::RQM) && self.current_cmd.to_cpu() {
                    let byte = self.exec_data.get(self.exec_pos).copied().unwrap_or(0xe5);
                    self.exec_pos += 1;
                    self.clear_rqm();
                    self.advance_read_execution();
                    byte
                }
                else {
                    0xff
                }
            },
            Phase::Result => {
                let byte = self.result.get(self.result_pos).copied().unwrap_or(0xff);
                self.result_pos += 1;
                if self.result_pos >= self.result.len() {
                    self.go_to_command_phase();
                }
                byte
            },
            Phase::Command => 0xff
        }
    }

    // ---- Status register helpers ----

    fn get_bit(&self, bit: u8) -> bool {
        self.st & (1 << bit) != 0
    }

    fn set_bit(&mut self, bit: u8, val: bool) {
        if val {
            self.st |= 1 << bit;
        }
        else {
            self.st &= !(1 << bit);
        }
    }

    fn clear_rqm(&mut self) {
        self.set_bit(st_bit::RQM, false);
    }

    // ---- Phase transitions ----

    fn go_to_command_phase(&mut self) {
        self.phase = Phase::Command;
        self.command.clear();
        self.command_len = 1;
        self.set_bit(st_bit::DIO, false);
        self.set_bit(st_bit::CB, false);
        self.set_bit(st_bit::EXM, false);
        self.set_bit(st_bit::RQM, true);
    }

    fn go_to_execution_phase(&mut self) {
        self.phase = Phase::Execution;
        self.set_bit(st_bit::DIO, self.current_cmd.to_cpu());
        self.set_bit(st_bit::EXM, true);
        self.set_bit(st_bit::RQM, true);
    }

    fn go_to_result_phase(&mut self) {
        self.phase = Phase::Result;
        self.result_pos = 0;
        self.set_bit(st_bit::EXM, false);
        self.set_bit(st_bit::CB, false);
        self.set_bit(st_bit::DIO, true);
        self.set_bit(st_bit::RQM, true);
    }

    // ---- Command phase ----

    fn write_command_byte(&mut self, val: u8) {
        if self.command.is_empty() {
            self.current_cmd = Cmd::from_byte(val);
            self.command_len = self.current_cmd.command_len();
            self.set_bit(st_bit::CB, true);
        }
        self.command.push(val);

        if self.command.len() == self.command_len {
            self.dispatch_command();
        }
    }

    fn dispatch_command(&mut self) {
        match self.current_cmd {
            Cmd::ReadData | Cmd::WriteData => self.start_read_write(),
            Cmd::ReadId => self.do_read_id(),
            Cmd::Recalibrate => self.do_seek_like(0, true),
            Cmd::Seek => {
                let target = self.command[CMD_TRACK];
                self.do_seek_like(target, false)
            },
            Cmd::SenseInterruptStatus => self.do_sense_interrupt_status(),
            Cmd::Specify => self.go_to_command_phase(),
            Cmd::SenseDriveStatus => self.do_sense_drive_status(),
            Cmd::Invalid => self.do_invalid()
        }
    }

    fn do_invalid(&mut self) {
        self.st0 = st0_ic::INVALID;
        self.result = vec![self.st0];
        self.go_to_result_phase();
    }

    fn unit_index(&self) -> usize {
        (self.command[CMD_UNIT] & 0x03) as usize
    }

    fn unit_head(&self) -> u8 {
        (self.command[CMD_UNIT] >> 2) & 1
    }

    // ---- Recalibrate / Seek ----

    /// Real hardware steps the head one track at a time and takes a
    /// variable number of milliseconds; we don't model that (nothing here
    /// depends on seek timing), just jump straight to the target and raise
    /// the "seek end" pending flag Sense Interrupt Status is expected to
    /// clear.
    fn do_seek_like(&mut self, target_track: u8, is_recalibrate: bool) {
        let unit = self.unit_index();
        self.active_drive = unit;

        let max_track = 41; // real drives max out at ~42 usable tracks
        let target_track = target_track.min(max_track);
        self.drives[unit].current_track = target_track;

        self.st0 = st0_ic::NORMAL | (1 << st0_bit::SE) | (unit as u8 & 0x07);
        if is_recalibrate && self.drives[unit].current_track != 0 {
            self.st0 |= 1 << st0_bit::EC;
        }

        // Mark this unit as having a pending "seek end" interrupt.
        self.st |= 1 << unit;

        self.go_to_command_phase();
    }

    fn do_sense_interrupt_status(&mut self) {
        let pending = self.st & 0x0f;
        if pending != 0 {
            let unit = pending.trailing_zeros() as usize;
            self.st &= !(1 << unit);
            self.result = vec![self.st0, self.drives[unit].current_track];
        }
        else {
            self.st0 = st0_ic::INVALID;
            self.result = vec![self.st0, 0xff];
        }
        self.go_to_result_phase();
    }

    fn do_sense_drive_status(&mut self) {
        let unit = self.unit_index();
        self.active_drive = unit;
        let head = self.unit_head();

        let mut st3 = self.command[CMD_UNIT] & 0x07; // HD, US1, US0 echoed back
        if let Some(disk) = &self.drives[unit].disk {
            st3 |= 1 << st3_bit::RY;
            if self.drives[unit].current_track == 0 {
                st3 |= 1 << st3_bit::T0;
            }
            if is_double_sided(disk) {
                st3 |= 1 << st3_bit::TS;
            }
        }
        let _ = head;

        self.result = vec![st3];
        self.go_to_result_phase();
    }

    // ---- Read ID ----

    fn do_read_id(&mut self) {
        let unit = self.unit_index();
        self.active_drive = unit;
        let head = self.unit_head();
        let track = self.drives[unit].current_track;

        self.st0 = st0_ic::NORMAL | (unit as u8 & 0x07);
        self.st1 = 0;
        self.st2 = 0;

        let mut chrn = (track, head, 1u8, 2u8);
        if !self.drive_ready(unit) {
            self.st0 = st0_ic::ABNORMAL | (unit as u8 & 0x07) | (1 << st0_bit::NR);
        }
        else if let Some(disk) = &self.drives[unit].disk {
            match first_sector_chrn(disk, head, track) {
                Some(found) => chrn = found,
                None => {
                    self.st0 = st0_ic::ABNORMAL | (unit as u8 & 0x07);
                    self.st1 |= 1 << st1_bit::ND;
                }
            }
        }
        else {
            self.st0 = st0_ic::ABNORMAL | (unit as u8 & 0x07);
            self.st1 |= 1 << st1_bit::ND;
        }

        self.result = vec![self.st0, self.st1, self.st2, chrn.0, chrn.1, chrn.2, chrn.3];
        self.go_to_result_phase();
    }

    // ---- Read Data / Write Data ----

    fn drive_ready(&self, unit: usize) -> bool {
        self.motor && self.drives[unit].disk.is_some()
    }

    fn start_read_write(&mut self) {
        let unit = self.unit_index();
        self.active_drive = unit;

        self.st0 = st0_ic::NORMAL | (unit as u8 & 0x07);
        self.st1 = 0;
        self.st2 = 0;

        if !self.drive_ready(unit) {
            self.st0 = st0_ic::ABNORMAL | (unit as u8 & 0x07) | (1 << st0_bit::NR);
            self.finish_read_write();
            return;
        }

        self.load_current_sector();
    }

    /// Fetch the sector named by the command's current C/H/R into
    /// `exec_data` and enter the execution phase, or fail straight to the
    /// result phase if it can't be found.
    fn load_current_sector(&mut self) {
        let unit = self.active_drive;
        let head: Head = self.command[CMD_HEAD].into();
        let track = self.command[CMD_TRACK];
        let sector_id = self.command[CMD_SECTOR];

        let bytes = self.drives[unit]
            .disk
            .as_ref()
            .and_then(|disk| disk.sector_read_bytes(head, track, sector_id));

        match bytes {
            Some(bytes) => {
                self.exec_data = if self.current_cmd == Cmd::WriteData {
                    vec![0; bytes.len()]
                }
                else {
                    bytes
                };
                self.exec_pos = 0;
                self.go_to_execution_phase();
            },
            None => {
                self.st0 |= st0_ic::ABNORMAL;
                self.st1 |= 1 << st1_bit::ND;
                self.finish_read_write();
            }
        }
    }

    fn advance_read_execution(&mut self) {
        self.maybe_advance_sector();
    }

    fn advance_write_execution(&mut self) {
        if self.exec_pos >= self.exec_data.len() {
            let unit = self.active_drive;
            let head: Head = self.command[CMD_HEAD].into();
            let track = self.command[CMD_TRACK];
            let sector_id = self.command[CMD_SECTOR];
            if let Some(disk) = self.drives[unit].disk.as_mut() {
                let _ = disk.sector_write_bytes(head, track, sector_id, &self.exec_data);
            }
        }
        self.maybe_advance_sector();
    }

    /// Once a whole sector has been transferred, either move on to the
    /// next sector (still below EOT) or end the command.
    fn maybe_advance_sector(&mut self) {
        if self.exec_pos < self.exec_data.len() {
            // Still transferring the current sector.
            self.set_bit(st_bit::RQM, true);
            return;
        }

        let eot = self.command[CMD_EOT];
        let has_error = self.st1 != 0 || self.st2 != 0;
        if !has_error && self.command[CMD_SECTOR] != eot {
            self.command[CMD_SECTOR] += 1;
            self.load_current_sector();
        }
        else {
            // Reaching exactly EOT with no error is a normal, successful
            // completion - not the "tried to read past the last sector of
            // the cylinder" condition ST1's EN bit is documented for, so
            // (unlike some reference emulators) this doesn't set EN.
            self.finish_read_write();
        }
    }

    fn finish_read_write(&mut self) {
        self.result = vec![
            self.st0,
            self.st1,
            self.st2,
            self.command.get(CMD_TRACK).copied().unwrap_or(0),
            self.command.get(CMD_HEAD).copied().unwrap_or(0),
            self.command.get(CMD_SECTOR).copied().unwrap_or(0),
            self.command.get(CMD_SIZE).copied().unwrap_or(0),
        ];
        self.go_to_result_phase();
    }
}

fn is_double_sided(disk: &AnyDisc) -> bool {
    match disk {
        AnyDisc::Edsk(dsk) => dsk.is_double_head(),
        #[cfg(feature = "hfe")]
        AnyDisc::Hfe(_) => false
    }
}

/// The (track, head, sector-id, size-code) of the first sector cpclib-disc
/// knows about on a track - a reasonable stand-in for "whichever sector
/// happens to be under the head" since we don't model disk rotation.
fn first_sector_chrn(disk: &AnyDisc, head: u8, track: u8) -> Option<(u8, u8, u8, u8)> {
    match disk {
        AnyDisc::Edsk(dsk) => {
            let info = dsk.get_track_information(Head::from(head), track)?;
            let sector_id = info.min_sector()?;
            let sector = info.sector(sector_id)?;
            Some((track, head, sector_id, size_code_from_len(sector.len())))
        },
        #[cfg(feature = "hfe")]
        AnyDisc::Hfe(_) => None
    }
}

/// Reverse of the NEC765 "N" sector-size code (`real size = 0x80 << N`).
fn size_code_from_len(len: u16) -> u8 {
    let mut n = 0u8;
    let mut size = 128u32;
    while size < len as u32 && n < 8 {
        size <<= 1;
        n += 1;
    }
    n
}
