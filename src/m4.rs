//! The M4 board (Duke's WiFi/SD expansion), on the expansion port, its SD card a host folder, or
//! files in memory (a web page's: [`MemoryCard`]).
//!
//! The board's ROM (Duke's open source M4ROM, https://github.com/M4Duke/m4rom, v2.0.8 as
//! M4FIRM.BIN carries it, in `roms/m4rom.rom`) is served in upper ROM slot 6 through ROMDIS
//! and the data bus. The ROM talks to the board's microcontroller
//! through two ports: the bytes of a command go to &FExx (a length, the command word, low byte
//! first, its parameters), any byte to &FCxx runs it. The board answers at once by writing into
//! its ROM image: the response at &E800 (a length, the command echoed, the payload from &E803);
//! some commands also write the ROM's configuration (&F400) and work areas. The commands and
//! their formats are those M4ROM.s (v2.0.6) sends and reads.
//!
//! What the board does on its own (WiFi, HTTP, the ROM manager, disc images mounted from its
//! web page) is not emulated: those commands answer "not supported". (The WiFi is the host's
//! network where there is one: not in a web page.)

use std::any::Any;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use cpclib_disc::amsdos::{AmsdosCatalog, AmsdosManagerNonMut};
use cpclib_disc::edsk::{ExtendedDsk, Head};

use crate::chip::Chip;
use crate::expansion_port::{ExpansionDevice, ExpansionPin, ExpansionPort};

/// The upper ROM slot of the M4's ROM, unless told another (with UniDOS, above 16: 127).
pub const M4_ROM_SLOT: u8 = 6;

/// The slot of the M4's ROM beside UniDOS: above 16, where UniDOS initialises it after itself
/// (in slot 6 it takes the discs over: AMSDOS's drive A is not found any more).
pub const M4_ROM_SLOT_WITH_UNIDOS: u8 = 127;

/// Where the response goes, in the ROM (&E800).
const RESPONSE: usize = 0x2800;
/// The ROM's configuration area (&F400).
const CONFIG: usize = 0x3400;
const UNSUPPORTED: &str = "Not supported by bemu's M4.";

const C_OPEN: u16 = 0x4301;
const C_READ: u16 = 0x4302;
const C_WRITE: u16 = 0x4303;
const C_CLOSE: u16 = 0x4304;
const C_SEEK: u16 = 0x4305;
const C_READDIR: u16 = 0x4306;
const C_EOF: u16 = 0x4307;
const C_CD: u16 = 0x4308;
const C_FREE: u16 = 0x4309;
const C_FTELL: u16 = 0x430a;
const C_READSECTOR: u16 = 0x430b;
const C_WRITESECTOR: u16 = 0x430c;
const C_ERASEFILE: u16 = 0x430e;
const C_RENAME: u16 = 0x430f;
const C_MAKEDIR: u16 = 0x4310;
const C_FSIZE: u16 = 0x4311;
const C_READ2: u16 = 0x4312;
const C_GETPATH: u16 = 0x4313;
const C_FSTAT: u16 = 0x4316;
const C_SNAFLAT: u16 = 0x4317;
const C_WRITE2: u16 = 0x431b;
const C_NETSTAT: u16 = 0x4323;
const C_TIME: u16 = 0x4324;
const C_DIRSETARGS: u16 = 0x4325;
const C_VERSION: u16 = 0x4326;
const C_COPYFILE: u16 = 0x432a;
const C_ROMCP: u16 = 0x43fc;
const C_ROMWRITE: u16 = 0x43fd;
const C_CONFIG: u16 = 0x43fe;

/// FatFs's open modes and results, which the ROM uses.
const FA_WRITE: u8 = 0x02;
const FA_CREATE_NEW: u8 = 0x04;
const FA_CREATE_ALWAYS: u8 = 0x08;
const FA_OPEN_ALWAYS: u8 = 0x10;
const FA_OPEN_APPEND: u8 = 0x30;
/// Not FatFs's: the ROM's "give me a file descriptor of my own".
const FD_DYNAMIC: u8 = 0x80;
const FR_DISK_ERR: u8 = 1;
const FR_NO_FILE: u8 = 4;
const FR_NO_PATH: u8 = 5;
const FR_DENIED: u8 = 7;
const FR_EXIST: u8 = 8;
/// The end of a file, for C_READ2.
const EOF_STATUS: u8 = 20;

/// How a file is opened, as `std::fs::OpenOptions` says it (reading is always allowed).
#[derive(Clone, Copy, Default)]
struct OpenMode {
    write: bool,
    create: bool,
    truncate: bool,
    create_new: bool,
    append: bool,
}

/// What a file or a directory of the card is.
struct Meta {
    is_dir: bool,
    len: u64,
    read_only: bool,
}

/// A node of a [`MemoryCard`].
enum Node {
    Dir,
    File(Arc<Mutex<Vec<u8>>>),
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The parts of a path of the card (its root and `.` left out).
fn parts_of(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

/// An SD card kept in memory: files and folders by their names, as a web page gives them. It is
/// a handle: a clone is the same card, so the page keeps one to put files in and take them out
/// while the board has the other. Names are matched as given (the board looks them up ignoring
/// case, as FAT does; the page's calls do not).
#[derive(Clone, Default)]
pub struct MemoryCard(Arc<Mutex<BTreeMap<Vec<String>, Node>>>);

impl MemoryCard {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_dir_at(nodes: &BTreeMap<Vec<String>, Node>, key: &[String]) -> bool {
        key.is_empty() || matches!(nodes.get(key), Some(Node::Dir))
    }

    fn meta(&self, key: &[String]) -> Option<Meta> {
        let nodes = locked(&self.0);
        match nodes.get(key) {
            Some(Node::File(data)) => Some(Meta {
                is_dir: false,
                len: locked(data).len() as u64,
                read_only: false,
            }),
            Some(Node::Dir) => Some(Meta { is_dir: true, len: 0, read_only: false }),
            None if key.is_empty() => Some(Meta { is_dir: true, len: 0, read_only: false }),
            None => None,
        }
    }

    /// The names in directory `dir`.
    fn names(&self, dir: &[String]) -> io::Result<Vec<String>> {
        let nodes = locked(&self.0);
        if !Self::is_dir_at(&nodes, dir) {
            return Err(not_found());
        }
        Ok(nodes
            .keys()
            .filter(|key| key.len() == dir.len() + 1 && key.starts_with(dir))
            .map(|key| key[dir.len()].clone())
            .collect())
    }

    /// The directories above `key`, made.
    fn make_parents(nodes: &mut BTreeMap<Vec<String>, Node>, key: &[String]) {
        for depth in 1..key.len() {
            nodes.entry(key[..depth].to_vec()).or_insert(Node::Dir);
        }
    }

    fn open(&self, key: &[String], mode: OpenMode) -> io::Result<MemFile> {
        let mut nodes = locked(&self.0);
        let writable = mode.write || mode.append;
        match nodes.get(key) {
            Some(Node::File(data)) => {
                if mode.create_new {
                    return Err(io::Error::from(io::ErrorKind::AlreadyExists));
                }
                if mode.truncate && writable {
                    locked(data).clear();
                }
                Ok(MemFile { data: data.clone(), position: 0, writable, append: mode.append })
            }
            Some(Node::Dir) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            None => {
                let parent = &key[..key.len().saturating_sub(1)];
                if key.is_empty() || !Self::is_dir_at(&nodes, parent) {
                    return Err(not_found());
                }
                if !((mode.create || mode.create_new) && writable) {
                    return Err(not_found());
                }
                let data = Arc::new(Mutex::new(Vec::new()));
                nodes.insert(key.to_vec(), Node::File(data.clone()));
                Ok(MemFile { data, position: 0, writable, append: mode.append })
            }
        }
    }

    fn remove_file(&self, key: &[String]) -> io::Result<()> {
        let mut nodes = locked(&self.0);
        match nodes.get(key) {
            Some(Node::File(_)) => {
                nodes.remove(key);
                Ok(())
            }
            Some(Node::Dir) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            None => Err(not_found()),
        }
    }

    fn create_dir(&self, key: &[String]) -> io::Result<()> {
        let mut nodes = locked(&self.0);
        if key.is_empty() || nodes.contains_key(key) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        if !Self::is_dir_at(&nodes, &key[..key.len() - 1]) {
            return Err(not_found());
        }
        nodes.insert(key.to_vec(), Node::Dir);
        Ok(())
    }

    /// `from` (and what is below it) under the name `to`.
    fn rename(&self, from: &[String], to: &[String]) -> io::Result<()> {
        let mut nodes = locked(&self.0);
        if !nodes.contains_key(from) {
            return Err(not_found());
        }
        if nodes.contains_key(to) {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        if to.is_empty() || !Self::is_dir_at(&nodes, &to[..to.len() - 1]) || to.starts_with(from) {
            return Err(not_found());
        }
        let moved: Vec<Vec<String>> = nodes.keys().filter(|key| key.starts_with(from)).cloned().collect();
        for key in moved {
            if let Some(node) = nodes.remove(&key) {
                let mut renamed = to.to_vec();
                renamed.extend_from_slice(&key[from.len()..]);
                nodes.insert(renamed, node);
            }
        }
        Ok(())
    }

    fn read(&self, key: &[String]) -> io::Result<Vec<u8>> {
        match locked(&self.0).get(key) {
            Some(Node::File(data)) => Ok(locked(data).clone()),
            Some(Node::Dir) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            None => Err(not_found()),
        }
    }

    /// The page's `path` ("games/demo.bin") written with `data`, its folders made.
    pub fn write_file(&self, path: &str, data: Vec<u8>) -> Result<(), String> {
        let key = parts_of(Path::new(path));
        let mut nodes = locked(&self.0);
        if key.is_empty() || matches!(nodes.get(&key), Some(Node::Dir)) {
            return Err(format!("{path}: not a file name"));
        }
        for depth in 1..key.len() {
            if matches!(nodes.get(&key[..depth]), Some(Node::File(_))) {
                return Err(format!("{path}: {} is a file", key[..depth].join("/")));
            }
        }
        Self::make_parents(&mut nodes, &key);
        match nodes.get(&key) {
            Some(Node::File(file)) => *locked(file) = data,
            _ => {
                nodes.insert(key, Node::File(Arc::new(Mutex::new(data))));
            }
        }
        Ok(())
    }

    /// The page's `path`, if it is a file.
    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        self.read(&parts_of(Path::new(path))).ok()
    }

    /// A folder made (and the ones above it).
    pub fn make_dir(&self, path: &str) -> Result<(), String> {
        let key = parts_of(Path::new(path));
        let mut nodes = locked(&self.0);
        for depth in 1..=key.len() {
            if matches!(nodes.get(&key[..depth]), Some(Node::File(_))) {
                return Err(format!("{path}: {} is a file", key[..depth].join("/")));
            }
        }
        Self::make_parents(&mut nodes, &key);
        if !key.is_empty() {
            nodes.entry(key).or_insert(Node::Dir);
        }
        Ok(())
    }

    /// The folder `path`'s entries: (name, is a folder, size), folders first; none if it is not a folder.
    pub fn list(&self, path: &str) -> Option<Vec<(String, bool, u64)>> {
        let dir = parts_of(Path::new(path));
        let mut entries: Vec<(String, bool, u64)> = self
            .names(&dir)
            .ok()?
            .into_iter()
            .filter_map(|name| {
                let mut key = dir.clone();
                key.push(name.clone());
                self.meta(&key).map(|meta| (name, meta.is_dir, meta.len))
            })
            .collect();
        entries.sort_by_key(|(name, folder, _)| (!folder, name.to_ascii_uppercase()));
        Some(entries)
    }

    /// A file, or a folder and what is in it, taken away; false if there was none.
    pub fn remove(&self, path: &str) -> bool {
        let key = parts_of(Path::new(path));
        let mut nodes = locked(&self.0);
        let before = nodes.len();
        nodes.retain(|name, _| key.is_empty() || !name.starts_with(&key));
        nodes.len() != before
    }
}

/// A file of a [`MemoryCard`], open.
struct MemFile {
    data: Arc<Mutex<Vec<u8>>>,
    position: u64,
    writable: bool,
    append: bool,
}

impl Read for MemFile {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let data = locked(&self.data);
        let start = usize::try_from(self.position).unwrap_or(usize::MAX).min(data.len());
        let count = buffer.len().min(data.len() - start);
        buffer[..count].copy_from_slice(&data[start..start + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl Write for MemFile {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if !self.writable {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let mut data = locked(&self.data);
        if self.append {
            self.position = data.len() as u64;
        }
        let start = usize::try_from(self.position).unwrap_or(usize::MAX);
        if start > data.len() {
            data.resize(start, 0);
        }
        let overlap = buffer.len().min(data.len() - start);
        data[start..start + overlap].copy_from_slice(&buffer[..overlap]);
        data.extend_from_slice(&buffer[overlap..]);
        self.position += buffer.len() as u64;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for MemFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let size = locked(&self.data).len() as i64;
        let target = match to {
            SeekFrom::Start(offset) => i64::try_from(offset).unwrap_or(i64::MAX),
            SeekFrom::End(offset) => size.saturating_add(offset),
            SeekFrom::Current(offset) => (self.position as i64).saturating_add(offset),
        };
        if target < 0 {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        self.position = target as u64;
        Ok(self.position)
    }
}

/// Where the board's files are: a folder of the host (the paths are the host's), or a card in
/// memory (the paths are below `/`).
#[derive(Clone)]
enum SdCard {
    Host,
    Memory(MemoryCard),
}

impl SdCard {
    fn meta(&self, path: &Path) -> Option<Meta> {
        match self {
            Self::Host => std::fs::metadata(path).ok().map(|meta| Meta {
                is_dir: meta.is_dir(),
                len: meta.len(),
                read_only: meta.permissions().readonly(),
            }),
            Self::Memory(card) => card.meta(&parts_of(path)),
        }
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.meta(path).is_some_and(|meta| meta.is_dir)
    }

    fn is_file(&self, path: &Path) -> bool {
        self.meta(path).is_some_and(|meta| !meta.is_dir)
    }

    /// The names in directory `dir` (none if it can't be read).
    fn names(&self, dir: &Path) -> Vec<std::ffi::OsString> {
        match self {
            Self::Host => std::fs::read_dir(dir)
                .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
                .unwrap_or_default(),
            Self::Memory(card) => card
                .names(&parts_of(dir))
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self {
            Self::Host => std::fs::read(path),
            Self::Memory(card) => card.read(&parts_of(path)),
        }
    }

    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<OpenFile> {
        match self {
            Self::Host => {
                let mut options = std::fs::OpenOptions::new();
                options
                    .read(true)
                    .write(mode.write)
                    .create(mode.create)
                    .truncate(mode.truncate)
                    .create_new(mode.create_new)
                    .append(mode.append);
                options.open(path).map(OpenFile::Host)
            }
            Self::Memory(card) => card.open(&parts_of(path), mode).map(OpenFile::Shared),
        }
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        match self {
            Self::Host => std::fs::remove_file(path),
            Self::Memory(card) => card.remove_file(&parts_of(path)),
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        match self {
            Self::Host => std::fs::rename(from, to),
            Self::Memory(card) => card.rename(&parts_of(from), &parts_of(to)),
        }
    }

    fn copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        match self {
            Self::Host => std::fs::copy(from, to).map(drop),
            Self::Memory(card) => {
                let data = card.read(&parts_of(from))?;
                let mut file = card.open(
                    &parts_of(to),
                    OpenMode { write: true, create: true, truncate: true, ..OpenMode::default() },
                )?;
                file.write_all(&data)
            }
        }
    }

    fn create_dir(&self, path: &Path) -> io::Result<()> {
        match self {
            Self::Host => std::fs::create_dir(path),
            Self::Memory(card) => card.create_dir(&parts_of(path)),
        }
    }
}

/// An open file: on the SD card, or a snapshot's flattened memory (C_SNAFLAT).
enum OpenFile {
    Host(File),
    /// A file of a [`MemoryCard`].
    Shared(MemFile),
    Memory(Cursor<Vec<u8>>),
}

impl OpenFile {
    fn io(&mut self) -> &mut dyn ReadWriteSeek {
        match self {
            Self::Host(file) => file,
            Self::Shared(file) => file,
            Self::Memory(cursor) => cursor,
        }
    }

    fn size(&mut self) -> u64 {
        let io = self.io();
        let position = io.stream_position().unwrap_or(0);
        let size = io.seek(SeekFrom::End(0)).unwrap_or(0);
        let _ = io.seek(SeekFrom::Start(position));
        size
    }
}

trait ReadWriteSeek: Read + Write + Seek {}
impl<T: Read + Write + Seek> ReadWriteSeek for T {}

/// One directory entry, as listed.
struct DirEntry {
    name: String,
    /// Its 8.3 name, as CAT and |DIR show it.
    short: String,
    directory: bool,
    size: u64,
    read_only: bool,
}

/// The M4 board.
pub struct M4Board {
    /// The ROM, written by the board (responses, configuration).
    rom: Box<[u8; 0x4000]>,
    /// The upper ROM the CPU selected (&DFxx).
    selected_rom: u8,
    /// The slot of its ROM.
    slot: u8,
    /// The command being received.
    command: Vec<u8>,
    /// The SD card: where it is, and its root.
    card: SdCard,
    root: PathBuf,
    /// The current directory, below `root`.
    cwd: Vec<String>,
    files: BTreeMap<u8, OpenFile>,
    /// Files written under a name without extension: given one when closed, as AMSDOS does.
    unnamed: BTreeMap<u8, PathBuf>,
    /// The directory being listed, and the pattern it is filtered with.
    listing: Option<std::vec::IntoIter<DirEntry>>,
    pattern: String,
    /// The WiFi: the host's network.
    #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
    net: crate::m4_net::Network,
    /// Ticks since the sockets were last polled.
    #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
    ticks: u32,
}

/// Where the socket table is in the ROM (&FE00).
#[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
const SOCK_INFO: usize = 0x3e00;
/// How often the sockets are polled: every millisecond (16 MHz ticks).
#[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
const POLL_TICKS: u32 = 16_000;

impl M4Board {
    /// The board with its ROM, `sd` its SD card.
    pub fn new(rom: &[u8], sd: impl Into<PathBuf>) -> Result<Self, String> {
        let root = sd.into();
        if !root.is_dir() {
            return Err(format!("{}: not a folder", root.display()));
        }
        Self::build(rom, root, SdCard::Host)
    }

    fn build(rom: &[u8], root: PathBuf, card: SdCard) -> Result<Self, String> {
        let rom = match rom.len() {
            0x4080 => &rom[0x80..],
            len if len > 0x4000 => return Err(format!("the M4 ROM is 16K, not {len} bytes")),
            _ => rom,
        };
        let mut image = Box::new([0xffu8; 0x4000]);
        image[..rom.len()].copy_from_slice(rom);
        #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
        let net = {
            let mut net = crate::m4_net::Network::new(true);
            // the settings file is the host card's
            if matches!(card, SdCard::Host) {
                net.load_settings(&root);
            }
            net
        };
        Ok(Self {
            rom: image,
            selected_rom: 0,
            slot: M4_ROM_SLOT,
            command: Vec::new(),
            card,
            root,
            cwd: Vec::new(),
            files: BTreeMap::new(),
            unnamed: BTreeMap::new(),
            listing: None,
            pattern: String::new(),
            #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
            net,
            #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
            ticks: 0,
        })
    }

    /// The WiFi on (the host's network used, the default) or off; `|WIFI` switches it too.
    #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
    pub fn with_wifi(mut self, on: bool) -> Self {
        self.net.enabled = on;
        self
    }

    /// `name` answered with `target` (DNS and HTTP): a local server in place of a site.
    #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
    pub fn with_host(mut self, name: &str, target: &str) -> Self {
        self.net.map_host(name, target);
        self
    }

    /// The sockets moved on, and their table in the ROM.
    #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
    fn poll_network(&mut self) {
        self.net.poll();
        let table = self.net.sock_info();
        self.write_rom(SOCK_INFO, &table);
    }

    /// The board with its ROM, its SD card the files of `card` (a web page's).
    pub fn with_memory_card(rom: &[u8], card: MemoryCard) -> Result<Self, String> {
        Self::build(rom, PathBuf::from("/"), SdCard::Memory(card))
    }

    /// The board with the M4 ROM v2.0.8, `sd` its SD card.
    pub fn with_bundled_rom(sd: impl Into<PathBuf>) -> Result<Self, String> {
        Self::new(crate::roms::M4, sd)
    }

    /// [`Self::with_memory_card`] with the M4 ROM v2.0.8.
    pub fn with_bundled_rom_in_memory(card: MemoryCard) -> Result<Self, String> {
        Self::with_memory_card(crate::roms::M4, card)
    }

    /// Its ROM in slot `slot` instead.
    pub fn in_slot(mut self, slot: u8) -> Self {
        self.slot = slot;
        self
    }

    /// The ROM as the board has written it.
    pub fn rom(&self) -> &[u8; 0x4000] {
        &self.rom
    }

    /// The current directory, as the ROM shows it ("/", "/games").
    pub fn path(&self) -> String {
        format!("/{}", self.cwd.join("/"))
    }

    /// Run a command: the bytes sent to &FExx (the length byte first, which some of the ROM's
    /// senders leave at 0: the bytes received are what counts).
    pub fn execute(&mut self, packet: &[u8]) {
        if packet.len() < 3 {
            return;
        }
        let command = u16::from_le_bytes([packet[1], packet[2]]);
        let params = &packet[3..];
        if std::env::var_os("BEMU_M4_TRACE").is_some() {
            eprintln!("M4 {command:04X} {:02X?}", &params[..params.len().min(32)]);
        }
        #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
        {
            let directory = self.host_path(".").unwrap_or_else(|_| self.root.clone());
            let root = self.root.clone();
            if let Some(payload) = self.net.command(command, params, &directory, &root) {
                self.respond(packet[1], packet[2], &payload);
                let table = self.net.sock_info();
                self.write_rom(SOCK_INFO, &table);
                return;
            }
        }
        let payload = match command {
            C_OPEN => self.open(params),
            C_READ => self.read(params),
            C_READ2 => self.read2(params),
            C_WRITE => self.write(params),
            C_WRITE2 => {
                // fd, a 16-bit size, the data
                let mut data = params.to_vec();
                if data.len() >= 3 {
                    data.drain(1..3);
                }
                self.write(&data)
            }
            C_CLOSE => vec![if self.close(byte(params, 0)) { 0 } else { 0xff }],
            C_SEEK => self.seek(params),
            C_EOF => self.eof(params),
            C_FSIZE => self.with_file(params, |file| file.size().to_le_bytes()[..4].to_vec()),
            C_FTELL => self.with_file(params, |file| {
                file.io().stream_position().unwrap_or(0).to_le_bytes()[..4].to_vec()
            }),
            C_DIRSETARGS => {
                self.pattern = string(params);
                self.listing = None;
                Vec::new()
            }
            C_READDIR => self.read_dir(params),
            // the UniDOS node reads the number after the first two characters (a host folder
            // has no size of its own: 1 GB free)
            C_FREE => b"\r\n1048576K free\0".to_vec(),
            C_GETPATH => nul_terminated(&self.path()),
            C_CD => vec![if self.change_dir(&string(params)) {
                0
            } else {
                0xff
            }],
            C_ERASEFILE => self.status(
                self.host_path(&string(params))
                    .and_then(|path| self.card.remove_file(&path).map_err(|e| e.to_string())),
            ),
            C_RENAME => {
                let (from, to) = two_strings(params);
                self.status(self.host_path(&from).and_then(|from| {
                    let to = self.host_path(&to)?;
                    self.card.rename(&from, &to).map_err(|e| e.to_string())
                }))
            }
            C_COPYFILE => {
                let (from, to) = two_strings(params);
                self.status(self.host_path(&from).and_then(|from| {
                    let to = self.host_path(&to)?;
                    self.card.copy(&from, &to).map_err(|e| e.to_string())
                }))
            }
            C_MAKEDIR => self.status(
                self.host_path(&string(params))
                    .and_then(|path| self.card.create_dir(&path).map_err(|e| e.to_string())),
            ),
            C_FSTAT => self.fstat(&string(params)),
            C_SNAFLAT => self.snapshot_flat(params),
            C_READSECTOR => {
                // the disc images the board mounts from its web page: none here
                let mut payload = vec![0xff];
                payload.resize(1 + 512, 0xe5);
                payload
            }
            C_WRITESECTOR => vec![0xff],
            C_CONFIG => {
                let offset = usize::from(byte(params, 0));
                self.write_rom(CONFIG + offset, params.get(1..).unwrap_or(&[]));
                Vec::new()
            }
            C_ROMWRITE => {
                // destination, size, bank (&FF: the NMI ROM, not emulated), the data
                let destination = usize::from(word(params, 0)) & 0x3fff;
                let size = usize::from(word(params, 2));
                if byte(params, 4) != 0xff {
                    let data = params.get(5..).unwrap_or(&[]);
                    self.write_rom(destination, &data[..size.min(data.len())]);
                }
                Vec::new()
            }
            C_ROMCP => {
                // destination, source bank, source, destination bank, size: within the M4 ROM
                let destination = usize::from(word(params, 0)) & 0x3fff;
                let source = usize::from(word(params, 3)) & 0x3fff;
                let size = usize::from(word(params, 6));
                let end = (source + size).min(0x4000);
                let data = self.rom[source..end].to_vec();
                self.write_rom(destination, &data);
                Vec::new()
            }
            // short, as the firmware's: RulezCharge copies it into a fixed field of its help box
            // (a longer one ran over its code)
            C_VERSION => b"v2.0.8 (bemu)\0".to_vec(),
            C_TIME => nul_terminated(&now(self.time_zone())),
            // the last payload byte is the status: 5 would be "connected"
            C_NETSTAT => nul_terminated("No network in bemu."),
            _ => nul_terminated(UNSUPPORTED),
        };
        self.respond(packet[1], packet[2], &payload);
    }

    /// The time zone of the board's settings (`tz`, hours).
    fn time_zone(&self) -> i64 {
        #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
        return self.net.time_zone();
        #[cfg(not(all(feature = "m4-wifi", not(target_arch = "wasm32"))))]
        0
    }

    fn respond(&mut self, low: u8, high: u8, payload: &[u8]) {
        let payload = &payload[..payload.len().min(0xa00 - 3)];
        self.rom[RESPONSE] = (payload.len() + 2).min(255) as u8;
        self.rom[RESPONSE + 1] = low;
        self.rom[RESPONSE + 2] = high;
        self.rom[RESPONSE + 3..RESPONSE + 3 + payload.len()].copy_from_slice(payload);
    }

    fn write_rom(&mut self, offset: usize, data: &[u8]) {
        let end = (offset + data.len()).min(0x4000);
        if offset < end {
            self.rom[offset..end].copy_from_slice(&data[..end - offset]);
        }
    }

    /// 0, or an error and its message.
    fn status(&self, result: Result<(), String>) -> Vec<u8> {
        match result {
            Ok(()) => vec![0],
            Err(e) => {
                let mut payload = vec![0xff];
                payload.extend(nul_terminated(&e));
                payload
            }
        }
    }

    fn with_file(&mut self, params: &[u8], f: impl FnOnce(&mut OpenFile) -> Vec<u8>) -> Vec<u8> {
        match self.files.get_mut(&byte(params, 0)) {
            Some(file) => f(file),
            None => vec![0; 4],
        }
    }

    /// The host path of a name of the SD card: absolute (`/`) or from the current directory,
    /// each part matched ignoring case (FAT does), `..` not above the card.
    fn host_path(&self, name: &str) -> Result<PathBuf, String> {
        let mut parts = if name.starts_with('/') {
            Vec::new()
        } else {
            self.cwd.clone()
        };
        for part in name
            .split(['/', '\\'])
            .filter(|p| !p.is_empty() && *p != ".")
        {
            if part == ".." {
                parts.pop();
            } else {
                parts.push(part.to_owned());
            }
        }
        let mut path = self.root.clone();
        for part in &parts {
            // the long name, else the 8.3 one (ROMCON~1.BIN for romconfig.bin), the case ignored,
            // its spaces dropped as AMSDOS does (a name copied from CAT: "COLUMNS .BAS")
            let entries = fat_directory(&self.card, &path);
            let compact: String = part.chars().filter(|c| *c != ' ').collect();
            let short = compact.trim_end_matches('.');
            let found = entries
                .iter()
                .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case(part))
                .or_else(|| {
                    entries
                        .iter()
                        .find(|(_, name)| name.eq_ignore_ascii_case(short))
                })
                .map(|(name, _)| name.clone());
            match found {
                Some(found) => path.push(found),
                None => path.push(part),
            }
        }
        Ok(path)
    }

    fn change_dir(&mut self, name: &str) -> bool {
        let Ok(path) = self.host_path(name) else {
            return false;
        };
        // a DSK image is entered as a directory, its AMSDOS files in it (read only)
        let image = matches!(dsk_parts(&self.card, &path), Some((_, None)))
            && read_dsk(&self.card, &path)
                .inspect_err(|e| eprintln!("M4: {} can't be entered: {e}", path.display()))
                .is_ok();
        if !self.card.is_dir(&path) && !image {
            return false;
        }
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        self.cwd = relative
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        self.listing = None;
        true
    }

    /// C_OPEN: mode, name. Response: the fd, the result.
    fn open(&mut self, params: &[u8]) -> Vec<u8> {
        let mode = byte(params, 0);
        let name = open_name(params.get(1..).unwrap_or(&[]));
        let fd = if mode & FD_DYNAMIC != 0 {
            (3..=255)
                .find(|fd| !self.files.contains_key(fd))
                .unwrap_or(255)
        } else if mode & FA_WRITE != 0 {
            2
        } else {
            1
        };
        self.close(fd);
        let Ok(mut path) = self.host_path(&name) else {
            return vec![fd, FR_NO_PATH];
        };
        let writing = mode & FA_WRITE != 0;
        if let Some((image, Some(file))) = dsk_parts(&self.card, &path) {
            if writing {
                return vec![fd, FR_DENIED];
            }
            return match dsk_file(&self.card, &image, &file) {
                Some(data) => {
                    self.files.insert(fd, OpenFile::Memory(Cursor::new(data)));
                    vec![fd, 0]
                }
                None => vec![fd, FR_NO_FILE],
            };
        }
        if !writing && !self.card.is_file(&path) && !name.contains('.') {
            // as AMSDOS: the name as given, then .BAS, then .BIN
            for extension in ["bas", "bin"] {
                if let Ok(candidate) = self.host_path(&format!("{name}.{extension}")) {
                    if self.card.is_file(&candidate) {
                        path = candidate;
                        break;
                    }
                }
            }
        }
        if path.parent().is_some_and(|parent| !self.card.is_dir(parent)) {
            return vec![fd, FR_NO_PATH];
        }
        if self.card.is_dir(&path) {
            // FatFs opens files only
            return vec![fd, FR_NO_FILE];
        }
        let mut options = OpenMode { write: writing, ..OpenMode::default() };
        match mode & 0x3c {
            FA_CREATE_ALWAYS => {
                options.create = true;
                options.truncate = true;
            }
            FA_CREATE_NEW => options.create_new = true,
            FA_OPEN_ALWAYS => options.create = writing,
            FA_OPEN_APPEND => {
                options.create = true;
                options.append = true;
            }
            _ => {}
        }
        match self.card.open(&path, options) {
            Ok(file) => {
                self.files.insert(fd, file);
                let file_name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if writing && !file_name.contains('.') {
                    self.unnamed.insert(fd, path);
                }
                vec![fd, 0]
            }
            Err(e) => {
                let result = match e.kind() {
                    std::io::ErrorKind::NotFound => FR_NO_FILE,
                    std::io::ErrorKind::AlreadyExists => FR_EXIST,
                    std::io::ErrorKind::PermissionDenied => FR_DENIED,
                    _ => FR_DISK_ERR,
                };
                vec![fd, result]
            }
        }
    }

    /// Close `fd`; false when it was not open. A file written under a name without extension
    /// gets AMSDOS's: .BIN for a binary (its header says), .BAS otherwise.
    fn close(&mut self, fd: u8) -> bool {
        let Some(mut file) = self.files.remove(&fd) else {
            return false;
        };
        if let Some(path) = self.unnamed.remove(&fd) {
            let mut header = [0u8; 128];
            let _ = file.io().seek(SeekFrom::Start(0));
            let read = file.io().read(&mut header).unwrap_or(0);
            drop(file);
            let binary = read == 128 && amsdos_header_type(&header) == Some(2);
            let extension = if binary { "BIN" } else { "BAS" };
            let lower = path.file_name().is_some_and(|name| {
                !name
                    .to_string_lossy()
                    .chars()
                    .any(|c| c.is_ascii_uppercase())
            });
            let extension = if lower {
                extension.to_ascii_lowercase()
            } else {
                extension.to_owned()
            };
            let _ = self.card.rename(&path, &path.with_extension(extension));
        }
        true
    }

    /// Up to `size` bytes of a file, and whether its end was reached.
    fn read_bytes(&mut self, fd: u8, size: usize) -> Option<(Vec<u8>, bool)> {
        let file = self.files.get_mut(&fd)?;
        let mut data = vec![0u8; size];
        let mut count = 0;
        while count < size {
            match file.io().read(&mut data[count..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => count += n,
            }
        }
        data.truncate(count);
        Some((data, count < size))
    }

    /// C_READ: fd, size. Response: status, then the data (the size asked, whatever was read).
    fn read(&mut self, params: &[u8]) -> Vec<u8> {
        let size = usize::from(word(params, 1));
        match self.read_bytes(byte(params, 0), size) {
            Some((mut data, _)) => {
                data.resize(size, 0);
                let mut payload = vec![0];
                payload.extend(data);
                payload
            }
            None => vec![FR_DISK_ERR],
        }
    }

    /// C_READ2: fd, size. Response: 20 at the end of the file, the count read, 2 bytes, the data.
    fn read2(&mut self, params: &[u8]) -> Vec<u8> {
        let size = usize::from(word(params, 1));
        let (data, end) = self
            .read_bytes(byte(params, 0), size)
            .unwrap_or((Vec::new(), true));
        let mut payload = vec![if end || data.is_empty() {
            EOF_STATUS
        } else {
            0
        }];
        payload.extend_from_slice(&(data.len() as u16).to_le_bytes());
        payload.extend_from_slice(&[0, 0]);
        payload.extend(data);
        payload
    }

    /// C_WRITE: fd, the data.
    fn write(&mut self, params: &[u8]) -> Vec<u8> {
        let data = params.get(1..).unwrap_or(&[]);
        let written = self
            .files
            .get_mut(&byte(params, 0))
            .is_some_and(|file| file.io().write_all(data).is_ok());
        vec![if written { 0 } else { FR_DISK_ERR }]
    }

    /// C_SEEK: fd, a 32-bit offset.
    fn seek(&mut self, params: &[u8]) -> Vec<u8> {
        let offset = u32::from_le_bytes([
            byte(params, 1),
            byte(params, 2),
            byte(params, 3),
            byte(params, 4),
        ]);
        let sought = self
            .files
            .get_mut(&byte(params, 0))
            .is_some_and(|file| file.io().seek(SeekFrom::Start(offset.into())).is_ok());
        vec![if sought { 0 } else { FR_DISK_ERR }]
    }

    fn eof(&mut self, params: &[u8]) -> Vec<u8> {
        self.with_file(params, |file| {
            let position = file.io().stream_position().unwrap_or(0);
            vec![u8::from(position >= file.size())]
        })
    }

    /// C_FSTAT: 0, then FatFs's FILINFO (size, date, time, attributes, 8.3 name), the long
    /// name at +23.
    fn fstat(&self, name: &str) -> Vec<u8> {
        let Ok(path) = self.host_path(name) else {
            return self.status(Err("Bad name.".into()));
        };
        let Some(metadata) = self.card.meta(&path) else {
            return self.status(Err("File not found.".into()));
        };
        let long_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut payload = vec![0];
        payload.extend_from_slice(&(metadata.len as u32).to_le_bytes());
        payload.extend_from_slice(&[0; 4]);
        payload.push(if metadata.is_dir { 0x10 } else { 0x20 });
        let mut short = path
            .parent()
            .map(|dir| short_name_in(&self.card, dir, &long_name))
            .unwrap_or_default()
            .into_bytes();
        short.resize(13, 0);
        payload.extend(short);
        payload.extend(nul_terminated(&long_name));
        payload
    }

    /// C_SNAFLAT: the snapshot open on fd becomes its memory, flat (a compressed version 3
    /// snapshot); its size in KB.
    fn snapshot_flat(&mut self, params: &[u8]) -> Vec<u8> {
        let fd = byte(params, 0);
        let Some(file) = self.files.get_mut(&fd) else {
            return vec![64];
        };
        let mut bytes = Vec::new();
        let _ = file.io().seek(SeekFrom::Start(0));
        let _ = file.io().read_to_end(&mut bytes);
        match cpclib::sna::Snapshot::from_buffer(bytes).and_then(|snapshot| snapshot.memory_dump())
        {
            Ok(memory) => {
                let kb = (memory.len() / 1024).min(255) as u8;
                self.files.insert(fd, OpenFile::Memory(Cursor::new(memory)));
                vec![kb]
            }
            Err(_) => vec![64],
        }
    }

    /// The entries of the current directory, by name, those starting with a dot left out; in a
    /// DSK image, its AMSDOS files (not the system ones).
    fn directory(&self) -> Vec<DirEntry> {
        let Ok(path) = self.host_path(".") else {
            return Vec::new();
        };
        if let Some((_, catalog)) = dsk_files(&self.card, &path) {
            let mut entries: Vec<DirEntry> = (0..catalog.len())
                .map(|index| &catalog[index])
                .filter(|entry| !entry.system())
                .map(|entry| DirEntry {
                    name: entry.file_name().ibm_filename(),
                    short: entry.file_name().filename(),
                    directory: false,
                    size: entry.size() as u64 * 1024,
                    read_only: true,
                })
                .collect();
            entries.retain(|entry| self.pattern.is_empty() || wildcard(&self.pattern, &entry.name));
            entries.sort_by_key(|entry| entry.name.to_ascii_uppercase());
            return entries;
        }
        let mut entries: Vec<DirEntry> = fat_directory(&self.card, &path)
            .into_iter()
            .filter_map(|(name, short)| {
                let metadata = self.card.meta(&path.join(&name))?;
                let name = name.to_string_lossy().into_owned();
                (!name.starts_with('.')).then(|| DirEntry {
                    name,
                    short,
                    directory: metadata.is_dir,
                    size: metadata.len,
                    read_only: metadata.read_only,
                })
            })
            .collect();
        entries.retain(|entry| self.pattern.is_empty() || wildcard(&self.pattern, &entry.name));
        entries.sort_by_key(|entry| (!entry.directory, entry.name.to_ascii_uppercase()));
        entries
    }

    /// C_READDIR: the next entry (none: the end), 8.3 (no parameter) or long (the width).
    fn read_dir(&mut self, params: &[u8]) -> Vec<u8> {
        if self.listing.is_none() {
            self.listing = Some(self.directory().into_iter());
        }
        let Some(entry) = self.listing.as_mut().and_then(Iterator::next) else {
            self.listing = None;
            return Vec::new();
        };
        if let Some(&width) = params.first() {
            // unterminated: RulezCharge reads a directory as `>name` and a file as the name,
            // a NUL, then the size in 5 characters, from the length byte
            let name = truncate(&entry.name, usize::from(width));
            if entry.directory {
                return format!(">{name}").into_bytes();
            }
            let mut payload = nul_terminated(&name);
            payload.extend(format!("{:>4}K", entry.size.div_ceil(1024).min(9999)).bytes());
            return payload;
        }
        let short = &entry.short;
        let (base, extension) = short.split_once('.').unwrap_or((short, ""));
        if entry.directory {
            let mut payload = format!(">{short}").into_bytes();
            payload.resize(20, 0);
            return payload;
        }
        let kb = entry.size.div_ceil(1024).min(999) as u16;
        let mut payload = format!("{base:<8}.{extension:<3} {kb:>3}K").into_bytes();
        if entry.read_only {
            payload[9] |= 0x80;
        }
        payload.push(0);
        payload.extend_from_slice(&kb.to_le_bytes());
        payload
    }
}

impl ExpansionDevice for M4Board {
    fn tick(&mut self, port: &mut ExpansionPort) {
        if let Some((address, value)) = port.io_write_started() {
            if address & 0x2000 == 0 {
                // the upper ROM select, which the board decodes too
                self.selected_rom = value;
            }
            // &FE00 and &FC00 (A7 low: &FE80 and above are other cards', the Albireo's, the Nova's)
            match address & 0xff80 {
                0xfe00 => self.command.push(value),
                0xfc00 => {
                    let packet = std::mem::take(&mut self.command);
                    self.execute(&packet);
                }
                _ => {}
            }
        }
        #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
        {
            self.ticks += 1;
            if self.ticks >= POLL_TICKS {
                self.ticks = 0;
                self.poll_network();
            }
        }
        // its ROM, in place of the CPC's
        let address = port.address();
        if !port.get_pin(ExpansionPin::Neg_ROMEN)
            && address >= 0xc000
            && self.selected_rom == self.slot
        {
            port.assert(ExpansionPin::ROMDIS);
            port.drive_data(self.rom[usize::from(address & 0x3fff)]);
        }
    }

    fn reset(&mut self) {
        self.selected_rom = 0;
        self.command.clear();
        for fd in self.files.keys().copied().collect::<Vec<_>>() {
            self.close(fd);
        }
        self.listing = None;
        #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
        {
            self.net.reset();
            self.poll_network();
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The file type of a valid AMSDOS header (its checksum: bytes 0-66, at 67-68).
/// A DSK image on the SD card (`path` itself), or a file of one (`path` in it): the image and
/// the file's name.
fn dsk_parts(card: &SdCard, path: &Path) -> Option<(PathBuf, Option<String>)> {
    let is_image = |path: &Path| {
        card.is_file(path)
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dsk"))
    };
    if is_image(path) {
        return Some((path.to_path_buf(), None));
    }
    let parent = path.parent()?;
    is_image(parent).then(|| {
        (
            parent.to_path_buf(),
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        )
    })
}

/// A DSK image's disc and AMSDOS catalogue (its data format's, side A).
fn dsk_files(card: &SdCard, image: &Path) -> Option<(ExtendedDsk, AmsdosCatalog)> {
    read_dsk(card, image).ok()
}

/// [`dsk_files`], or why the image can't be read as a directory.
fn read_dsk(card: &SdCard, image: &Path) -> Result<(ExtendedDsk, AmsdosCatalog), String> {
    let bytes = card.read(image).map_err(|e| e.to_string())?;
    let disc = ExtendedDsk::from_buffer(&bytes).map_err(|e| format!("not a DSK image: {e}"))?;
    let catalog = AmsdosManagerNonMut::new_from_disc(&disc, Head::A)
        .catalog()
        .map_err(|e| format!("no AMSDOS catalogue: {e}"))?
        .to_amsdos_catalog();
    Ok((disc, catalog))
}

/// The file `name` of a DSK image, its AMSDOS header included, as AMSDOS finds it: the name as
/// given, the case ignored, then with .BAS, then .BIN.
fn dsk_file(card: &SdCard, image: &Path, name: &str) -> Option<Vec<u8>> {
    let (disc, catalog) = dsk_files(card, image)?;
    // its spaces dropped, as AMSDOS does (a name copied from CAT: "COLUMNS .BAS")
    let name: String = name.chars().filter(|c| *c != ' ').collect();
    let name = name.trim_end_matches('.');
    let mut candidates = vec![name.to_owned()];
    if !name.contains('.') {
        candidates.extend([format!("{name}.BAS"), format!("{name}.BIN")]);
    }
    let entry = candidates.iter().find_map(|candidate| {
        (0..catalog.len())
            .map(|index| &catalog[index])
            .find(|entry| {
                entry
                    .file_name()
                    .ibm_filename()
                    .eq_ignore_ascii_case(candidate)
            })
    })?;
    let file = AmsdosManagerNonMut::new_from_disc(&disc, Head::A)
        .get_file(*entry.file_name())
        .ok()??;
    Some(file.header_and_content().to_vec())
}

fn amsdos_header_type(header: &[u8; 128]) -> Option<u8> {
    let sum: u16 = header[..67].iter().map(|&b| u16::from(b)).sum();
    (sum == u16::from_le_bytes([header[67], header[68]])).then_some(header[18])
}

fn byte(params: &[u8], index: usize) -> u8 {
    params.get(index).copied().unwrap_or(0)
}

fn word(params: &[u8], index: usize) -> u16 {
    u16::from_le_bytes([byte(params, index), byte(params, index + 1)])
}

/// The string up to its NUL (or the end).
fn string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

/// C_OPEN's name: NUL-terminated, except from |SNA, which sends 3 bytes more and no NUL.
fn open_name(bytes: &[u8]) -> String {
    match bytes.iter().position(|&b| b == 0) {
        Some(_) => string(bytes),
        None => string(&bytes[..bytes.len().saturating_sub(3)]),
    }
}

/// Two NUL-terminated strings.
fn two_strings(bytes: &[u8]) -> (String, String) {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    (string(bytes), string(bytes.get(end + 1..).unwrap_or(&[])))
}

fn nul_terminated(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

fn truncate(text: &str, width: usize) -> String {
    text.chars().take(width.max(1)).collect()
}

/// An 8.3 name, upper case: 8 characters of the name, 3 of the extension.
/// A long name's 8.3 form as FatFs's create_name makes it (upper case; spaces and the dots but
/// the last one dropped; `+,;=[]` and what isn't ASCII as `_`; at most 8 and 3 characters), and
/// whether something was lost, which asks for a numbered tail (`~1`).
fn fat_base_name(name: &str) -> (String, String, bool) {
    let name = name.trim_end_matches([' ', '.']);
    let trimmed = name.trim_start_matches([' ', '.']);
    let mut lossy = trimmed.len() != name.len();
    let last_dot = trimmed.rfind('.');
    let (mut base, mut extension) = (String::new(), String::new());
    for (index, c) in trimmed.char_indices() {
        if c == ' ' || (c == '.' && Some(index) != last_dot) {
            lossy = true;
            continue;
        }
        if Some(index) == last_dot {
            continue;
        }
        let c = if !c.is_ascii() || "+,;=[]".contains(c) {
            lossy = true;
            '_'
        } else {
            c.to_ascii_uppercase()
        };
        let (part, length) = if last_dot.is_some_and(|dot| index > dot) {
            (&mut extension, 3)
        } else {
            (&mut base, 8)
        };
        if part.len() < length {
            part.push(c);
        } else {
            lossy = true;
        }
    }
    (base, extension, lossy)
}

/// FatFs's gen_numname: `base` with the tail `~` and `seq` in hexadecimal, shortened to fit 8
/// characters; above 5, the tail is a CRC of the long name instead.
fn fat_numbered(base: &str, long: &str, seq: u32) -> String {
    let mut number = seq;
    if seq > 5 {
        let mut sreg = seq;
        for unit in long.encode_utf16() {
            let mut wc = u32::from(unit);
            for _ in 0..16 {
                sreg = (sreg << 1) + (wc & 1);
                wc >>= 1;
                if sreg & 0x10000 != 0 {
                    sreg ^= 0x11021;
                }
            }
        }
        number = sreg;
    }
    let mut tail = String::new();
    loop {
        tail.insert(
            0,
            char::from_digit(number % 16, 16)
                .unwrap()
                .to_ascii_uppercase(),
        );
        number /= 16;
        if number == 0 || tail.len() == 7 {
            break;
        }
    }
    tail.insert(0, '~');
    let keep = 8 - tail.len();
    format!("{}{tail}", base.chars().take(keep).collect::<String>())
}

/// The 8.3 names of a directory's entries (`names`, in the same order), as a FAT card holds
/// them: a name that fits keeps it, upper case; the others get `~1`, `~2`... the first free,
/// in the names' order (a card's follows the files' creation, which the host doesn't keep).
fn fat_short_names(names: &[String]) -> Vec<String> {
    let parts: Vec<(String, String, bool)> = names.iter().map(|name| fat_base_name(name)).collect();
    let join = |base: &str, extension: &str| {
        if extension.is_empty() {
            base.to_owned()
        } else {
            format!("{base}.{extension}")
        }
    };
    let mut taken: std::collections::HashSet<String> = parts
        .iter()
        .filter(|(_, _, lossy)| !lossy)
        .map(|(base, extension, _)| join(base, extension))
        .collect();
    let mut order: Vec<usize> = (0..names.len()).filter(|&index| parts[index].2).collect();
    order.sort_by_key(|&index| names[index].to_ascii_uppercase());
    let mut short: Vec<String> = parts
        .iter()
        .map(|(base, extension, _)| join(base, extension))
        .collect();
    for index in order {
        let (base, extension, _) = &parts[index];
        let name = (1..=0xffff)
            .map(|seq| join(&fat_numbered(base, &names[index], seq), extension))
            .find(|name| !taken.contains(name))
            .unwrap_or_else(|| join(base, extension));
        taken.insert(name.clone());
        short[index] = name;
    }
    short
}

/// The names in host directory `dir` (all of them: FAT numbers the hidden ones too), with their
/// 8.3 names.
fn fat_directory(card: &SdCard, dir: &Path) -> Vec<(std::ffi::OsString, String)> {
    let names = card.names(dir);
    let long: Vec<String> = names
        .iter()
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    names.into_iter().zip(fat_short_names(&long)).collect()
}

/// The 8.3 name of `long` in host directory `dir`, `NAME.EXT` (`NAME.` without an extension).
fn short_name_in(card: &SdCard, dir: &Path, long: &str) -> String {
    let short = fat_directory(card, dir)
        .into_iter()
        .find(|(name, _)| name.to_string_lossy() == long)
        .map(|(_, short)| short)
        .unwrap_or_else(|| fat_short_names(&[long.to_owned()]).remove(0));
    if short.contains('.') {
        short
    } else {
        format!("{short}.")
    }
}

/// `*` and `?` wildcards, ignoring case.
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
    let pattern = pattern.rsplit(['/', '\\']).next().unwrap_or(pattern);
    pattern.is_empty() || matches(pattern.as_bytes(), name.as_bytes())
}

/// The time, as the board prints it (UTC).
/// The time, as the board prints it (and UniDOS's node reads it): UTC plus `time_zone` hours.
fn now(time_zone: i64) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{SystemTime, UNIX_EPOCH};
    // std's clock panics in a browser
    #[cfg(target_arch = "wasm32")]
    use web_time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
        + time_zone * 3600;
    let (days, time) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = crate::calendar::civil_from_days(days);
    format!(
        "{:02}:{:02}:{:02} {year}-{month:02}-{day:02}",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

/// Where a test's SD card goes.
#[cfg(test)]
fn scratch_sd(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bemu-m4-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod test {
    use super::*;

    /// A command as the ROM sends it, and the response's payload.
    fn run(board: &mut M4Board, command: u16, params: &[u8]) -> Vec<u8> {
        let mut packet = vec![(params.len() + 2) as u8];
        packet.extend_from_slice(&command.to_le_bytes());
        packet.extend_from_slice(params);
        board.execute(&packet);
        let rom = board.rom();
        assert_eq!(
            u16::from_le_bytes([rom[RESPONSE + 1], rom[RESPONSE + 2]]),
            command,
            "echoed"
        );
        let length = usize::from(rom[RESPONSE]).saturating_sub(2);
        rom[RESPONSE + 3..RESPONSE + 3 + length].to_vec()
    }

    fn board(name: &str) -> (M4Board, PathBuf) {
        let sd = scratch_sd(name);
        (M4Board::new(&[0u8; 0x4000], &sd).unwrap(), sd)
    }

    fn named(name: &str, mode: u8) -> Vec<u8> {
        let mut params = vec![mode];
        params.extend(nul_terminated(name));
        params
    }

    #[test]
    fn files_are_written_and_read_back() {
        let (mut board, sd) = board("files");
        // as CAS OUT: fd 2
        assert_eq!(
            run(
                &mut board,
                C_OPEN,
                &named("hello.txt", FA_WRITE | FA_CREATE_ALWAYS)
            ),
            vec![2, 0]
        );
        assert_eq!(run(&mut board, C_WRITE, &[2, b'H', b'i', b'!']), vec![0]);
        assert_eq!(run(&mut board, C_CLOSE, &[2]), vec![0]);
        assert_eq!(std::fs::read(sd.join("hello.txt")).unwrap(), b"Hi!");
        assert_eq!(run(&mut board, C_CLOSE, &[2]), vec![0xff], "not open");

        // as CAS IN, the name in another case: fd 1
        assert_eq!(
            run(&mut board, C_OPEN, &named("HELLO.TXT", 0x01)),
            vec![1, 0]
        );
        assert_eq!(run(&mut board, C_FSIZE, &[1])[..2], [3, 0]);
        let read = run(&mut board, C_READ, &[1, 2, 0]);
        assert_eq!(read, vec![0, b'H', b'i']);
        assert_eq!(run(&mut board, C_FTELL, &[1])[..2], [2, 0]);
        let read2 = run(&mut board, C_READ2, &[1, 0, 8]);
        assert_eq!((read2[0], read2[1], read2[5]), (EOF_STATUS, 1, b'!'));
        assert_eq!(run(&mut board, C_SEEK, &[1, 0, 0, 0, 0]), vec![0]);
        assert_eq!(run(&mut board, C_READ, &[1, 1, 0]), vec![0, b'H']);

        // dynamic descriptors, and AMSDOS's extensions
        std::fs::write(sd.join("GAME.BAS"), b"10 PRINT").unwrap();
        assert_eq!(
            run(&mut board, C_OPEN, &named("game", 0x01 | FD_DYNAMIC)),
            vec![3, 0]
        );
        assert_eq!(
            run(&mut board, C_OPEN, &named("missing", 0x01))[1],
            FR_NO_FILE
        );
    }

    #[test]
    fn directories_are_listed_and_entered() {
        let (mut board, sd) = board("dirs");
        std::fs::write(sd.join("demo.bin"), vec![0; 2000]).unwrap();
        std::fs::write(sd.join(".hidden"), b"").unwrap();
        assert_eq!(
            run(&mut board, C_MAKEDIR, &nul_terminated("games")),
            vec![0]
        );

        run(&mut board, C_DIRSETARGS, &[0]);
        let first = run(&mut board, C_READDIR, &[]);
        assert_eq!(&first[..6], b">GAMES", "directories first");
        let second = run(&mut board, C_READDIR, &[]);
        assert_eq!(&second[..17], b"DEMO    .BIN   2K");
        assert_eq!((second[17], second[18]), (0, 2));
        assert!(run(&mut board, C_READDIR, &[]).is_empty(), "the end");
        // the long form (RulezCharge's): `>name`, or the name, a NUL and the size in 5
        // characters, unterminated
        run(&mut board, C_DIRSETARGS, &[0]);
        assert_eq!(run(&mut board, C_READDIR, &[30]), b">games");
        run(&mut board, C_DIRSETARGS, &nul_terminated("*.bin"));
        assert_eq!(run(&mut board, C_READDIR, &[30]), b"demo.bin\x00   2K");

        assert_eq!(run(&mut board, C_CD, &nul_terminated("GAMES")), vec![0]);
        assert_eq!(run(&mut board, C_GETPATH, &[]), nul_terminated("/games"));
        assert_eq!(board.rom()[RESPONSE], 2 + 7, "+0: 2 + the path and its NUL");
        assert_eq!(
            run(&mut board, C_CD, &nul_terminated("nowhere")),
            vec![0xff]
        );
        assert_eq!(run(&mut board, C_CD, &nul_terminated("../../..")), vec![0]);
        assert_eq!(board.path(), "/", "not above the card");

        assert_eq!(
            run(
                &mut board,
                C_RENAME,
                &[nul_terminated("demo.bin"), nul_terminated("other.bin")].concat()
            ),
            vec![0]
        );
        assert!(sd.join("other.bin").is_file());
        assert_eq!(
            run(&mut board, C_ERASEFILE, &nul_terminated("other.bin")),
            vec![0]
        );
        assert!(!sd.join("other.bin").exists());
        assert_eq!(
            run(&mut board, C_ERASEFILE, &nul_terminated("other.bin"))[0],
            0xff
        );
    }

    /// A data disc holding one BASIC file, COLUMNS.BAS (3K with its header), as written to
    /// `path`.
    fn columns_disc(path: &std::path::Path) {
        use cpclib_disc::amsdos::{
            AmsdosAddBehavior, AmsdosFile, AmsdosFileName, AmsdosManagerMut,
        };
        let mut disc = cpclib_disc::builder::single_head_data_dsk();
        let name = AmsdosFileName::try_from("COLUMNS.BAS").unwrap();
        let file = AmsdosFile::basic_file_from_buffer(&name, &[0x55; 2800]).unwrap();
        AmsdosManagerMut::new_from_disc(&mut disc, Head::A)
            .add_file(&file, None, false, false, AmsdosAddBehavior::FailIfPresent)
            .unwrap();
        let mut bytes = Vec::new();
        disc.to_buffer(&mut bytes);
        std::fs::write(path, bytes).unwrap();
    }

    /// A DSK image is entered as a directory (RulezCharge's "Reset + Open .DSK"), its AMSDOS
    /// files listed and read, read only, and left with `..`.
    #[test]
    fn a_dsk_image_is_a_directory() {
        let (mut board, sd) = board("dsk");
        columns_disc(&sd.join("Columns.dsk"));
        assert_eq!(
            run(&mut board, C_CD, &nul_terminated("columns.DSK")),
            vec![0]
        );
        assert_eq!(run(&mut board, C_CD, &nul_terminated("..")), vec![0]);
        // by its 8.3 name too (CAT shows it so)
        std::fs::rename(
            sd.join("Columns.dsk"),
            sd.join("Columns_CPC_v2004.1.1__ENGLISH.dsk"),
        )
        .unwrap();
        run(&mut board, C_DIRSETARGS, &[0]);
        assert_eq!(&run(&mut board, C_READDIR, &[])[..12], b"COLUMN~1.DSK");
        assert_eq!(
            run(&mut board, C_CD, &nul_terminated("column~1.dsk")),
            vec![0]
        );
        assert_eq!(
            run(&mut board, C_CD, &nul_terminated("/COLUMN~1 .DSK")),
            vec![0],
            "as CAT pads it"
        );
        std::fs::rename(
            sd.join("Columns_CPC_v2004.1.1__ENGLISH.dsk"),
            sd.join("Columns.dsk"),
        )
        .unwrap();
        assert_eq!(
            run(&mut board, C_CD, &nul_terminated("/columns.DSK")),
            vec![0]
        );
        assert_eq!(
            run(&mut board, C_GETPATH, &[]),
            nul_terminated("/Columns.dsk")
        );

        run(&mut board, C_DIRSETARGS, &[0]);
        let mut entry = run(&mut board, C_READDIR, &[]);
        assert!(entry[9] & 0x80 != 0, "read only: the extension's bit 7");
        entry[9] &= 0x7f;
        assert_eq!(&entry[..17], b"COLUMNS .BAS   3K");
        assert!(run(&mut board, C_READDIR, &[]).is_empty(), "one file");

        // RUN"COLUMNS: the name without extension, then .BAS; its AMSDOS header first
        assert_eq!(run(&mut board, C_OPEN, &named("columns", 0x01))[1], 0);
        // as CAT shows it, its spaces dropped
        assert_eq!(run(&mut board, C_OPEN, &named("COLUMNS .BAS", 0x01))[1], 0);
        let data = run(&mut board, C_READ, &[1, 128, 0]);
        assert_eq!(&data[2..9], b"COLUMNS", "the header's name");
        assert_eq!(
            run(&mut board, C_OPEN, &named("nothere.bas", 0x01))[1],
            FR_NO_FILE
        );
        assert_eq!(
            run(
                &mut board,
                C_OPEN,
                &named("columns.bas", FA_WRITE | FA_CREATE_ALWAYS)
            )[1],
            FR_DENIED
        );

        assert_eq!(run(&mut board, C_CD, &nul_terminated("..")), vec![0]);
        assert_eq!(board.path(), "/");
        // a file that isn't a disc image is no directory
        std::fs::write(sd.join("bad.dsk"), b"not a disc").unwrap();
        assert_eq!(
            run(&mut board, C_CD, &nul_terminated("bad.dsk")),
            vec![0xff]
        );
    }

    #[test]
    fn the_rom_is_written_by_the_board() {
        let (mut board, _) = board("rom");
        run(&mut board, C_CONFIG, &[5, 2]);
        assert_eq!(board.rom()[CONFIG + 5], 2);
        // C_ROMWRITE to &FFF0 (the raw senders' length byte is 0)
        board.execute(&[0, 0xfd, 0x43, 0xf0, 0xff, 2, 0, 2, 0x37, 0xc9]);
        assert_eq!(&board.rom()[0x3ff0..0x3ff2], &[0x37, 0xc9]);
        // to the NMI ROM: not emulated, nothing written
        board.execute(&[0, 0xfd, 0x43, 0x01, 0x00, 1, 0, 0xff, 0x55]);
        assert_ne!(board.rom()[1], 0x55);
        assert_eq!(
            run(&mut board, 0x4327, &[])[..3],
            *b"Not",
            "UPGRADE: not emulated"
        );
    }

    #[test]
    fn a_name_without_extension_gets_amsdos_one() {
        let (mut board, sd) = board("extension");
        run(
            &mut board,
            C_OPEN,
            &named("prog", FA_WRITE | FA_CREATE_ALWAYS),
        );
        run(&mut board, C_WRITE, &[2, 1, 2, 3]);
        run(&mut board, C_CLOSE, &[2]);
        assert!(sd.join("prog.bas").is_file(), "no header: .BAS");
        // a binary's header: type 2, checksum of bytes 0-66
        let mut header = [0u8; 128];
        header[18] = 2;
        let sum: u16 = header[..67].iter().map(|&b| u16::from(b)).sum();
        header[67..69].copy_from_slice(&sum.to_le_bytes());
        run(
            &mut board,
            C_OPEN,
            &named("CODE", FA_WRITE | FA_CREATE_ALWAYS),
        );
        let mut write = vec![2];
        write.extend_from_slice(&header);
        run(&mut board, C_WRITE, &write);
        run(&mut board, C_CLOSE, &[2]);
        assert!(sd.join("CODE.BIN").is_file());
    }

    #[test]
    fn names() {
        let short = |names: &[&str]| {
            fat_short_names(
                &names
                    .iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<_>>(),
            )
        };
        // the M4's documentation (|LONGNAME): "ROMCON~1.BIN" is romconfig.bin
        assert_eq!(short(&["romconfig.bin"]), ["ROMCON~1.BIN"]);
        assert_eq!(
            short(&[
                "Columns_CPC_v2004.1.1__ENGLISH.dsk",
                "Blight-Benediction.dsk"
            ]),
            ["COLUMN~1.DSK", "BLIGHT~1.DSK"]
        );
        // 8.3 names keep theirs, upper case; dots but the last, spaces, +,;=[] are lost
        assert_eq!(
            short(&[
                "readme",
                "Game.bas",
                "hello.world.txt",
                "a b.txt",
                "x+y.bin"
            ]),
            [
                "README",
                "GAME.BAS",
                "HELLOW~1.TXT",
                "AB~1.TXT",
                "X_Y~1.BIN"
            ]
        );
        // collisions take the next number, in the names' order; past 5 a hash of the long name
        let many: Vec<String> = (1..=7).map(|n| format!("longfilename{n}.txt")).collect();
        let names = fat_short_names(&many);
        assert_eq!(
            &names[..5],
            [
                "LONGFI~1.TXT",
                "LONGFI~2.TXT",
                "LONGFI~3.TXT",
                "LONGFI~4.TXT",
                "LONGFI~5.TXT"
            ]
        );
        assert!(
            names[5].starts_with("LO") && names[5].contains('~') && names[5].len() == 12,
            "{}",
            names[5]
        );
        assert_ne!(names[5], names[6]);
        assert!(wildcard("*.BAS", "game.bas"));
        assert!(!wildcard("*.BAS", "game.bin"));
        assert!(wildcard("g?me*", "GAME.BIN"));
        assert_eq!(open_name(b"GAME.SNA\0xx"), "GAME.SNA");
        assert_eq!(open_name(b"GAME.SNAxyz"), "GAME.SNA");
    }

    /// The board over a memory card: what the CPC writes is on the card, what the page puts on the
    /// card the CPC reads, the names matched ignoring case, folders made, listed and entered.
    #[test]
    fn a_memory_card_is_the_sd_card() {
        let card = MemoryCard::new();
        let mut board = M4Board::with_memory_card(&[0u8; 0x4000], card.clone()).unwrap();

        // the CPC writes, the page reads
        assert_eq!(
            run(&mut board, C_OPEN, &named("hello.txt", FA_WRITE | FA_CREATE_ALWAYS)),
            vec![2, 0]
        );
        assert_eq!(run(&mut board, C_WRITE, &[2, b'H', b'i', b'!']), vec![0]);
        assert_eq!(run(&mut board, C_CLOSE, &[2]), vec![0]);
        assert_eq!(card.read_file("hello.txt").unwrap(), b"Hi!");
        assert_eq!(card.read_file("/hello.txt").unwrap(), b"Hi!");

        // the page writes, the CPC reads, the name in another case
        card.write_file("games/DEMO.BIN", vec![7; 2000]).unwrap();
        assert_eq!(run(&mut board, C_OPEN, &named("HELLO.TXT", 0x01)), vec![1, 0]);
        assert_eq!(run(&mut board, C_FSIZE, &[1])[..2], [3, 0]);
        assert_eq!(run(&mut board, C_READ, &[1, 2, 0]), vec![0, b'H', b'i']);
        assert_eq!(run(&mut board, C_CLOSE, &[1]), vec![0]);
        assert_eq!(
            run(&mut board, C_OPEN, &named("Games/demo.bin", 0x01)),
            vec![1, 0],
            "a file in a folder"
        );
        assert_eq!(run(&mut board, C_READ, &[1, 2, 0]), vec![0, 7, 7]);
        run(&mut board, C_CLOSE, &[1]);
        assert_eq!(
            run(&mut board, C_OPEN, &named("missing.bin", 0x01)),
            vec![1, FR_NO_FILE]
        );

        // listed (folders first), entered, made, renamed, copied, erased
        assert_eq!(run(&mut board, C_MAKEDIR, &nul_terminated("saves")), vec![0]);
        run(&mut board, C_DIRSETARGS, &[0]);
        assert_eq!(&run(&mut board, C_READDIR, &[])[..6], b">GAMES");
        assert_eq!(&run(&mut board, C_READDIR, &[])[..6], b">SAVES");
        assert_eq!(&run(&mut board, C_READDIR, &[])[..17], b"HELLO   .TXT   1K");
        assert!(run(&mut board, C_READDIR, &[]).is_empty());
        assert_eq!(run(&mut board, C_CD, &nul_terminated("GAMES")), vec![0]);
        assert_eq!(board.path(), "/games");
        assert_eq!(run(&mut board, C_CD, &nul_terminated("nowhere")), vec![0xff]);
        assert_eq!(
            run(&mut board, C_RENAME, &[nul_terminated("demo.bin"), nul_terminated("other.bin")].concat()),
            vec![0]
        );
        assert_eq!(
            run(&mut board, C_COPYFILE, &[nul_terminated("other.bin"), nul_terminated("../saves/copy.bin")].concat()),
            vec![0]
        );
        assert_eq!(card.read_file("saves/copy.bin").unwrap(), vec![7; 2000]);
        assert_eq!(run(&mut board, C_ERASEFILE, &nul_terminated("other.bin")), vec![0]);
        assert_eq!(card.read_file("games/other.bin"), None);
        assert_eq!(run(&mut board, C_ERASEFILE, &nul_terminated("other.bin")).first(), Some(&0xff));
        assert_eq!(run(&mut board, C_CD, &nul_terminated("../../..")), vec![0]);
        assert_eq!(board.path(), "/", "not above the card");
    }

    /// A name without extension is given AMSDOS's on a memory card too, and a file can be appended to.
    #[test]
    fn a_memory_card_names_and_appends() {
        let card = MemoryCard::new();
        let mut board = M4Board::with_memory_card(&[0u8; 0x4000], card.clone()).unwrap();
        run(&mut board, C_OPEN, &named("prog", FA_WRITE | FA_CREATE_ALWAYS));
        run(&mut board, C_WRITE, &[2, 1, 2, 3]);
        run(&mut board, C_CLOSE, &[2]);
        assert_eq!(card.read_file("prog.bas").unwrap(), vec![1, 2, 3]);
        assert_eq!(card.read_file("prog"), None);

        run(&mut board, C_OPEN, &named("log.txt", FA_WRITE | FA_CREATE_ALWAYS));
        run(&mut board, C_WRITE, &[2, b'a']);
        run(&mut board, C_CLOSE, &[2]);
        run(&mut board, C_OPEN, &named("log.txt", FA_WRITE | FA_OPEN_APPEND));
        run(&mut board, C_WRITE, &[2, b'b']);
        run(&mut board, C_CLOSE, &[2]);
        assert_eq!(card.read_file("log.txt").unwrap(), b"ab");
        assert_eq!(
            run(&mut board, C_OPEN, &named("log.txt", FA_WRITE | FA_CREATE_NEW)),
            vec![2, FR_EXIST]
        );
    }

    /// The page's side of the card: files in folders, listed, taken away.
    #[test]
    fn a_memory_card_is_kept_by_the_page() {
        let card = MemoryCard::new();
        card.write_file("a/b/c.txt", b"x".to_vec()).unwrap();
        card.write_file("a/d.txt", b"yy".to_vec()).unwrap();
        card.make_dir("empty").unwrap();
        assert_eq!(
            card.list("").unwrap(),
            vec![("a".to_owned(), true, 0), ("empty".to_owned(), true, 0)]
        );
        assert_eq!(
            card.list("a").unwrap(),
            vec![("b".to_owned(), true, 0), ("d.txt".to_owned(), false, 2)]
        );
        assert_eq!(card.list("a/d.txt"), None, "not a folder");
        assert!(card.write_file("a/d.txt/e", vec![]).is_err(), "under a file");
        assert!(card.remove("a/b"));
        assert_eq!(card.read_file("a/b/c.txt"), None);
        assert!(!card.remove("a/b"));
        assert_eq!(card.read_file("a/d.txt").unwrap(), b"yy");
    }
}
