//! The Albireo: a CH376 USB host controller on the expansion port (&FE80 data, &FE81 command
//! written, status read: bit 7 low while an interrupt is pending), its USB key a FAT image file.
//!
//! The CH376 does the file system itself (its "file mode": open, enumerate, read and write
//! bytes, create, erase, directory entries) or gives the raw sectors (DISK_READ/DISK_WRITE, 64
//! bytes at a time), as its datasheet describes. Here the file mode goes through the `fatfs`
//! crate, on the image's FAT volume (the first partition when the image has a partition table),
//! the raw sectors straight to the image: both are coherent, the file system being mounted
//! afresh for each operation. A host folder can stand for the key too (its files under 8.3
//! names, no raw sectors).
//!
//! `BEMU_CH376_TRACE=1` prints the commands and their parameters as they come.

use std::any::Any;
use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::expansion_port::{ExpansionDevice, ExpansionPort};

/// What a USB key is: the image, read and written.
pub trait Storage: Read + Write + Seek + Send {}
impl<T: Read + Write + Seek + Send> Storage for T {}

const CMD_GET_IC_VER: u8 = 0x01;
const CMD_RESET_ALL: u8 = 0x05;
const CMD_CHECK_EXIST: u8 = 0x06;
const CMD_READ_VAR8: u8 = 0x0a;
const CMD_SET_RETRY: u8 = 0x0b;
const CMD_GET_FILE_SIZE: u8 = 0x0c;
const CMD_SET_USB_MODE: u8 = 0x15;
const CMD_GET_STATUS: u8 = 0x22;
const CMD_RD_USB_DATA0: u8 = 0x27;
const CMD_RD_USB_DATA: u8 = 0x28;
const CMD_WR_HOST_DATA: u8 = 0x2c;
const CMD_WR_REQ_DATA: u8 = 0x2d;
const CMD_WR_OFS_DATA: u8 = 0x2e;
const CMD_SET_FILE_NAME: u8 = 0x2f;
const CMD_DISK_CONNECT: u8 = 0x30;
const CMD_DISK_MOUNT: u8 = 0x31;
const CMD_FILE_OPEN: u8 = 0x32;
const CMD_FILE_ENUM_GO: u8 = 0x33;
const CMD_FILE_CREATE: u8 = 0x34;
const CMD_FILE_ERASE: u8 = 0x35;
const CMD_FILE_CLOSE: u8 = 0x36;
const CMD_DIR_INFO_READ: u8 = 0x37;
const CMD_DIR_INFO_SAVE: u8 = 0x38;
const CMD_BYTE_LOCATE: u8 = 0x39;
const CMD_BYTE_READ: u8 = 0x3a;
const CMD_BYTE_RD_GO: u8 = 0x3b;
const CMD_BYTE_WRITE: u8 = 0x3c;
const CMD_BYTE_WR_GO: u8 = 0x3d;
const CMD_DISK_CAPACITY: u8 = 0x3e;
const CMD_DISK_QUERY: u8 = 0x3f;
const CMD_DIR_CREATE: u8 = 0x40;
const CMD_DISK_READ: u8 = 0x54;
const CMD_DISK_RD_GO: u8 = 0x55;
const CMD_DISK_WRITE: u8 = 0x56;
const CMD_DISK_WR_GO: u8 = 0x57;

pub const USB_INT_SUCCESS: u8 = 0x14;
pub const USB_INT_CONNECT: u8 = 0x15;
pub const USB_INT_DISK_READ: u8 = 0x1d;
pub const USB_INT_DISK_WRITE: u8 = 0x1e;
pub const USB_INT_DISK_ERR: u8 = 0x1f;
pub const ERR_OPEN_DIR: u8 = 0x41;
pub const ERR_MISS_FILE: u8 = 0x42;
pub const ERR_FOUND_NAME: u8 = 0x43;
pub const ERR_DISK_FULL: u8 = 0xb1;
const CMD_RET_SUCCESS: u8 = 0x51;

/// The chip's own version (GET_IC_VER).
const IC_VERSION: u8 = 0x43;
/// What a file transfer gives at a time, and a sector transfer.
const CHUNK: usize = 64;
const SECTOR: u64 = 512;

/// A transfer under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transfer {
    None,
    /// BYTE_READ: bytes left, the size of the chunk given.
    ByteRead {
        left: u32,
        chunk: u32,
    },
    /// BYTE_WRITE: bytes left.
    ByteWrite {
        left: u32,
    },
    /// DISK_READ/DISK_WRITE: the sector, sectors left, the offset in the sector.
    Disk {
        lba: u64,
        left: u32,
        offset: u64,
        write: bool,
    },
}

/// The file or directory open.
#[derive(Debug, Clone, Default)]
struct OpenEntry {
    /// From the root, the names as on the volume.
    path: Vec<String>,
    directory: bool,
    position: u64,
}

/// The CH376.
pub struct Ch376 {
    /// The USB key plugged.
    volume: Option<Box<dyn Volume>>,
    /// SET_USB_MODE's: the key answers in USB host mode (5-7), not as an SD card (3).
    mode: u8,
    command: u8,
    /// The parameters written since the command.
    input: Vec<u8>,
    output: VecDeque<u8>,
    /// The chip's data buffer (RD_USB_DATA0, WR_OFS_DATA, DIR_INFO_SAVE).
    buffer: Vec<u8>,
    /// What WR_REQ_DATA asked for and is being written.
    requested: usize,
    interrupt: Option<u8>,
    /// The last interrupt status (GET_STATUS).
    status: u8,
    file_name: String,
    /// The directory names are relative to.
    directory: Vec<String>,
    open: Option<OpenEntry>,
    /// FILE_OPEN with a wildcard: the entries matching, the next one.
    enumeration: Option<(Vec<[u8; 32]>, usize)>,
    transfer: Transfer,
}

impl Ch376 {
    /// The chip with `volume` as its USB key (none: no key plugged).
    pub fn new(volume: Option<Box<dyn Volume>>) -> Self {
        let mut chip = Self {
            volume: None,
            mode: 0,
            command: 0,
            input: Vec::new(),
            output: VecDeque::new(),
            buffer: Vec::new(),
            requested: 0,
            interrupt: None,
            status: 0,
            file_name: String::new(),
            directory: Vec::new(),
            open: None,
            enumeration: None,
            transfer: Transfer::None,
        };
        chip.plug(volume);
        chip
    }

    /// Plug a USB key (or unplug it).
    pub fn plug(&mut self, volume: Option<Box<dyn Volume>>) {
        self.volume = volume;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.mode = 0;
        self.command = 0;
        self.input.clear();
        self.output.clear();
        self.buffer.clear();
        self.requested = 0;
        self.interrupt = None;
        self.status = 0;
        self.file_name.clear();
        self.directory.clear();
        self.open = None;
        self.enumeration = None;
        self.transfer = Transfer::None;
    }

    /// The status port: bit 7 low while an interrupt is pending (INT#).
    pub fn read_status(&self) -> u8 {
        if self.interrupt.is_some() {
            0x00
        } else {
            0x80
        }
    }

    /// The data port.
    pub fn read_data(&mut self) -> u8 {
        let value = self.output.pop_front().unwrap_or(0);
        if self.command == CMD_GET_STATUS {
            self.interrupt = None;
        }
        value
    }

    /// A command written.
    pub fn write_command(&mut self, command: u8) {
        self.command = command;
        self.input.clear();
        self.output.clear();
        self.parameters_complete();
    }

    /// A data byte written.
    pub fn write_data(&mut self, value: u8) {
        self.input.push(value);
        self.parameters_complete();
    }

    fn raise(&mut self, status: u8) {
        self.status = status;
        self.interrupt = Some(status);
    }

    fn give(&mut self, bytes: &[u8]) {
        self.output.extend(bytes.iter().copied());
    }

    /// Run the command once its parameters are in.
    fn parameters_complete(&mut self) {
        let input = self.input.clone();
        if std::env::var_os("BEMU_CH376_TRACE").is_some() {
            eprintln!(
                "CH376 {:02X} {:02X?} name={:?} int={:?}",
                self.command, input, self.file_name, self.interrupt
            );
        }
        let needs = |n: usize| input.len() == n;
        match self.command {
            CMD_GET_IC_VER if needs(0) => self.give(&[IC_VERSION]),
            CMD_RESET_ALL if needs(0) => self.reset(),
            CMD_READ_VAR8 if needs(1) => self.give(&[0]),
            CMD_CHECK_EXIST if needs(1) => self.give(&[!input[0]]),
            CMD_SET_RETRY if needs(2) => {}
            CMD_GET_FILE_SIZE if needs(1) => {
                let size = self.open_size().unwrap_or(0) as u32;
                self.give(&size.to_le_bytes());
            }
            CMD_SET_USB_MODE if needs(1) => {
                self.mode = input[0];
                self.give(&[CMD_RET_SUCCESS]);
                if matches!(input[0], 6 | 7) && self.volume.is_some() {
                    self.raise(USB_INT_CONNECT);
                }
            }
            CMD_GET_STATUS if needs(0) => self.give(&[self.status]),
            CMD_RD_USB_DATA0 | CMD_RD_USB_DATA if needs(0) => {
                let buffer = std::mem::take(&mut self.buffer);
                self.give(&[buffer.len() as u8]);
                self.give(&buffer);
            }
            CMD_WR_REQ_DATA if needs(0) => {
                self.requested = self.request_size();
                self.give(&[self.requested as u8]);
                self.buffer.clear();
            }
            CMD_WR_REQ_DATA => {
                // the bytes asked for
                self.buffer.push(*input.last().unwrap());
                if self.buffer.len() == self.requested {
                    self.store_written();
                }
            }
            CMD_WR_HOST_DATA if !input.is_empty() && input.len() == 1 + usize::from(input[0]) => {
                self.buffer = input[1..].to_vec();
                self.store_written();
            }
            CMD_WR_OFS_DATA if input.len() >= 2 && input.len() == 2 + usize::from(input[1]) => {
                let offset = usize::from(input[0]);
                if self.buffer.len() < offset + input.len() - 2 {
                    self.buffer.resize(offset + input.len() - 2, 0);
                }
                self.buffer[offset..offset + input.len() - 2].copy_from_slice(&input[2..]);
            }
            CMD_SET_FILE_NAME if input.last() == Some(&0) || input.len() > 14 => {
                let end = input.iter().position(|&b| b == 0).unwrap_or(input.len());
                // 8.3 names may come padded, as a directory entry holds them ("NAME    .EXT")
                self.file_name = String::from_utf8_lossy(&input[..end]).replace(' ', "");
            }
            CMD_DISK_CONNECT | CMD_DISK_MOUNT if needs(0) => {
                let usb = matches!(self.mode, 3 | 5..=7); // 3: the SD card slot of the board (UniDOS's "SD:" device)
                let command = self.command;
                let mounted = usb
                    && self.volume.as_mut().is_some_and(|volume| {
                        command == CMD_DISK_CONNECT || volume.query().is_ok()
                    });
                if mounted {
                    if self.command == CMD_DISK_MOUNT {
                        // the key's INQUIRY data
                        let mut inquiry = vec![0, 0x80, 2, 2, 31, 0, 0, 0];
                        inquiry.extend_from_slice(b"bemu    USB key         1.00");
                        self.buffer = inquiry;
                    }
                    self.raise(USB_INT_SUCCESS);
                } else {
                    self.raise(USB_INT_DISK_ERR);
                }
            }
            CMD_FILE_OPEN if needs(0) => self.file_open(),
            CMD_FILE_ENUM_GO if needs(0) => self.next_entry(),
            CMD_FILE_CREATE if needs(0) => self.file_create(),
            CMD_FILE_ERASE if needs(0) => self.file_erase(),
            CMD_FILE_CLOSE if needs(1) => {
                self.open = None;
                self.transfer = Transfer::None;
                self.raise(USB_INT_SUCCESS);
            }
            CMD_DIR_INFO_READ if needs(1) => match self.open_entry_info() {
                Some(entry) => {
                    self.buffer = entry.to_vec();
                    self.raise(USB_INT_SUCCESS);
                }
                None => self.raise(ERR_MISS_FILE),
            },
            CMD_DIR_INFO_SAVE if needs(0) => self.dir_info_save(),
            CMD_BYTE_LOCATE if needs(4) => {
                let offset = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
                let size = self.open_size();
                match (&mut self.open, size) {
                    (Some(open), Some(size)) if !open.directory => {
                        open.position = if offset == u32::MAX {
                            size
                        } else {
                            u64::from(offset).min(size)
                        };
                        self.raise(USB_INT_SUCCESS);
                    }
                    _ => self.raise(ERR_MISS_FILE),
                }
            }
            CMD_BYTE_READ if needs(2) => {
                self.transfer = Transfer::ByteRead {
                    left: u16::from_le_bytes([input[0], input[1]]).into(),
                    chunk: 0,
                };
                self.byte_read_chunk();
            }
            CMD_BYTE_RD_GO if needs(0) => self.byte_read_chunk(),
            CMD_BYTE_WRITE if needs(2) => {
                let left = u16::from_le_bytes([input[0], input[1]]).into();
                self.transfer = Transfer::ByteWrite { left };
                self.raise(if left == 0 {
                    USB_INT_SUCCESS
                } else {
                    USB_INT_DISK_WRITE
                });
            }
            CMD_BYTE_WR_GO if needs(0) => match self.transfer {
                Transfer::ByteWrite { left } if left > 0 => self.raise(USB_INT_DISK_WRITE),
                _ => {
                    self.transfer = Transfer::None;
                    self.raise(USB_INT_SUCCESS);
                }
            },
            CMD_DISK_CAPACITY if needs(0) => {
                match self.volume.as_mut().map(|volume| volume.capacity()) {
                    Some(sectors) => {
                        self.buffer = (sectors as u32).to_le_bytes().to_vec();
                        self.raise(USB_INT_SUCCESS);
                    }
                    None => self.raise(USB_INT_DISK_ERR),
                }
            }
            CMD_DISK_QUERY if needs(0) => match self.volume.as_mut().map(|volume| volume.query()) {
                Some(Ok((total, free, kind))) => {
                    let mut query = total.to_le_bytes().to_vec();
                    query.extend_from_slice(&free.to_le_bytes());
                    query.push(kind);
                    self.buffer = query;
                    self.raise(USB_INT_SUCCESS);
                }
                _ => self.raise(USB_INT_DISK_ERR),
            },
            CMD_DIR_CREATE if needs(0) => self.dir_create(),
            CMD_DISK_READ | CMD_DISK_WRITE if needs(5) => {
                let lba = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
                let write = self.command == CMD_DISK_WRITE;
                self.transfer = Transfer::Disk {
                    lba: lba.into(),
                    left: input[4].into(),
                    offset: 0,
                    write,
                };
                self.disk_step(false);
            }
            CMD_DISK_RD_GO | CMD_DISK_WR_GO if needs(0) => self.disk_step(true),
            _ => {}
        }
    }

    fn volume(&mut self) -> io::Result<&mut Box<dyn Volume>> {
        self.volume
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no USB key"))
    }

    /// `name` (CH376: `/` from the root, else from the directory open) as a path.
    fn resolve(&self, name: &str) -> Vec<String> {
        let mut path = if name.starts_with('/') || name.starts_with('\\') {
            Vec::new()
        } else {
            self.directory.clone()
        };
        for part in name.split(['/', '\\']).filter(|part| !part.is_empty()) {
            match part {
                "." => {}
                ".." => {
                    path.pop();
                }
                // an 8.3 name with no extension ends with its dot (UniDOS opens folders so)
                part => path.push(part.strip_suffix('.').unwrap_or(part).to_owned()),
            }
        }
        path
    }

    /// What `path` is: a directory, a file and its size, or nothing.
    fn lookup(&mut self, path: &[String]) -> Option<(bool, u64)> {
        if path.is_empty() {
            return self.volume.is_some().then_some((true, 0));
        }
        self.volume.as_mut()?.lookup(path)
    }

    fn file_open(&mut self) {
        self.transfer = Transfer::None;
        let name = self.file_name.clone();
        if name.contains(['*', '?']) {
            let (directory, pattern) = match name.rsplit_once(['/', '\\']) {
                Some((directory, pattern)) => (
                    self.resolve(if directory.is_empty() { "/" } else { directory }),
                    pattern.to_owned(),
                ),
                None => (self.directory.clone(), name.clone()),
            };
            let entries = self.entries(&directory).unwrap_or_default();
            let matching = entries
                .into_iter()
                .filter(|entry| wildcard(&pattern, &entry_name(entry)))
                .collect();
            self.enumeration = Some((matching, 0));
            self.next_entry();
            return;
        }
        let path = self.resolve(&name);
        match self.lookup(&path) {
            Some((true, _)) => {
                self.directory = path.clone();
                self.open = Some(OpenEntry {
                    path,
                    directory: true,
                    position: 0,
                });
                self.raise(ERR_OPEN_DIR);
            }
            Some((false, _)) => {
                self.open = Some(OpenEntry {
                    path,
                    directory: false,
                    position: 0,
                });
                self.raise(USB_INT_SUCCESS);
            }
            None => {
                self.open = None;
                self.raise(ERR_MISS_FILE);
            }
        }
    }

    /// The next entry of an enumeration: its directory entry in the buffer.
    fn next_entry(&mut self) {
        match &mut self.enumeration {
            Some((entries, next)) if *next < entries.len() => {
                self.buffer = entries[*next].to_vec();
                *next += 1;
                self.raise(USB_INT_DISK_READ);
            }
            _ => {
                self.enumeration = None;
                self.raise(ERR_MISS_FILE);
            }
        }
    }

    /// The directory entries of `path`, as the CH376 gives them (32 bytes, FAT's).
    fn entries(&mut self, path: &[String]) -> io::Result<Vec<[u8; 32]>> {
        self.volume()?.entries(path)
    }

    fn open_size(&mut self) -> Option<u64> {
        let open = self.open.clone()?;
        self.lookup(&open.path).map(|(_, size)| size)
    }

    /// The directory entry of the file open.
    fn open_entry_info(&mut self) -> Option<[u8; 32]> {
        let open = self.open.clone()?;
        let (parent, name) = open
            .path
            .split_last()
            .map(|(name, parent)| (parent.to_vec(), name.clone()))?;
        self.entries(&parent)
            .ok()?
            .into_iter()
            .find(|entry| {
                entry_name(entry).eq_ignore_ascii_case(&name)
                    || entry_name(entry)
                        .trim_end_matches('.')
                        .eq_ignore_ascii_case(&name)
            })
            .or_else(|| {
                // a long name: the volume's own lookup
                self.lookup(&open.path).map(|(directory, size)| {
                    let mut entry = [0u8; 32];
                    entry[..11].copy_from_slice(&short_bytes(&name));
                    entry[11] = if directory { 0x10 } else { 0x20 };
                    entry[28..32].copy_from_slice(&(size as u32).to_le_bytes());
                    entry
                })
            })
    }

    /// DIR_INFO_SAVE: the entry in the buffer written back; here, a new name renames the file.
    fn dir_info_save(&mut self) {
        let (Some(open), Some(current)) = (self.open.clone(), self.open_entry_info()) else {
            return self.raise(ERR_MISS_FILE);
        };
        if self.buffer.len() >= 11 && self.buffer[..11] != current[..11] {
            let mut new_name = [0u8; 32];
            new_name[..11].copy_from_slice(&self.buffer[..11]);
            let new_name = entry_name(&new_name);
            let mut to = open.path.clone();
            *to.last_mut().unwrap() = new_name.clone();
            let renamed = self
                .volume()
                .and_then(|volume| volume.rename(&open.path, &to));
            if renamed.is_err() {
                return self.raise(USB_INT_DISK_ERR);
            }
            let mut path = open.path.clone();
            *path.last_mut().unwrap() = new_name;
            self.open = Some(OpenEntry { path, ..open });
        }
        self.raise(USB_INT_SUCCESS);
    }

    fn file_create(&mut self) {
        let path = self.resolve(&self.file_name.clone());
        let created = self.volume().and_then(|volume| volume.create(&path));
        match created {
            Ok(()) => {
                self.open = Some(OpenEntry {
                    path,
                    directory: false,
                    position: 0,
                });
                self.raise(USB_INT_SUCCESS);
            }
            Err(_) => self.raise(ERR_DISK_FULL),
        }
    }

    fn dir_create(&mut self) {
        let path = self.resolve(&self.file_name.clone());
        if let Some((directory, _)) = self.lookup(&path) {
            if !directory {
                return self.raise(ERR_FOUND_NAME);
            }
        } else {
            if self
                .volume()
                .and_then(|volume| volume.create_dir(&path))
                .is_err()
            {
                return self.raise(ERR_DISK_FULL);
            }
        }
        self.directory = path.clone();
        self.open = Some(OpenEntry {
            path,
            directory: true,
            position: 0,
        });
        self.raise(USB_INT_SUCCESS);
    }

    fn file_erase(&mut self) {
        // the file open (closed first by the driver), or the name given
        let path = match &self.open {
            Some(open) => open.path.clone(),
            None => self.resolve(&self.file_name.clone()),
        };
        match self.volume().and_then(|volume| volume.remove(&path)) {
            Ok(()) => {
                self.open = None;
                self.raise(USB_INT_SUCCESS);
            }
            Err(_) => self.raise(ERR_MISS_FILE),
        }
    }

    /// BYTE_READ/BYTE_RD_GO: the next chunk of the file, or the end.
    fn byte_read_chunk(&mut self) {
        let Transfer::ByteRead { left, chunk } = self.transfer else {
            return self.raise(USB_INT_SUCCESS);
        };
        let left = left - chunk.min(left);
        let Some(open) = self.open.clone().filter(|open| !open.directory) else {
            self.transfer = Transfer::None;
            return self.raise(USB_INT_SUCCESS);
        };
        let size = self.open_size().unwrap_or(0);
        let count = u64::from(left)
            .min(CHUNK as u64)
            .min(size.saturating_sub(open.position));
        if count == 0 {
            self.transfer = Transfer::None;
            return self.raise(USB_INT_SUCCESS);
        }
        let data = self
            .volume()
            .and_then(|volume| volume.read(&open.path, open.position, count as usize));
        match data {
            Ok(data) => {
                if let Some(open) = &mut self.open {
                    open.position += data.len() as u64;
                }
                self.transfer = Transfer::ByteRead {
                    left,
                    chunk: data.len() as u32,
                };
                self.buffer = data;
                self.raise(USB_INT_DISK_READ);
            }
            Err(_) => {
                self.transfer = Transfer::None;
                self.raise(USB_INT_DISK_ERR);
            }
        }
    }

    /// How many bytes WR_REQ_DATA asks for.
    fn request_size(&self) -> usize {
        match self.transfer {
            Transfer::ByteWrite { left } => (left as usize).min(CHUNK),
            Transfer::Disk { write: true, .. } => CHUNK,
            _ => 0,
        }
    }

    /// The bytes the CPU wrote (WR_REQ_DATA, WR_HOST_DATA): to the file, or to the sector.
    fn store_written(&mut self) {
        let data = std::mem::take(&mut self.buffer);
        match self.transfer {
            Transfer::ByteWrite { left } => {
                let Some(open) = self.open.clone() else {
                    return;
                };
                let written = self
                    .volume()
                    .and_then(|volume| volume.write(&open.path, open.position, &data));
                if written.is_ok() {
                    if let Some(open) = &mut self.open {
                        open.position += data.len() as u64;
                    }
                    self.transfer = Transfer::ByteWrite {
                        left: left.saturating_sub(data.len() as u32),
                    };
                }
            }
            Transfer::Disk {
                lba,
                offset,
                write: true,
                ..
            } => {
                if let Some(storage) = self.volume.as_mut().and_then(|volume| volume.sectors()) {
                    if storage.seek(SeekFrom::Start(lba * SECTOR + offset)).is_ok() {
                        let _ = storage.write_all(&data);
                    }
                }
                if let Transfer::Disk { offset, .. } = &mut self.transfer {
                    *offset += data.len() as u64;
                }
            }
            _ => {}
        }
    }

    /// DISK_READ/DISK_WRITE and their GOs: the next 64 bytes, the next sector, or the end.
    fn disk_step(&mut self, go: bool) {
        let Transfer::Disk {
            mut lba,
            mut left,
            mut offset,
            write,
        } = self.transfer
        else {
            return self.raise(USB_INT_SUCCESS);
        };
        if go && !write {
            offset += CHUNK as u64;
        }
        if offset >= SECTOR {
            offset = 0;
            lba += 1;
            left = left.saturating_sub(1);
        }
        if left == 0 {
            self.transfer = Transfer::None;
            if let Some(storage) = self.volume.as_mut().and_then(|volume| volume.sectors()) {
                let _ = storage.flush();
            }
            return self.raise(USB_INT_SUCCESS);
        }
        self.transfer = Transfer::Disk {
            lba,
            left,
            offset,
            write,
        };
        if write {
            return self.raise(USB_INT_DISK_WRITE);
        }
        let mut data = vec![0u8; CHUNK];
        let read = self
            .volume
            .as_mut()
            .and_then(|volume| volume.sectors())
            .is_some_and(|storage| {
                storage.seek(SeekFrom::Start(lba * SECTOR + offset)).is_ok()
                    && storage.read_exact(&mut data).is_ok()
            });
        if read {
            self.buffer = data;
            self.raise(USB_INT_DISK_READ);
        } else {
            self.transfer = Transfer::None;
            self.raise(USB_INT_DISK_ERR);
        }
    }
}

/// What the CH376 reads and writes: a FAT volume's files (paths from the root, names matched
/// ignoring case, by their 8.3 or long name) and, for an image, its raw sectors.
pub trait Volume: Send {
    /// A directory, or a file and its size.
    fn lookup(&mut self, path: &[String]) -> Option<(bool, u64)>;
    /// The entries of a directory, as FAT stores them (32 bytes).
    fn entries(&mut self, path: &[String]) -> io::Result<Vec<[u8; 32]>>;
    fn read(&mut self, path: &[String], position: u64, count: usize) -> io::Result<Vec<u8>>;
    fn write(&mut self, path: &[String], position: u64, data: &[u8]) -> io::Result<()>;
    /// A new, empty file (an existing one emptied).
    fn create(&mut self, path: &[String]) -> io::Result<()>;
    fn create_dir(&mut self, path: &[String]) -> io::Result<()>;
    fn remove(&mut self, path: &[String]) -> io::Result<()>;
    fn rename(&mut self, from: &[String], to: &[String]) -> io::Result<()>;
    /// Sectors in all, free, and the FAT type (1: FAT12, 6: FAT16, &0C: FAT32).
    fn query(&mut self) -> io::Result<(u32, u32, u8)>;
    /// The key's size, in sectors.
    fn capacity(&mut self) -> u64;
    /// The raw sectors (none for a folder).
    fn sectors(&mut self) -> Option<&mut dyn Storage>;
}

/// A FAT image file: the file system through `fatfs`, mounted afresh for each operation (so
/// coherent with the raw sectors).
pub struct FatImage {
    storage: Box<dyn Storage>,
    /// Where the FAT volume starts in the image.
    offset: u64,
}

impl FatImage {
    pub fn new(mut storage: Box<dyn Storage>) -> Self {
        let offset = volume_offset(storage.as_mut());
        Self { storage, offset }
    }

    fn with_fs<R>(
        &mut self,
        f: impl FnOnce(&fatfs::FileSystem<&mut Partition>) -> io::Result<R>,
    ) -> io::Result<R> {
        let mut partition = Partition {
            storage: self.storage.as_mut(),
            offset: self.offset,
        };
        partition.seek(SeekFrom::Start(0))?;
        let fs = fatfs::FileSystem::new(&mut partition, fatfs::FsOptions::new())?;
        f(&fs)
    }
}

impl Volume for FatImage {
    fn lookup(&mut self, path: &[String]) -> Option<(bool, u64)> {
        let joined = path.join("/");
        self.with_fs(|fs| {
            let root = fs.root_dir();
            let (parent, name) = match joined.rsplit_once('/') {
                Some((parent, name)) => (root.open_dir(parent)?, name.to_owned()),
                None => (root, joined.clone()),
            };
            for entry in parent.iter() {
                let entry = entry?;
                if entry.short_file_name().eq_ignore_ascii_case(&name)
                    || entry.file_name().eq_ignore_ascii_case(&name)
                {
                    return Ok(Some((entry.is_dir(), entry.len())));
                }
            }
            Ok(None)
        })
        .ok()
        .flatten()
    }

    fn entries(&mut self, path: &[String]) -> io::Result<Vec<[u8; 32]>> {
        let joined = path.join("/");
        self.with_fs(|fs| {
            let directory = if joined.is_empty() {
                fs.root_dir()
            } else {
                fs.root_dir().open_dir(&joined)?
            };
            let mut entries = Vec::new();
            for entry in directory.iter() {
                entries.push(dir_entry_bytes(&entry?));
            }
            Ok(entries)
        })
    }

    fn read(&mut self, path: &[String], position: u64, count: usize) -> io::Result<Vec<u8>> {
        let joined = path.join("/");
        self.with_fs(|fs| {
            let mut file = fs.root_dir().open_file(&joined)?;
            file.seek(SeekFrom::Start(position))?;
            let mut data = vec![0u8; count];
            file.read_exact(&mut data)?;
            Ok(data)
        })
    }

    fn write(&mut self, path: &[String], position: u64, data: &[u8]) -> io::Result<()> {
        let joined = path.join("/");
        self.with_fs(|fs| {
            let mut file = fs.root_dir().open_file(&joined)?;
            file.seek(SeekFrom::Start(position))?;
            file.write_all(data)?;
            file.flush()
        })
    }

    fn create(&mut self, path: &[String]) -> io::Result<()> {
        let joined = path.join("/");
        self.with_fs(|fs| fs.root_dir().create_file(&joined)?.truncate())
    }

    fn create_dir(&mut self, path: &[String]) -> io::Result<()> {
        let joined = path.join("/");
        self.with_fs(|fs| fs.root_dir().create_dir(&joined).map(drop))
    }

    fn remove(&mut self, path: &[String]) -> io::Result<()> {
        let joined = path.join("/");
        self.with_fs(|fs| fs.root_dir().remove(&joined))
    }

    fn rename(&mut self, from: &[String], to: &[String]) -> io::Result<()> {
        let (from, to) = (from.join("/"), to.join("/"));
        self.with_fs(|fs| {
            let root = fs.root_dir();
            root.rename(&from, &root, &to)
        })
    }

    fn query(&mut self) -> io::Result<(u32, u32, u8)> {
        self.with_fs(|fs| {
            let stats = fs.stats()?;
            let per_cluster = stats.cluster_size() / 512;
            let kind = match fs.fat_type() {
                fatfs::FatType::Fat12 => 0x01,
                fatfs::FatType::Fat16 => 0x06,
                fatfs::FatType::Fat32 => 0x0c,
            };
            Ok((
                stats.total_clusters() * per_cluster,
                stats.free_clusters() * per_cluster,
                kind,
            ))
        })
    }

    fn capacity(&mut self) -> u64 {
        self.storage.seek(SeekFrom::End(0)).unwrap_or(0) / SECTOR
    }

    fn sectors(&mut self) -> Option<&mut dyn Storage> {
        Some(self.storage.as_mut())
    }
}

/// A folder of the host as the key's FAT volume: its files and folders under 8.3 names (upper
/// case, the name and extension cut to 8 and 3 characters), dot files left out. No raw sectors.
pub struct HostFolder {
    root: PathBuf,
}

/// What a folder reports as its size (a 2 GB key, half free).
const FOLDER_SECTORS: u32 = 4 * 1024 * 1024;

impl HostFolder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The host entries of a folder: (host name, its 8.3 name).
    fn listing(directory: &Path) -> Vec<(std::ffi::OsString, String)> {
        let mut names: Vec<_> = std::fs::read_dir(directory)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                    .map(|entry| {
                        let name = entry.file_name();
                        let short = short_name(&name.to_string_lossy());
                        (name, short)
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        names
    }

    /// The host path of `path`, each part matched to its 8.3 or host name, ignoring case; the
    /// last part as given when it does not exist (to create it).
    fn host_path(&self, path: &[String]) -> io::Result<PathBuf> {
        let mut host = self.root.clone();
        for (index, part) in path.iter().enumerate() {
            let found = Self::listing(&host).into_iter().find(|(name, short)| {
                short.eq_ignore_ascii_case(part)
                    || name.to_string_lossy().eq_ignore_ascii_case(part)
            });
            match found {
                Some((name, _)) => host.push(name),
                None if index + 1 == path.len() => host.push(part),
                None => return Err(io::Error::new(io::ErrorKind::NotFound, part.clone())),
            }
        }
        Ok(host)
    }
}

impl Volume for HostFolder {
    fn lookup(&mut self, path: &[String]) -> Option<(bool, u64)> {
        let metadata = std::fs::metadata(self.host_path(path).ok()?).ok()?;
        Some((metadata.is_dir(), metadata.len()))
    }

    fn entries(&mut self, path: &[String]) -> io::Result<Vec<[u8; 32]>> {
        let directory = self.host_path(path)?;
        let mut entries = Vec::new();
        for (name, short) in Self::listing(&directory) {
            let Ok(metadata) = std::fs::metadata(directory.join(&name)) else {
                continue;
            };
            let mut entry = [0u8; 32];
            entry[..11].copy_from_slice(&short_bytes(&short));
            entry[11] = match (metadata.is_dir(), metadata.permissions().readonly()) {
                (true, _) => 0x10,
                (false, true) => 0x21,
                (false, false) => 0x20,
            };
            if let Ok(modified) = metadata.modified() {
                let (date, time) = fat_date_time(modified);
                entry[22..24].copy_from_slice(&time.to_le_bytes());
                entry[24..26].copy_from_slice(&date.to_le_bytes());
            }
            entry[28..32].copy_from_slice(&(metadata.len() as u32).to_le_bytes());
            entries.push(entry);
        }
        Ok(entries)
    }

    fn read(&mut self, path: &[String], position: u64, count: usize) -> io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(self.host_path(path)?)?;
        file.seek(SeekFrom::Start(position))?;
        let mut data = vec![0u8; count];
        file.read_exact(&mut data)?;
        Ok(data)
    }

    fn write(&mut self, path: &[String], position: u64, data: &[u8]) -> io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(self.host_path(path)?)?;
        file.seek(SeekFrom::Start(position))?;
        file.write_all(data)
    }

    fn create(&mut self, path: &[String]) -> io::Result<()> {
        std::fs::File::create(self.host_path(path)?).map(drop)
    }

    fn create_dir(&mut self, path: &[String]) -> io::Result<()> {
        std::fs::create_dir(self.host_path(path)?)
    }

    fn remove(&mut self, path: &[String]) -> io::Result<()> {
        let host = self.host_path(path)?;
        if host.is_dir() {
            std::fs::remove_dir(host)
        } else {
            std::fs::remove_file(host)
        }
    }

    fn rename(&mut self, from: &[String], to: &[String]) -> io::Result<()> {
        std::fs::rename(self.host_path(from)?, self.host_path(to)?)
    }

    fn query(&mut self) -> io::Result<(u32, u32, u8)> {
        if self.root.is_dir() {
            Ok((FOLDER_SECTORS, FOLDER_SECTORS / 2, 0x0c))
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "no folder"))
        }
    }

    fn capacity(&mut self) -> u64 {
        FOLDER_SECTORS.into()
    }

    fn sectors(&mut self) -> Option<&mut dyn Storage> {
        None
    }
}

/// A host name as an 8.3 name: upper case, the name and extension cut to 8 and 3 characters,
/// spaces and dots left out.
fn short_name(name: &str) -> String {
    let (base, extension) = match name.rsplit_once('.') {
        Some((base, extension)) if !base.is_empty() => (base, extension),
        _ => (name, ""),
    };
    let clean = |text: &str, length: usize| -> String {
        text.chars()
            .filter(|c| c.is_ascii_graphic() && *c != '.')
            .take(length)
            .collect::<String>()
            .to_ascii_uppercase()
    };
    let (base, extension) = (clean(base, 8), clean(extension, 3));
    if extension.is_empty() {
        base
    } else {
        format!("{base}.{extension}")
    }
}

/// A modification time as FAT's date and time (UTC).
fn fat_date_time(time: std::time::SystemTime) -> (u16, u16) {
    let seconds = time
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = crate::calendar::civil_from_days(days as i64);
    let date = ((year.clamp(1980, 2107) - 1980) as u16) << 9 | (month as u16) << 5 | day as u16;
    let time = ((of_day / 3600) as u16) << 11
        | ((of_day / 60 % 60) as u16) << 5
        | (of_day % 60 / 2) as u16;
    (date, time)
}

/// The FAT volume in an image: the whole of it, or its first FAT partition.
struct Partition<'a> {
    storage: &'a mut dyn Storage,
    offset: u64,
}

impl Read for Partition<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.storage.read(buf)
    }
}

impl Write for Partition<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.storage.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.storage.flush()
    }
}

impl Seek for Partition<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let absolute = match position {
            SeekFrom::Start(offset) => self.storage.seek(SeekFrom::Start(self.offset + offset))?,
            other => self.storage.seek(other)?,
        };
        Ok(absolute.saturating_sub(self.offset))
    }
}

/// Where the FAT volume starts: 0 when sector 0 is a FAT boot sector, else the first FAT
/// partition of the MBR.
fn volume_offset(storage: &mut dyn Storage) -> u64 {
    let mut sector = [0u8; 512];
    if storage.seek(SeekFrom::Start(0)).is_err() || storage.read_exact(&mut sector).is_err() {
        return 0;
    }
    let fat_boot_sector = matches!(sector[0], 0xeb | 0xe9)
        && u16::from_le_bytes([sector[11], sector[12]]) == 512
        && (sector[54..57] == *b"FAT" || sector[82..85] == *b"FAT");
    if fat_boot_sector || sector[510..512] != [0x55, 0xaa] {
        return 0;
    }
    (0..4)
        .map(|index| &sector[446 + index * 16..462 + index * 16])
        .find(|entry| matches!(entry[4], 0x01 | 0x04 | 0x06 | 0x0b | 0x0c | 0x0e))
        .map_or(0, |entry| {
            u64::from(u32::from_le_bytes([
                entry[8], entry[9], entry[10], entry[11],
            ])) * SECTOR
        })
}

/// A directory entry as FAT stores it (32 bytes), from fatfs's.
fn dir_entry_bytes<T: fatfs::ReadWriteSeek>(entry: &fatfs::DirEntry<T>) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[..11].copy_from_slice(&short_bytes(&entry.short_file_name()));
    bytes[11] = entry.attributes().bits();
    let modified = entry.modified();
    let time = (modified.time.hour << 11) | (modified.time.min << 5) | (modified.time.sec / 2);
    let date = ((modified.date.year.saturating_sub(1980)) << 9)
        | (modified.date.month << 5)
        | modified.date.day;
    bytes[22..24].copy_from_slice(&time.to_le_bytes());
    bytes[24..26].copy_from_slice(&date.to_le_bytes());
    bytes[28..32].copy_from_slice(&(entry.len() as u32).to_le_bytes());
    bytes
}

/// "NAME.EXT" as the 11 bytes of a directory entry.
fn short_bytes(name: &str) -> [u8; 11] {
    let mut bytes = [b' '; 11];
    let (base, extension) = match name {
        "." | ".." => (name, ""),
        _ => name.rsplit_once('.').unwrap_or((name, "")),
    };
    for (index, byte) in base.bytes().take(8).enumerate() {
        bytes[index] = byte.to_ascii_uppercase();
    }
    for (index, byte) in extension.bytes().take(3).enumerate() {
        bytes[8 + index] = byte.to_ascii_uppercase();
    }
    bytes
}

/// The name of a directory entry, "NAME.EXT" ("NAME" without extension).
fn entry_name(entry: &[u8; 32]) -> String {
    let base = String::from_utf8_lossy(&entry[..8]).trim_end().to_owned();
    let extension = String::from_utf8_lossy(&entry[8..11]).trim_end().to_owned();
    if extension.is_empty() {
        base
    } else {
        format!("{base}.{extension}")
    }
}

/// `*` and `?`, ignoring case; `*` alone matches all.
fn wildcard(pattern: &str, name: &str) -> bool {
    fn matches(pattern: &[u8], name: &[u8]) -> bool {
        match (pattern.first(), name.first()) {
            (None, None) => true,
            (Some(b'*'), _) => {
                matches(&pattern[1..], name) || (!name.is_empty() && matches(pattern, &name[1..]))
            }
            (Some(b'?'), Some(_)) => matches(&pattern[1..], &name[1..]),
            (Some(p), Some(n)) => p.eq_ignore_ascii_case(n) && matches(&pattern[1..], &name[1..]),
            _ => false,
        }
    }
    matches(pattern.as_bytes(), name.as_bytes())
        || (pattern.ends_with(".*")
            && matches(&pattern.as_bytes()[..pattern.len() - 2], name.as_bytes()))
}

/// The Albireo board: its CH376 at &FE80 (data) and &FE81 (command, status).
pub struct Albireo {
    pub ch376: Ch376,
    /// The data byte read by the IN under way.
    read_latch: u8,
}

impl Albireo {
    pub fn new(volume: Option<Box<dyn Volume>>) -> Self {
        Self {
            ch376: Ch376::new(volume),
            read_latch: 0,
        }
    }

    /// The board with `path` as its USB key: a folder of the host, or a FAT image file.
    pub fn open(path: &Path) -> Result<Self, String> {
        let volume: Box<dyn Volume> = if path.is_dir() {
            Box::new(HostFolder::new(path))
        } else {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            Box::new(FatImage::new(Box::new(file)))
        };
        Ok(Self::new(Some(volume)))
    }

    /// The board with a FAT image given as bytes as its USB key (a page's upload): what the CPC
    /// writes stays in memory.
    pub fn with_image(image: Vec<u8>) -> Self {
        Self::new(Some(Box::new(FatImage::new(Box::new(io::Cursor::new(
            image,
        ))))))
    }
}

impl ExpansionDevice for Albireo {
    fn tick(&mut self, port: &mut ExpansionPort) {
        if let Some((address, value)) = port.io_write_started() {
            match address {
                0xfe80 => self.ch376.write_data(value),
                0xfe81 => self.ch376.write_command(value),
                _ => {}
            }
        }
        if let Some(address) = port.io_read() {
            // once per IN, when it begins, and held on the bus until the Z80 latches it
            let first = port.io_read_started();
            match address {
                0xfe80 => {
                    if first {
                        self.read_latch = self.ch376.read_data();
                    }
                    port.drive_data(self.read_latch);
                }
                0xfe81 => port.drive_data(self.ch376.read_status()),
                _ => {}
            }
        }
    }

    fn reset(&mut self) {
        self.ch376.reset();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// A FAT image of `size` bytes, in memory, with `files` (name, content) in its root.
    fn image(size: usize, files: &[(&str, &[u8])]) -> io::Cursor<Vec<u8>> {
        let mut disk = io::Cursor::new(vec![0u8; size]);
        fatfs::format_volume(&mut disk, fatfs::FormatVolumeOptions::new()).unwrap();
        {
            let fs = fatfs::FileSystem::new(&mut disk, fatfs::FsOptions::new()).unwrap();
            for (name, content) in files {
                fs.root_dir()
                    .create_file(name)
                    .unwrap()
                    .write_all(content)
                    .unwrap();
            }
        }
        disk.set_position(0);
        disk
    }

    fn chip(files: &[(&str, &[u8])]) -> Ch376 {
        Ch376::new(Some(Box::new(FatImage::new(Box::new(image(
            1440 * 1024,
            files,
        ))))))
    }

    /// A command and its parameters; the interrupt it raised, if any (cleared, as GET_STATUS
    /// does).
    fn command(chip: &mut Ch376, command: u8, parameters: &[u8]) -> Option<u8> {
        chip.write_command(command);
        for &byte in parameters {
            chip.write_data(byte);
        }
        let interrupt = chip.interrupt;
        if interrupt.is_some() {
            assert_eq!(chip.read_status() & 0x80, 0, "INT# low");
            chip.write_command(CMD_GET_STATUS);
            assert_eq!(Some(chip.read_data()), interrupt);
            assert_eq!(chip.read_status() & 0x80, 0x80, "INT# released");
        }
        interrupt
    }

    fn read_buffer(chip: &mut Ch376) -> Vec<u8> {
        chip.write_command(CMD_RD_USB_DATA0);
        let length = chip.read_data();
        (0..length).map(|_| chip.read_data()).collect()
    }

    fn set_name(chip: &mut Ch376, name: &str) {
        let mut bytes = name.as_bytes().to_vec();
        bytes.push(0);
        command(chip, CMD_SET_FILE_NAME, &bytes);
    }

    fn mount(chip: &mut Ch376) {
        chip.write_command(CMD_CHECK_EXIST);
        chip.write_data(0x57);
        assert_eq!(chip.read_data(), 0xa8);
        chip.write_command(CMD_SET_USB_MODE);
        chip.write_data(6);
        assert_eq!(chip.read_data(), CMD_RET_SUCCESS);
        assert_eq!(chip.interrupt, Some(USB_INT_CONNECT));
        command(chip, CMD_GET_STATUS, &[]);
        assert_eq!(command(chip, CMD_DISK_CONNECT, &[]), Some(USB_INT_SUCCESS));
        assert_eq!(command(chip, CMD_DISK_MOUNT, &[]), Some(USB_INT_SUCCESS));
    }

    #[test]
    fn files_are_read_and_written_in_chunks() {
        let content: Vec<u8> = (0..200u32).map(|i| i as u8).collect();
        let mut chip = chip(&[("GAME.BIN", &content)]);
        mount(&mut chip);

        set_name(&mut chip, "/GAME.BIN");
        assert_eq!(
            command(&mut chip, CMD_FILE_OPEN, &[]),
            Some(USB_INT_SUCCESS)
        );
        chip.write_command(CMD_GET_FILE_SIZE);
        chip.write_data(0x68);
        let size: Vec<u8> = (0..4).map(|_| chip.read_data()).collect();
        assert_eq!(size, 200u32.to_le_bytes());
        let mut read = Vec::new();
        let mut interrupt = command(&mut chip, CMD_BYTE_READ, &300u16.to_le_bytes());
        while interrupt == Some(USB_INT_DISK_READ) {
            let chunk = read_buffer(&mut chip);
            assert!(chunk.len() <= CHUNK);
            read.extend(chunk);
            interrupt = command(&mut chip, CMD_BYTE_RD_GO, &[]);
        }
        assert_eq!(interrupt, Some(USB_INT_SUCCESS));
        assert_eq!(read, content, "the whole file, the request being larger");
        assert_eq!(
            command(&mut chip, CMD_FILE_CLOSE, &[0]),
            Some(USB_INT_SUCCESS)
        );

        // a new file, 100 bytes through WR_REQ_DATA
        set_name(&mut chip, "/SAVE.DAT");
        assert_eq!(
            command(&mut chip, CMD_FILE_CREATE, &[]),
            Some(USB_INT_SUCCESS)
        );
        let mut interrupt = command(&mut chip, CMD_BYTE_WRITE, &100u16.to_le_bytes());
        let mut written = 0u8;
        while interrupt == Some(USB_INT_DISK_WRITE) {
            chip.write_command(CMD_WR_REQ_DATA);
            let asked = chip.read_data();
            for _ in 0..asked {
                chip.write_data(written);
                written += 1;
            }
            interrupt = command(&mut chip, CMD_BYTE_WR_GO, &[]);
        }
        assert_eq!((interrupt, written), (Some(USB_INT_SUCCESS), 100));
        command(&mut chip, CMD_FILE_CLOSE, &[1]);
        set_name(&mut chip, "/SAVE.DAT");
        command(&mut chip, CMD_FILE_OPEN, &[]);
        command(&mut chip, CMD_BYTE_LOCATE, &50u32.to_le_bytes());
        assert_eq!(
            command(&mut chip, CMD_BYTE_READ, &2u16.to_le_bytes()),
            Some(USB_INT_DISK_READ)
        );
        assert_eq!(read_buffer(&mut chip), vec![50, 51]);

        set_name(&mut chip, "/NOWHERE.TXT");
        assert_eq!(command(&mut chip, CMD_FILE_OPEN, &[]), Some(ERR_MISS_FILE));
    }

    #[test]
    fn directories_are_enumerated_and_entered() {
        let mut chip = chip(&[("ONE.BAS", b"10"), ("TWO.BIN", b"xx")]);
        mount(&mut chip);
        set_name(&mut chip, "/GAMES");
        assert_eq!(
            command(&mut chip, CMD_DIR_CREATE, &[]),
            Some(USB_INT_SUCCESS)
        );
        set_name(&mut chip, "/");
        assert_eq!(command(&mut chip, CMD_FILE_OPEN, &[]), Some(ERR_OPEN_DIR));

        set_name(&mut chip, "*");
        let mut names = Vec::new();
        let mut interrupt = command(&mut chip, CMD_FILE_OPEN, &[]);
        while interrupt == Some(USB_INT_DISK_READ) {
            let entry: [u8; 32] = read_buffer(&mut chip).try_into().unwrap();
            names.push((
                entry_name(&entry),
                entry[11] & 0x10 != 0,
                u32::from_le_bytes(entry[28..32].try_into().unwrap()),
            ));
            interrupt = command(&mut chip, CMD_FILE_ENUM_GO, &[]);
        }
        assert_eq!(interrupt, Some(ERR_MISS_FILE), "the end");
        assert!(names.contains(&("ONE.BAS".into(), false, 2)), "{names:?}");
        assert!(names.contains(&("GAMES".into(), true, 0)), "{names:?}");

        // into the directory: names are relative to it
        set_name(&mut chip, "GAMES");
        assert_eq!(command(&mut chip, CMD_FILE_OPEN, &[]), Some(ERR_OPEN_DIR));
        set_name(&mut chip, "NEW.TXT");
        command(&mut chip, CMD_FILE_CREATE, &[]);
        command(&mut chip, CMD_FILE_CLOSE, &[1]);
        set_name(&mut chip, "/GAMES/NEW.TXT");
        assert_eq!(
            command(&mut chip, CMD_FILE_OPEN, &[]),
            Some(USB_INT_SUCCESS)
        );
        assert_eq!(
            command(&mut chip, CMD_DIR_INFO_READ, &[0xff]),
            Some(USB_INT_SUCCESS)
        );
        let mut entry = read_buffer(&mut chip);
        assert_eq!(&entry[..11], b"NEW     TXT");
        // a rename through the directory entry
        entry[..11].copy_from_slice(b"OLD     TXT");
        chip.write_command(CMD_WR_OFS_DATA);
        for byte in [0u8, 11].into_iter().chain(entry[..11].iter().copied()) {
            chip.write_data(byte);
        }
        assert_eq!(
            command(&mut chip, CMD_DIR_INFO_SAVE, &[]),
            Some(USB_INT_SUCCESS)
        );
        command(&mut chip, CMD_FILE_CLOSE, &[0]);
        set_name(&mut chip, "/GAMES/OLD.TXT");
        assert_eq!(
            command(&mut chip, CMD_FILE_OPEN, &[]),
            Some(USB_INT_SUCCESS)
        );
        command(&mut chip, CMD_FILE_CLOSE, &[0]);
        assert_eq!(
            command(&mut chip, CMD_FILE_ERASE, &[]),
            Some(USB_INT_SUCCESS)
        );
        assert_eq!(command(&mut chip, CMD_FILE_OPEN, &[]), Some(ERR_MISS_FILE));
    }

    #[test]
    fn sectors_are_read_and_written_raw() {
        let disk = image(1440 * 1024, &[]);
        let boot = disk.get_ref()[..512].to_vec();
        let mut chip = Ch376::new(Some(Box::new(FatImage::new(Box::new(disk)))));
        mount(&mut chip);
        let mut parameters = 0u32.to_le_bytes().to_vec();
        parameters.push(1);
        let mut read = Vec::new();
        let mut interrupt = command(&mut chip, CMD_DISK_READ, &parameters);
        while interrupt == Some(USB_INT_DISK_READ) {
            read.extend(read_buffer(&mut chip));
            interrupt = command(&mut chip, CMD_DISK_RD_GO, &[]);
        }
        assert_eq!(interrupt, Some(USB_INT_SUCCESS));
        assert_eq!(read, boot, "sector 0, in 8 chunks");

        // write sector 100, then read it back
        let mut parameters = 100u32.to_le_bytes().to_vec();
        parameters.push(1);
        let mut interrupt = command(&mut chip, CMD_DISK_WRITE, &parameters);
        let mut chunks = 0u8;
        while interrupt == Some(USB_INT_DISK_WRITE) {
            chip.write_command(CMD_WR_HOST_DATA);
            chip.write_data(64);
            for _ in 0..64 {
                chip.write_data(chunks);
            }
            chunks += 1;
            interrupt = command(&mut chip, CMD_DISK_WR_GO, &[]);
        }
        assert_eq!((interrupt, chunks), (Some(USB_INT_SUCCESS), 8));
        let mut parameters = 100u32.to_le_bytes().to_vec();
        parameters.push(1);
        command(&mut chip, CMD_DISK_READ, &parameters);
        assert_eq!(read_buffer(&mut chip), vec![0; 64]);
        command(&mut chip, CMD_DISK_RD_GO, &[]);
        assert_eq!(read_buffer(&mut chip), vec![1; 64]);

        command(&mut chip, CMD_DISK_CAPACITY, &[]);
        assert_eq!(read_buffer(&mut chip), (1440u32 * 2).to_le_bytes());
    }

    #[test]
    fn a_host_folder_as_the_key() {
        let root = std::env::temp_dir().join(format!("bemu-ch376-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("games")).unwrap();
        std::fs::write(root.join("A Long Name.text"), b"long").unwrap();
        std::fs::write(root.join("games/demo.bin"), vec![7u8; 100]).unwrap();
        let mut chip = Ch376::new(Some(Box::new(HostFolder::new(&root))));
        mount(&mut chip);

        set_name(&mut chip, "/*");
        let mut names = Vec::new();
        let mut interrupt = command(&mut chip, CMD_FILE_OPEN, &[]);
        while interrupt == Some(USB_INT_DISK_READ) {
            let entry: [u8; 32] = read_buffer(&mut chip).try_into().unwrap();
            names.push(entry_name(&entry));
            interrupt = command(&mut chip, CMD_FILE_ENUM_GO, &[]);
        }
        assert_eq!(names, vec!["ALONGNAM.TEX".to_owned(), "GAMES".to_owned()]);

        // 8.3 names reach the host's
        set_name(&mut chip, "/GAMES/DEMO.BIN");
        assert_eq!(
            command(&mut chip, CMD_FILE_OPEN, &[]),
            Some(USB_INT_SUCCESS)
        );
        assert_eq!(
            command(&mut chip, CMD_BYTE_READ, &3u16.to_le_bytes()),
            Some(USB_INT_DISK_READ)
        );
        assert_eq!(read_buffer(&mut chip), vec![7, 7, 7]);
        command(&mut chip, CMD_FILE_CLOSE, &[0]);
        set_name(&mut chip, "/ALONGNAM.TEX");
        assert_eq!(
            command(&mut chip, CMD_FILE_OPEN, &[]),
            Some(USB_INT_SUCCESS)
        );
        command(&mut chip, CMD_FILE_CLOSE, &[0]);

        // UniDOS's way: the root, then the folder by its 8.3 name ("GAMES.": no extension),
        // then the file in it
        set_name(&mut chip, "/");
        assert_eq!(command(&mut chip, CMD_FILE_OPEN, &[]), Some(ERR_OPEN_DIR));
        set_name(&mut chip, "GAMES.");
        assert_eq!(
            command(&mut chip, CMD_FILE_OPEN, &[]),
            Some(ERR_OPEN_DIR),
            "the folder entered"
        );
        set_name(&mut chip, "DEMO.BIN");
        assert_eq!(
            command(&mut chip, CMD_FILE_OPEN, &[]),
            Some(USB_INT_SUCCESS)
        );
        command(&mut chip, CMD_FILE_CLOSE, &[0]);

        // a file written, then on the host
        set_name(&mut chip, "/SAVE.BAS");
        command(&mut chip, CMD_FILE_CREATE, &[]);
        command(&mut chip, CMD_BYTE_WRITE, &2u16.to_le_bytes());
        chip.write_command(CMD_WR_REQ_DATA);
        assert_eq!(chip.read_data(), 2);
        chip.write_data(b'O');
        chip.write_data(b'K');
        assert_eq!(
            command(&mut chip, CMD_BYTE_WR_GO, &[]),
            Some(USB_INT_SUCCESS)
        );
        command(&mut chip, CMD_FILE_CLOSE, &[1]);
        assert_eq!(std::fs::read(root.join("SAVE.BAS")).unwrap(), b"OK");

        // no raw sectors
        let mut parameters = 0u32.to_le_bytes().to_vec();
        parameters.push(1);
        assert_eq!(
            command(&mut chip, CMD_DISK_READ, &parameters),
            Some(USB_INT_DISK_ERR)
        );
    }

    #[test]
    fn the_sd_mode_mounts_the_volume_too() {
        // mode 3, the board's SD slot (UniDOS's "SD:"), sees the same volume as the USB key
        let mut chip = chip(&[]);
        chip.write_command(CMD_SET_USB_MODE);
        chip.write_data(3);
        assert_eq!(chip.read_data(), CMD_RET_SUCCESS);
        assert_eq!(
            command(&mut chip, CMD_DISK_MOUNT, &[]),
            Some(USB_INT_SUCCESS)
        );
        // a mode without a device (0) still has nothing to mount
        chip.write_command(CMD_SET_USB_MODE);
        chip.write_data(0);
        assert_eq!(chip.read_data(), CMD_RET_SUCCESS);
        assert_eq!(
            command(&mut chip, CMD_DISK_MOUNT, &[]),
            Some(USB_INT_DISK_ERR)
        );
    }

    #[test]
    fn a_partitioned_key() {
        // an MBR whose first partition (FAT16, type 6) starts at sector 64
        let volume = image(1440 * 1024, &[("INSIDE.TXT", b"hi")]).into_inner();
        let mut disk = vec![0u8; 64 * 512];
        disk[446 + 4] = 0x06;
        disk[446 + 8..446 + 12].copy_from_slice(&64u32.to_le_bytes());
        disk[510] = 0x55;
        disk[511] = 0xaa;
        disk.extend(volume);
        let mut chip = Ch376::new(Some(Box::new(FatImage::new(Box::new(io::Cursor::new(
            disk,
        ))))));
        mount(&mut chip);
        set_name(&mut chip, "/INSIDE.TXT");
        assert_eq!(
            command(&mut chip, CMD_FILE_OPEN, &[]),
            Some(USB_INT_SUCCESS)
        );
    }
}
