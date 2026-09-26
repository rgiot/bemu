//! NEC uPD765A (aka "FDC") emulation - the floppy disk controller used by
//! the CPC's disk interface.
//!
//! This is *not* a pin-level `Chip` like the CRTC/PPI/GA: the real uPD765's
//! interesting behaviour (command/execution/result phases, status
//! registers) is a byte-level protocol over its two registers, and the CPC
//! never uses DMA nor the interrupt line - the Z80 always transfers data with
//! plain `IN`/`OUT`, one byte per port access, polling the main status
//! register. There's nothing to gain from ticking it every CPU cycle, so it
//! is driven directly by `Motherboard`'s I/O dispatch via
//! `read_status`/`read_data`/`write_data`/`set_motor`.
//!
//! Semantics follow the NEC uPD765A/B datasheet (`doc/fdc.upd765ab.pdf`):
//! all 15 commands, the full sector ID comparison (C/H/R/N, with the
//! wrong/bad cylinder flags), the ST0-ST3 bit meanings, MT/SK, the
//! ready-change interrupt, and the recorded error/deleted-mark flags of a
//! sector being honoured.
//!
//! Unlike a real drive, seeks and rotational latency are instantaneous:
//! drivers only depend on the eventual status/result bytes being right, and
//! modelling gap/index timing would only matter for copy-protection schemes
//! that measure it. What *is* modelled of the rotation is which sector is
//! under the head (a cursor that advances as sectors pass), so Read ID walks
//! the track like the real chip does.

use std::collections::HashMap;

use crate::disc_image::{flag, size_from_code, DiscImage, Sector, Track};
use cpclib_disc::AnyDisc;

/// Main status register bits (datasheet Table 2).
mod msr {
    pub const RQM: u8 = 1 << 7; // Request for Master: data register ready
    pub const DIO: u8 = 1 << 6; // Data I/O direction: 1 = FDC -> CPU
    pub const EXM: u8 = 1 << 5; // Execution mode (non-DMA)
    pub const CB: u8 = 1 << 4; // FDC busy
}

/// ST0 (datasheet Table 3).
mod st0 {
    pub const IC_ABNORMAL: u8 = 0x40;
    pub const IC_INVALID: u8 = 0x80;
    pub const IC_READY_CHANGED: u8 = 0xc0;
    pub const SE: u8 = 1 << 5; // Seek End
    pub const NR: u8 = 1 << 3; // Not Ready
    pub const HD: u8 = 1 << 2; // Head address
}

/// ST1 (datasheet Table 3).
mod st1 {
    pub const DE: u8 = 1 << 5; // Data Error (CRC)
    pub const ND: u8 = 1 << 2; // No Data (sector not found)
    pub const NW: u8 = 1 << 1; // Not Writable
    pub const MA: u8 = 1 << 0; // Missing Address mark
}

/// ST2 (datasheet Table 3).
mod st2 {
    pub const CM: u8 = 1 << 6; // Control Mark (deleted data mark)
    pub const DD: u8 = 1 << 5; // Data error in the Data field
    pub const WC: u8 = 1 << 4; // Wrong Cylinder
    pub const SH: u8 = 1 << 3; // Scan equal Hit
    pub const SN: u8 = 1 << 2; // Scan Not satisfied
    pub const BC: u8 = 1 << 1; // Bad Cylinder (C = &FF)
}

/// ST3 (datasheet Table 3).
mod st3 {
    pub const RY: u8 = 1 << 5; // Ready
    pub const T0: u8 = 1 << 4; // Track 0
    pub const TS: u8 = 1 << 3; // Two-sided
}

/// Number of unit-select values the chip can address (US1-US0).
const NB_UNITS: usize = 4;
/// A drive head cannot go past the last track its mechanism allows.
const MAX_TRACK: u8 = 76;

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

/// The commands of the uPD765A (datasheet Table 4). `Version` is a 765B
/// command: on the 765A it is just another invalid command (ST0 = &80).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmd {
    ReadData,
    ReadDeletedData,
    WriteData,
    WriteDeletedData,
    ReadTrack,
    ReadId,
    FormatTrack,
    ScanEqual,
    ScanLowOrEqual,
    ScanHighOrEqual,
    Recalibrate,
    SenseInterruptStatus,
    Specify,
    SenseDriveStatus,
    Seek,
    Invalid
}

impl Cmd {
    /// `byte` is the very first command-phase byte (the command code, still
    /// carrying its MT/MF/SK flag bits in D7-D5).
    fn from_byte(byte: u8) -> Self {
        match byte & 0x1f {
            0x02 => Cmd::ReadTrack,
            0x03 => Cmd::Specify,
            0x04 => Cmd::SenseDriveStatus,
            0x05 => Cmd::WriteData,
            0x06 => Cmd::ReadData,
            0x07 => Cmd::Recalibrate,
            0x08 => Cmd::SenseInterruptStatus,
            0x09 => Cmd::WriteDeletedData,
            0x0a => Cmd::ReadId,
            0x0c => Cmd::ReadDeletedData,
            0x0d => Cmd::FormatTrack,
            0x0f => Cmd::Seek,
            0x11 => Cmd::ScanEqual,
            0x19 => Cmd::ScanLowOrEqual,
            0x1d => Cmd::ScanHighOrEqual,
            _ => Cmd::Invalid
        }
    }

    /// Total command-phase length (command code byte included).
    fn command_len(self) -> usize {
        match self {
            Cmd::ReadData
            | Cmd::ReadDeletedData
            | Cmd::WriteData
            | Cmd::WriteDeletedData
            | Cmd::ReadTrack
            | Cmd::ScanEqual
            | Cmd::ScanLowOrEqual
            | Cmd::ScanHighOrEqual => 9,
            Cmd::FormatTrack => 6,
            Cmd::ReadId | Cmd::Recalibrate | Cmd::SenseDriveStatus => 2,
            Cmd::Specify | Cmd::Seek => 3,
            Cmd::SenseInterruptStatus | Cmd::Invalid => 1
        }
    }

    /// Direction of the execution-phase transfer: true = the CPU sends the
    /// bytes (Write, Scan and Format take data from the CPU).
    fn from_cpu(self) -> bool {
        matches!(
            self,
            Cmd::WriteData
                | Cmd::WriteDeletedData
                | Cmd::FormatTrack
                | Cmd::ScanEqual
                | Cmd::ScanLowOrEqual
                | Cmd::ScanHighOrEqual
        )
    }

    fn is_scan(self) -> bool {
        matches!(self, Cmd::ScanEqual | Cmd::ScanLowOrEqual | Cmd::ScanHighOrEqual)
    }

    fn is_write(self) -> bool {
        matches!(self, Cmd::WriteData | Cmd::WriteDeletedData)
    }

    /// Whether the data mark the command is looking for is the deleted one.
    fn wants_deleted_mark(self) -> bool {
        matches!(self, Cmd::ReadDeletedData | Cmd::WriteDeletedData)
    }
}

// Command-phase byte indices (datasheet Table 4).
const CMD_CODE: usize = 0;
const CMD_UNIT: usize = 1;
const CMD_C: usize = 2;
const CMD_H: usize = 3;
const CMD_R: usize = 4;
const CMD_N: usize = 5;
const CMD_EOT: usize = 6;
const CMD_DTL: usize = 8; // also STP for the scan commands
// Format Track (Write ID) command bytes
const FMT_N: usize = 2;
const FMT_SC: usize = 3;
const FMT_GPL: usize = 4;
const FMT_D: usize = 5;

/// One command as seen on the data register: the bytes the CPU wrote and
/// the result bytes the FDC answered (empty for commands with no result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRecord {
    pub command: Vec<u8>,
    pub result: Vec<u8>
}

#[derive(Default)]
struct Drive {
    disc: Option<DiscImage>,
    /// Cylinder the head is over (PCN).
    track: u8,
    /// Index, in the track's sector list, of the sector that will pass
    /// under the head next.
    cursor: usize,
    /// How often each sector was read, to alternate the copies of a weak
    /// sector: (head, track, index in the track).
    reads: HashMap<(u8, u8, usize), usize>
}

/// The data-transfer command in progress: where it is and what it searches.
#[derive(Default)]
struct Transfer {
    unit: usize,
    /// Physical side being accessed.
    head: u8,
    /// ID searched for; also what the result phase reports.
    c: u8,
    h: u8,
    r: u8,
    n: u8,
    eot: u8,
    /// DTL (data length, used when N = 0) or STP (scan step).
    dtl: u8,
    mt: bool,
    sk: bool,
    /// Index of the sector currently being transferred.
    sector: usize,
    /// The command ends after the sector being transferred (error, deleted
    /// mark met...).
    stop_after_sector: bool,
    /// Scan: whether a sector satisfied the condition.
    scan_satisfied: bool,
    /// Read Track: the sector indexes still to read (last one first).
    track_queue: Vec<usize>
}

pub struct Fdc {
    motor: bool,
    drives: [Drive; NB_UNITS],

    /// Main status register.
    st: u8,
    st0: u8,
    st1: u8,
    st2: u8,

    /// Pending interrupt (its ST0) per unit: a finished seek/recalibrate or
    /// a ready-line change. Sense Interrupt Status reports and clears one.
    /// (The CPC does not wire /INT, but the chip still keeps them.)
    pending_interrupt: [Option<u8>; NB_UNITS],

    phase: Phase,
    current_cmd: Cmd,
    command: Vec<u8>,
    command_len: usize,

    result: Vec<u8>,
    result_pos: usize,

    xfer: Transfer,
    /// Bytes of the sector currently being transferred in the execution
    /// phase (the *whole* sector is fetched up front; `read_data`/
    /// `write_data` just walk through it one byte at a time, matching what
    /// the real chip does one bit-cell at a time).
    exec_data: Vec<u8>,
    exec_pos: usize,

    /// Every command executed so far, when logging was asked for (see
    /// [`Fdc::start_command_log`]): a debugging aid for whoever needs to see
    /// what a driver such as AMSDOS really says to the chip.
    command_log: Option<Vec<CommandRecord>>
}

impl Default for Fdc {
    fn default() -> Self {
        let mut fdc = Self {
            motor: false,
            drives: Default::default(),
            st: 0,
            st0: 0,
            st1: 0,
            st2: 0,
            pending_interrupt: [None; NB_UNITS],
            phase: Phase::Command,
            current_cmd: Cmd::Invalid,
            command: Vec::new(),
            command_len: 1,
            result: Vec::new(),
            result_pos: 0,
            xfer: Transfer::default(),
            exec_data: Vec::new(),
            exec_pos: 0,
            command_log: None
        };
        fdc.reset();
        fdc
    }
}

impl Fdc {
    pub fn reset(&mut self) {
        self.motor = false;
        self.st0 = 0;
        self.st1 = 0;
        self.st2 = 0;
        self.pending_interrupt = [None; NB_UNITS];
        self.command.clear();
        self.command_len = 1;
        self.current_cmd = Cmd::Invalid;
        self.result.clear();
        self.result_pos = 0;
        self.exec_data.clear();
        self.exec_pos = 0;
        for drive in &mut self.drives {
            drive.track = 0;
            drive.cursor = 0;
        }
        self.go_to_command_phase();
    }

    /// Start recording every command (and its result) from now on.
    pub fn start_command_log(&mut self) {
        self.command_log = Some(Vec::new());
    }

    /// The commands recorded since [`Fdc::start_command_log`].
    pub fn command_log(&self) -> &[CommandRecord] {
        self.command_log.as_deref().unwrap_or_default()
    }

    pub fn set_motor(&mut self, on: bool) {
        self.motor = on;
    }

    pub fn motor(&self) -> bool {
        self.motor
    }

    /// Put a `cpclib-disc` disc in a drive (an empty drive - `None` -
    /// ejects it). Fails when the disc cannot be represented.
    pub fn insert_disk(&mut self, drive: FdcDrive, disk: Option<AnyDisc>) -> Result<(), String> {
        let image = disk.as_ref().map(DiscImage::try_from).transpose()?;
        self.insert_image(drive, image);
        Ok(())
    }

    /// Put a disc image in a drive (`None` ejects). Inserting or ejecting
    /// changes the drive's ready line, which the chip reports as an
    /// interrupt to the next Sense Interrupt Status.
    pub fn insert_image(&mut self, drive: FdcDrive, image: Option<DiscImage>) {
        let unit = drive.index();
        self.drives[unit].disc = image;
        self.drives[unit].track = 0;
        self.drives[unit].cursor = 0;
        self.drives[unit].reads.clear();
        self.pending_interrupt[unit] = Some(st0::IC_READY_CHANGED | unit as u8);
    }

    pub fn has_disk(&self, drive: FdcDrive) -> bool {
        self.drives[drive.index()].disc.is_some()
    }

    /// The disc in a drive, with whatever the FDC wrote to it.
    pub fn image(&self, drive: FdcDrive) -> Option<&DiscImage> {
        self.drives[drive.index()].disc.as_ref()
    }

    // ---- Register access (called directly from Motherboard's I/O dispatch) ----

    pub fn read_status(&self) -> u8 {
        self.st
    }

    pub fn write_data(&mut self, val: u8) {
        if self.st & msr::RQM == 0 || self.st & msr::DIO != 0 {
            // Not our turn to accept a byte from the CPU.
            return;
        }

        match self.phase {
            Phase::Command => self.write_command_byte(val),
            Phase::Execution => {
                if self.current_cmd.from_cpu() {
                    if let Some(slot) = self.exec_data.get_mut(self.exec_pos) {
                        *slot = val;
                    }
                    self.exec_pos += 1;
                    self.st &= !msr::RQM;
                    self.advance_execution();
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
                if self.st & msr::RQM != 0 && !self.current_cmd.from_cpu() {
                    let byte = self.exec_data.get(self.exec_pos).copied().unwrap_or(0xe5);
                    self.exec_pos += 1;
                    self.st &= !msr::RQM;
                    self.advance_execution();
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

    // ---- Phase transitions ----

    fn go_to_command_phase(&mut self) {
        self.phase = Phase::Command;
        self.command.clear();
        self.command_len = 1;
        self.st = msr::RQM;
    }

    fn go_to_execution_phase(&mut self) {
        self.phase = Phase::Execution;
        self.exec_pos = 0;
        self.st = msr::RQM | msr::EXM | msr::CB;
        if !self.current_cmd.from_cpu() {
            self.st |= msr::DIO;
        }
    }

    /// CB stays set for the whole result phase (MSR = &D0): software such as
    /// AMSDOS reads result bytes "while CB is set", so clearing it early made
    /// it stop after the first byte and desynchronise the FDC.
    /// `go_to_command_phase` clears it once the last byte has been read.
    fn go_to_result_phase(&mut self) {
        if let Some(record) = self.command_log.as_mut().and_then(|log| log.last_mut()) {
            record.result = self.result.clone();
        }
        self.phase = Phase::Result;
        self.result_pos = 0;
        self.st = msr::RQM | msr::DIO | msr::CB;
    }

    // ---- Command phase ----

    fn write_command_byte(&mut self, val: u8) {
        if self.command.is_empty() {
            self.current_cmd = Cmd::from_byte(val);
            self.command_len = self.current_cmd.command_len();
            self.st |= msr::CB;
        }
        self.command.push(val);

        if self.command.len() == self.command_len {
            self.dispatch_command();
        }
    }

    fn dispatch_command(&mut self) {
        if let Some(log) = &mut self.command_log {
            log.push(CommandRecord {
                command: self.command.clone(),
                result: Vec::new()
            });
        }

        match self.current_cmd {
            Cmd::ReadData
            | Cmd::ReadDeletedData
            | Cmd::WriteData
            | Cmd::WriteDeletedData
            | Cmd::ReadTrack
            | Cmd::ScanEqual
            | Cmd::ScanLowOrEqual
            | Cmd::ScanHighOrEqual => self.start_transfer(),
            Cmd::FormatTrack => self.start_format(),
            Cmd::ReadId => self.do_read_id(),
            Cmd::Recalibrate => self.do_seek_like(0),
            Cmd::Seek => self.do_seek_like(self.command[CMD_C]),
            Cmd::SenseInterruptStatus => self.do_sense_interrupt_status(),
            Cmd::Specify => self.go_to_command_phase(),
            Cmd::SenseDriveStatus => self.do_sense_drive_status(),
            Cmd::Invalid => self.do_invalid()
        }
    }

    fn do_invalid(&mut self) {
        // Also what a 765A answers to the (765B) Version command.
        self.result = vec![st0::IC_INVALID];
        self.go_to_result_phase();
    }

    // ---- Decoding the command's HD/US byte and flags ----

    fn unit(&self) -> usize {
        usize::from(self.command[CMD_UNIT] & 0x03)
    }

    fn head(&self) -> u8 {
        (self.command[CMD_UNIT] >> 2) & 1
    }

    fn mt(&self) -> bool {
        self.command[CMD_CODE] & 0x80 != 0
    }

    fn mfm(&self) -> bool {
        self.command[CMD_CODE] & 0x40 != 0
    }

    fn sk(&self) -> bool {
        self.command[CMD_CODE] & 0x20 != 0
    }

    /// The unit and head bits every ST0 carries.
    fn st0_unit_head(&self) -> u8 {
        self.unit() as u8 | if self.head() != 0 { st0::HD } else { 0 }
    }

    /// The drive is ready when it spins with a disc in it.
    fn ready(&self, unit: usize) -> bool {
        self.motor && self.drives[unit].disc.is_some()
    }

    // ---- Recalibrate / Seek ----

    /// Real hardware steps the head one track at a time and takes a
    /// variable number of milliseconds; we jump straight to the target and
    /// raise the "seek end" interrupt Sense Interrupt Status reports.
    fn do_seek_like(&mut self, target_track: u8) {
        let unit = self.unit();
        let drive = &mut self.drives[unit];
        drive.track = target_track.min(MAX_TRACK);
        drive.cursor = 0;

        // A seek on a drive with no disc cannot complete normally.
        let st0 = if drive.disc.is_some() {
            st0::SE | unit as u8
        }
        else {
            st0::IC_ABNORMAL | st0::NR | unit as u8
        };
        self.pending_interrupt[unit] = Some(st0);
        self.go_to_command_phase();
    }

    fn do_sense_interrupt_status(&mut self) {
        match self.pending_interrupt.iter().position(Option::is_some) {
            Some(unit) => {
                let st0 = self.pending_interrupt[unit].take().unwrap_or(0);
                self.result = vec![st0, self.drives[unit].track];
            },
            // No interrupt pending: the chip treats this as an invalid
            // command and answers with the single byte ST0 = &80.
            None => self.result = vec![st0::IC_INVALID]
        }
        self.go_to_result_phase();
    }

    fn do_sense_drive_status(&mut self) {
        let unit = self.unit();
        let drive = &self.drives[unit];

        // HD, US1, US0 are echoed back from the command's HD/US byte
        let mut st3 = self.command[CMD_UNIT] & 0x07;
        if drive.track == 0 {
            st3 |= st3::T0;
        }
        if drive.disc.as_ref().is_some_and(DiscImage::is_double_sided) {
            st3 |= st3::TS;
        }
        if self.ready(unit) {
            st3 |= st3::RY;
        }

        self.result = vec![st3];
        self.go_to_result_phase();
    }

    // ---- Locating sectors ----

    /// The sectors of the track under `unit`'s head.
    fn sectors(&self, unit: usize, head: u8) -> &[Sector] {
        let drive = &self.drives[unit];
        drive
            .disc
            .as_ref()
            .and_then(|disc| disc.track(head, drive.track))
            .map_or(&[], |track| track.sectors.as_slice())
    }

    /// Search the track for the sector whose ID is exactly C/H/R/N, starting
    /// with the one now under the head. Fills ST1/ST2 the way a failed
    /// search does (No Data, Wrong/Bad Cylinder, Missing Address Mark).
    fn find_sector(&mut self) -> Option<usize> {
        let (unit, head) = (self.xfer.unit, self.xfer.head);
        let (c, h, r, n) = (self.xfer.c, self.xfer.h, self.xfer.r, self.xfer.n);
        let cursor = self.drives[unit].cursor;
        let sectors = self.sectors(unit, head);

        if sectors.is_empty() {
            // no ID address mark to be found on this track
            self.st1 |= st1::MA | st1::ND;
            return None;
        }

        let mut wrong_cylinder = false;
        let mut bad_cylinder = false;
        let mut found = None;
        for offset in 0..sectors.len() {
            let index = (cursor + offset) % sectors.len();
            let sector = &sectors[index];
            if sector.r == r && sector.c == c && sector.h == h && sector.n == n {
                found = Some(index);
                break;
            }
            if sector.r == r && sector.c != c {
                if sector.c == 0xff {
                    bad_cylinder = true;
                }
                else {
                    wrong_cylinder = true;
                }
            }
        }

        match found {
            Some(index) => {
                let len = sectors.len();
                self.drives[unit].cursor = (index + 1) % len;
                Some(index)
            },
            None => {
                self.st1 |= st1::ND;
                if wrong_cylinder {
                    self.st2 |= st2::WC;
                }
                if bad_cylinder {
                    self.st2 |= st2::BC;
                }
                None
            }
        }
    }

    // ---- Read ID ----

    fn do_read_id(&mut self) {
        let unit = self.unit();
        let head = self.head();
        self.st0 = self.st0_unit_head();
        self.st1 = 0;
        self.st2 = 0;

        let mut id = (self.drives[unit].track, head, 0u8, 0u8);
        if !self.ready(unit) {
            self.st0 |= st0::IC_ABNORMAL | st0::NR;
        }
        else if !self.mfm() {
            // The discs held as DSK images are MFM: no FM address mark
            self.st0 |= st0::IC_ABNORMAL;
            self.st1 |= st1::MA | st1::ND;
        }
        else {
            let cursor = self.drives[unit].cursor;
            let sectors = self.sectors(unit, head);
            if sectors.is_empty() {
                self.st0 |= st0::IC_ABNORMAL;
                self.st1 |= st1::MA | st1::ND;
            }
            else {
                let index = cursor % sectors.len();
                let sector = &sectors[index];
                id = (sector.c, sector.h, sector.r, sector.n);
                let len = sectors.len();
                self.drives[unit].cursor = (index + 1) % len;
            }
        }

        self.result = vec![self.st0, self.st1, self.st2, id.0, id.1, id.2, id.3];
        self.go_to_result_phase();
    }

    // ---- Data transfer commands (Read/Write/Scan, deleted or not, Read Track) ----

    fn start_transfer(&mut self) {
        let unit = self.unit();
        self.st0 = self.st0_unit_head();
        self.st1 = 0;
        self.st2 = 0;

        self.xfer = Transfer {
            unit,
            head: self.head(),
            c: self.command[CMD_C],
            h: self.command[CMD_H],
            r: self.command[CMD_R],
            n: self.command[CMD_N],
            eot: self.command[CMD_EOT],
            dtl: self.command[CMD_DTL],
            mt: self.mt(),
            sk: self.sk(),
            ..Transfer::default()
        };

        if !self.ready(unit) {
            self.st0 |= st0::IC_ABNORMAL | st0::NR;
            self.finish_transfer();
            return;
        }
        if !self.mfm() {
            // The discs held as DSK images are MFM: no FM address mark
            self.st0 |= st0::IC_ABNORMAL;
            self.st1 |= st1::MA | st1::ND;
            self.finish_transfer();
            return;
        }

        if self.current_cmd == Cmd::ReadTrack {
            // Every data field from the index hole on, EOT sectors in all
            let count = usize::from(self.xfer.eot).min(self.sectors(unit, self.xfer.head).len());
            self.xfer.track_queue = (0..count).rev().collect();
            self.drives[unit].cursor = 0;
            if !self.next_read_track_sector() {
                self.st0 |= st0::IC_ABNORMAL;
                self.st1 |= st1::MA | st1::ND;
                self.finish_transfer();
            }
            return;
        }

        self.search_and_load();
    }

    /// Find the sector the command is about and start (or skip) it; ends
    /// the command when it cannot be found.
    fn search_and_load(&mut self) {
        loop {
            let Some(index) = self.find_sector()
            else {
                self.st0 |= st0::IC_ABNORMAL;
                self.finish_transfer();
                return;
            };

            let (unit, head) = (self.xfer.unit, self.xfer.head);
            let deleted = self.sectors(unit, head)[index].is_deleted();

            // Data mark not the wanted kind (Read Data met a deleted
            // sector, Read Deleted Data a normal one)?
            let mark_mismatch = !self.current_cmd.is_write()
                && deleted != self.current_cmd.wants_deleted_mark();
            if mark_mismatch && self.xfer.sk {
                // SK: skip that sector without transferring it
                if !self.advance_to_next_sector() {
                    self.finish_transfer();
                    return;
                }
                continue;
            }

            self.load_sector(index, mark_mismatch);
            return;
        }
    }

    /// Fetch sector `index` and enter the execution phase.
    fn load_sector(&mut self, index: usize, mark_mismatch: bool) {
        let (unit, head) = (self.xfer.unit, self.xfer.head);
        let track = self.drives[unit].track;
        let key = (head, track, index);
        let copy = self.drives[unit].reads.get(&key).copied().unwrap_or(0);
        let filler = self.track_filler(unit, head);
        let (recorded_st1, recorded_st2, data, id) = {
            let sector = &self.sectors(unit, head)[index];
            (
                sector.st1,
                sector.st2,
                sector.read_copy(copy, filler),
                (sector.c, sector.h, sector.r, sector.n)
            )
        };

        self.xfer.sector = index;
        (self.xfer.c, self.xfer.h, self.xfer.r, self.xfer.n) = id;

        // What the medium recorded about this sector shows up in the flags
        self.st1 |= recorded_st1;
        self.st2 |= recorded_st2 & !st2::CM;
        if recorded_st1 & flag::ST1_DATA_ERROR != 0
            || recorded_st2 & flag::ST2_DATA_ERROR_IN_DATA != 0
        {
            self.st1 |= st1::DE;
            self.st2 |= st2::DD;
            self.xfer.stop_after_sector = true;
        }
        if mark_mismatch {
            // read anyway (SK=0), flag the other kind of mark, and stop
            self.st2 |= st2::CM;
            self.xfer.stop_after_sector = true;
        }

        if !self.current_cmd.is_write() {
            *self.drives[unit].reads.entry(key).or_insert(0) += 1;
        }

        // N = 0: only DTL bytes of the (128 byte) sector are transferred
        let mut len = data.len();
        if id.3 == 0 {
            len = len.min(usize::from(self.xfer.dtl));
        }
        self.exec_data = if self.current_cmd.is_write() || self.current_cmd.is_scan() {
            vec![0; len]
        }
        else {
            let mut data = data;
            data.truncate(len);
            data
        };
        self.go_to_execution_phase();
    }

    fn track_filler(&self, unit: usize, head: u8) -> u8 {
        let drive = &self.drives[unit];
        drive
            .disc
            .as_ref()
            .and_then(|disc| disc.track(head, drive.track))
            .map_or(0xe5, |track| track.filler)
    }

    /// Read Track: start the next queued sector. False when none is left.
    fn next_read_track_sector(&mut self) -> bool {
        let Some(index) = self.xfer.track_queue.pop()
        else {
            return false;
        };
        self.load_sector(index, false);
        true
    }

    /// After a sector: move on to the next one the command covers.
    /// False when the command is done.
    fn advance_to_next_sector(&mut self) -> bool {
        if self.xfer.r >= self.xfer.eot {
            // last sector of the cylinder: a multi-track command carries on
            // with the other side
            let two_sided = self.drives[self.xfer.unit]
                .disc
                .as_ref()
                .is_some_and(DiscImage::is_double_sided);
            if self.xfer.mt && self.xfer.head == 0 && two_sided {
                self.xfer.head = 1;
                self.xfer.h = self.xfer.h.wrapping_add(1);
                self.xfer.r = 1;
                self.drives[self.xfer.unit].cursor = 0;
                return true;
            }
            return false;
        }
        let step = if self.current_cmd.is_scan() {
            self.xfer.dtl.max(1)
        }
        else {
            1
        };
        self.xfer.r = self.xfer.r.wrapping_add(step);
        true
    }

    /// The execution phase moved one byte: when the sector is complete, go
    /// on to whatever comes next.
    fn advance_execution(&mut self) {
        if self.exec_pos < self.exec_data.len() {
            // still transferring the current sector
            self.st |= msr::RQM;
            return;
        }

        match self.current_cmd {
            Cmd::FormatTrack => self.finish_format(),
            _ => self.finish_sector()
        }
    }

    /// One sector's data has been fully transferred.
    fn finish_sector(&mut self) {
        let (unit, head, index) = (self.xfer.unit, self.xfer.head, self.xfer.sector);

        if self.current_cmd.is_write() {
            self.store_written_sector(unit, head, index);
        }
        else if self.current_cmd.is_scan() {
            self.compare_scanned_sector(unit, head, index);
        }

        if self.xfer.stop_after_sector {
            self.finish_transfer();
            return;
        }
        if self.current_cmd == Cmd::ReadTrack {
            if !self.next_read_track_sector() {
                self.finish_transfer();
            }
            return;
        }
        if self.current_cmd.is_scan() && self.xfer.scan_satisfied {
            self.finish_transfer();
            return;
        }
        if self.advance_to_next_sector() {
            self.search_and_load();
        }
        else {
            if self.current_cmd.is_scan() && !self.xfer.scan_satisfied {
                self.st2 |= st2::SN;
            }
            self.finish_transfer();
        }
    }

    /// Write Data / Write Deleted Data: put the received bytes in the
    /// sector (and its data mark).
    fn store_written_sector(&mut self, unit: usize, head: u8, index: usize) {
        let deleted = self.current_cmd.wants_deleted_mark();
        let drive = &mut self.drives[unit];
        let track = drive.track;
        let Some(sector) = drive
            .disc
            .as_mut()
            .and_then(|disc| disc.track_mut(head, track))
            .and_then(|track| track.sectors.get_mut(index))
        else {
            return;
        };

        let len = self.exec_data.len();
        if sector.data.len() < len {
            sector.data.resize(len, 0);
        }
        sector.data[..len].copy_from_slice(&self.exec_data);
        // rewriting a sector repairs it and (re)sets its data mark
        sector.st1 &= !flag::ST1_DATA_ERROR;
        sector.st2 &= !(flag::ST2_DATA_ERROR_IN_DATA | flag::ST2_CONTROL_MARK);
        if deleted {
            sector.st2 |= flag::ST2_CONTROL_MARK;
        }
    }

    /// Scan commands: compare the bytes the CPU sent with the sector. A byte
    /// of &FF from the CPU matches anything (datasheet).
    fn compare_scanned_sector(&mut self, unit: usize, head: u8, index: usize) {
        let drive = &self.drives[unit];
        let Some(sector) = drive
            .disc
            .as_ref()
            .and_then(|disc| disc.track(head, drive.track))
            .and_then(|track| track.sectors.get(index))
        else {
            return;
        };

        let stored = sector.read_copy(0, 0xe5);
        let mut equal = true;
        let mut lower_or_equal = true;
        let mut higher_or_equal = true;
        for (disc, cpu) in stored.iter().zip(&self.exec_data) {
            if *cpu == 0xff {
                continue;
            }
            equal &= disc == cpu;
            lower_or_equal &= disc <= cpu;
            higher_or_equal &= disc >= cpu;
        }

        let satisfied = match self.current_cmd {
            Cmd::ScanEqual => equal,
            Cmd::ScanLowOrEqual => lower_or_equal,
            _ => higher_or_equal
        };
        if satisfied {
            self.xfer.scan_satisfied = true;
            if equal {
                self.st2 |= st2::SH;
            }
        }
    }

    /// End of a data-transfer command: build the result phase. The IDs
    /// reported are those of the last sector handled.
    fn finish_transfer(&mut self) {
        // Anything recorded as wrong in ST1/ST2 makes the termination
        // abnormal (a bare control mark or scan hit does not).
        if self.st1 != 0 || self.st2 & !(st2::CM | st2::SH) != 0 {
            self.st0 |= st0::IC_ABNORMAL;
        }
        self.result = vec![
            self.st0,
            self.st1,
            self.st2,
            self.xfer.c,
            self.xfer.h,
            self.xfer.r,
            self.xfer.n,
        ];
        self.go_to_result_phase();
    }

    // ---- Format Track (Write ID) ----

    fn start_format(&mut self) {
        let unit = self.unit();
        self.st0 = self.st0_unit_head();
        self.st1 = 0;
        self.st2 = 0;
        self.xfer = Transfer {
            unit,
            head: self.head(),
            ..Transfer::default()
        };

        if !self.ready(unit) {
            self.st0 |= st0::IC_ABNORMAL | st0::NR;
            self.finish_transfer();
            return;
        }
        if !self.mfm() {
            self.st0 |= st0::IC_ABNORMAL;
            self.st1 |= st1::MA | st1::ND;
            self.finish_transfer();
            return;
        }

        // the CPU now sends one C/H/R/N per sector to create
        self.exec_data = vec![0; usize::from(self.command[FMT_SC]) * 4];
        if self.exec_data.is_empty() {
            self.finish_format();
        }
        else {
            self.go_to_execution_phase();
        }
    }

    fn finish_format(&mut self) {
        let (unit, head) = (self.xfer.unit, self.xfer.head);
        let n = self.command[FMT_N];
        let filler = self.command[FMT_D];
        let sectors: Vec<Sector> = self
            .exec_data
            .chunks_exact(4)
            .map(|id| {
                let mut sector = Sector::new(id[0], id[1], id[2], id[3], filler);
                // the data area follows the N of the command, not each ID's
                sector.data = vec![filler; size_from_code(n)];
                sector
            })
            .collect();

        let gap3 = self.command[FMT_GPL];
        let drive = &mut self.drives[unit];
        let track_number = drive.track;
        let formatted = drive.disc.as_mut().is_some_and(|disc| {
            disc.set_track(head, track_number, Track {
                track: track_number,
                head,
                gap3,
                filler,
                sectors
            })
            .is_ok()
        });
        drive.cursor = 0;

        if formatted {
            self.xfer.c = track_number;
            self.xfer.h = head;
            self.xfer.n = n;
        }
        else {
            // off the end of the disc
            self.st0 |= st0::IC_ABNORMAL;
            self.st1 |= st1::NW;
        }
        self.finish_transfer();
    }
}
