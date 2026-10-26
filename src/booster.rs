//! The CPC Booster+ (Antitec / Dirty Minds) and its variant the MiniBooster: an ATmega16 behind the
//! expansion port that answers the ports &FF00-&FF27 (A15-A8 high, A7-A0 the register).
//!
//! The registers are the ones of the board's BIOS (`booster_main.asm`, version 1.2, February 2005),
//! whose jump tables for the IN and the OUT are followed here one by one: a serial port (a UART:
//! baud rate, flags, frame format, a 255 bytes receive buffer, a time out), two PWM channels (a
//! stereo DAC), a 2 channels 8 bits ADC, 512 bytes of EEPROM, the 5 bits TTL port, a multiplier, a
//! keyboard decoder, the program memory (16K of flash, read in two ways, written 128 bytes at a
//! time: how the BIOS updates itself) and the text of the version.
//!
//! What the board has on its wires is plugged on the host:
//! - the UART is a [`SerialLink`]: a TCP socket ([`TcpLink`]), or its own TX looped to its RX
//!   ([`Loopback`]); the frames last what their baud rate says, so the flags (RXC, TXC, UDRE, DOR)
//!   behave;
//! - the ADC reads a sample file ([`AdcSource`]), looped, at the rate of its conversions;
//! - the PWM channels are the sound of the card (the first one on the left: the documentation
//!   does not say which is which);
//! - the EEPROM and the flash are kept in files when given some;
//! - the 5 TTL pins are driven from the host with [`Booster::drive_pins`].
//!
//! The MiniBooster differs by what `&FF01` reads (238 instead of 85, how software tells them apart;
//! its faster CPU is not modelled: a register access takes no time here) and by the connectors,
//! which are not emulated.
//!
//! Not modelled: the time the board makes the CPC wait (the ADC conversion, the time out of &FF09
//! returns at once with 0 when nothing was received), the 485 pin.

use std::any::Any;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::expansion_port::{ExpansionDevice, ExpansionPort};

/// The AVR's crystal.
pub const CLOCK_HZ: u64 = 11_059_200;
/// The emulated machine's clock: one tick of the expansion port.
const TICK_HZ: u64 = 16_000_000;
/// The registers: &FF00-&FF27.
const REGISTERS: u16 = 0x28;

const FLASH_SIZE: usize = 16 * 1024;
const PAGE_SIZE: usize = 128;
const EEPROM_SIZE: usize = 512;
/// Where the BIOS keeps the version text in its flash (word address &80).
const VERSION_ADDRESS: usize = 0x100;
const UART_BUFFER_SIZE: usize = 255;

/// UART flags, as `&FF06` gives them.
const RXC: u8 = 0x80;
const TXC: u8 = 0x40;
const UDRE: u8 = 0x20;
const DOR: u8 = 0x08;

/// Which board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// The CPC Booster+: `&FF01` reads 85.
    BoosterPlus,
    /// The MiniBooster: `&FF01` reads 238.
    MiniBooster
}

impl Model {
    fn second_test_byte(self) -> u8 {
        match self {
            Model::BoosterPlus => 0b0101_0101,
            Model::MiniBooster => 238
        }
    }

    fn version_text(self) -> &'static [u8] {
        match self {
            Model::BoosterPlus => {
                b"CPC BOOSTER+\r\nDEVELOPED BY\r\nAGRAFIOTIS PETROS \r\nVERSION 1.2 \r\nFebruary, 2, 2005 \r\n\r\nantitec@hotmail.com\0"
            },
            Model::MiniBooster => {
                b"CPC MINIBOOSTER\r\nDEVELOPED BY\r\nAGRAFIOTIS PETROS \r\nVERSION 1.2 \r\nFebruary, 2, 2005 \r\n\r\nantitec@hotmail.com\0"
            }
        }
    }
}

/// What the serial port is wired to.
pub trait SerialLink: Send {
    /// A byte leaves the UART.
    fn send(&mut self, byte: u8);
    /// A byte arrives at the UART, if there is one.
    fn receive(&mut self) -> Option<u8>;
    /// What is sent comes back: a frame is received only once it has been sent.
    fn loops_back(&self) -> bool {
        false
    }
}

/// The TX wired to the RX.
#[derive(Default)]
pub struct Loopback(VecDeque<u8>);

impl SerialLink for Loopback {
    fn send(&mut self, byte: u8) {
        self.0.push_back(byte);
    }

    fn receive(&mut self) -> Option<u8> {
        self.0.pop_front()
    }

    fn loops_back(&self) -> bool {
        true
    }
}

/// Nothing connected: what is sent is lost, nothing comes.
#[derive(Default)]
pub struct Unplugged;

impl SerialLink for Unplugged {
    fn send(&mut self, _byte: u8) {}

    fn receive(&mut self) -> Option<u8> {
        None
    }
}

/// A TCP socket, as a null modem: `listen` waits for one client at a time (the last to connect
/// takes the place of the previous), `connect` joins a server. Threads read the socket.
pub struct TcpLink {
    writer: Arc<Mutex<Option<TcpStream>>>,
    incoming: Receiver<u8>
}

impl TcpLink {
    /// Serve `address` (`127.0.0.1:2323`, `:2323` for every interface).
    pub fn listen(address: &str) -> std::io::Result<Self> {
        let address = if address.starts_with(':') { format!("0.0.0.0{address}") } else { address.to_owned() };
        let listener = TcpListener::bind(address)?;
        let (sender, incoming) = channel();
        let writer: Arc<Mutex<Option<TcpStream>>> = Arc::default();
        let accepting = Arc::clone(&writer);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_nodelay(true);
                if let Ok(reader) = stream.try_clone() {
                    *accepting.lock().unwrap() = Some(stream);
                    Self::read_forever(reader, sender.clone());
                }
            }
        });
        Ok(Self { writer, incoming })
    }

    /// Join the server at `address`.
    pub fn connect(address: impl ToSocketAddrs) -> std::io::Result<Self> {
        let stream = TcpStream::connect(address)?;
        stream.set_nodelay(true)?;
        let (sender, incoming) = channel();
        Self::read_forever(stream.try_clone()?, sender);
        Ok(Self { writer: Arc::new(Mutex::new(Some(stream))), incoming })
    }

    fn read_forever(mut stream: TcpStream, sender: Sender<u8>) {
        std::thread::spawn(move || {
            let mut buffer = [0u8; 256];
            while let Ok(count) = stream.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                for byte in &buffer[..count] {
                    if sender.send(*byte).is_err() {
                        return;
                    }
                }
            }
        });
    }
}

impl SerialLink for TcpLink {
    fn send(&mut self, byte: u8) {
        let mut writer = self.writer.lock().unwrap();
        if let Some(stream) = writer.as_mut() {
            if stream.write_all(&[byte]).is_err() {
                *writer = None;
            }
        }
    }

    fn receive(&mut self) -> Option<u8> {
        self.incoming.try_recv().ok()
    }
}

/// Where the ADC reads its samples: 8 bits unsigned, one list per channel, looped.
#[derive(Clone, Debug)]
pub struct AdcSource {
    channels: [Vec<u8>; 2],
    /// The samples per second of the file (a WAV's rate); `None`: one sample per conversion.
    rate: Option<u32>
}

impl AdcSource {
    /// A constant level (silence is 128).
    pub fn constant(level: u8) -> Self {
        Self { channels: [vec![level], vec![level]], rate: None }
    }

    /// A raw file of unsigned bytes (the same on both channels), one per conversion.
    pub fn raw(samples: Vec<u8>) -> Self {
        let samples = if samples.is_empty() { vec![128] } else { samples };
        Self { channels: [samples.clone(), samples], rate: None }
    }

    /// A PCM WAV (8 or 16 bits, mono or stereo; mono feeds both channels).
    pub fn wav(data: &[u8]) -> Result<Self, String> {
        let le16 = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
        let le32 = |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return Err("not a WAV".to_owned());
        }
        let (mut format, mut samples): (Option<(u16, u32, u16)>, Option<&[u8]>) = (None, None);
        let mut at = 12;
        while at + 8 <= data.len() {
            let size = le32(at + 4) as usize;
            let body = &data[at + 8..data.len().min(at + 8 + size)];
            match &data[at..at + 4] {
                b"fmt " if body.len() >= 16 => {
                    let tag = u16::from_le_bytes([body[0], body[1]]);
                    if tag != 1 && tag != 0xfffe {
                        return Err(format!("WAV format {tag}: PCM only"));
                    }
                    let _ = le16;
                    format = Some((u16::from_le_bytes([body[2], body[3]]), u32::from_le_bytes([body[4], body[5], body[6], body[7]]), u16::from_le_bytes([body[14], body[15]])));
                },
                b"data" => samples = Some(body),
                _ => {}
            }
            at += 8 + size + (size & 1);
        }
        let ((channels, rate, bits), samples) = format.zip(samples).ok_or("a WAV with no format or no data")?;
        if !matches!(bits, 8 | 16) || channels == 0 {
            return Err(format!("WAV of {channels} channels of {bits} bits: 8 or 16 bits only"));
        }
        let bytes = usize::from(bits / 8);
        let frame = bytes * usize::from(channels);
        let sample = |bytes_of: &[u8]| -> u8 {
            if bytes == 1 { bytes_of[0] } else { ((i16::from_le_bytes([bytes_of[0], bytes_of[1]]) >> 8) + 128) as u8 }
        };
        let mut lists = [Vec::new(), Vec::new()];
        for frame_bytes in samples.chunks_exact(frame) {
            for (index, list) in lists.iter_mut().enumerate() {
                let channel = index.min(usize::from(channels) - 1);
                list.push(sample(&frame_bytes[channel * bytes..(channel + 1) * bytes]));
            }
        }
        if lists[0].is_empty() {
            return Err("a WAV with no sample".to_owned());
        }
        Ok(Self { channels: lists, rate: Some(rate) })
    }

    /// The file: a WAV by its content, else raw bytes.
    pub fn open(path: &Path) -> Result<Self, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if data.starts_with(b"RIFF") {
            Self::wav(&data)
        }
        else {
            Ok(Self::raw(data))
        }
    }

    /// The sample a conversion gets at `seconds`, `conversions_per_second` being the board's.
    fn sample(&self, channel: usize, seconds: f64, conversions_per_second: f64) -> u8 {
        let list = &self.channels[channel & 1];
        let rate = self.rate.map_or(conversions_per_second, f64::from);
        list[(seconds * rate) as usize % list.len()]
    }
}

/// The 5 TTL pins: the host's side of the wires.
#[derive(Clone, Copy, Debug, Default)]
struct Pins {
    /// Bits the outside world drives...
    driven: u8,
    /// ...and the level it drives them to.
    level: u8
}

/// The UART: the registers of the ATmega16's and the frames in transit.
#[derive(Debug, Default)]
struct Uart {
    ubrr: u8,
    /// UDR as the CPU reads it, valid when RXC is set.
    received: u8,
    flags: u8,
    /// UART REG2: UMSEL, UPM, USBS, UCSZ, UCPOL as the CPC wrote them.
    reg2: u8,
    /// UART REG3: 5 bits.
    reg3: u8,
    timeout: u8,
    /// When the frame being received is complete (the next one starts then).
    rx_busy_until: u64,
    /// A looped back frame is there once it has been sent.
    rx_not_before: u64,
    /// When the frame being sent is complete.
    tx_busy_until: Option<u64>,
    /// The receive buffer (REG3 bit 4).
    buffer: VecDeque<u8>
}

impl Uart {
    fn new() -> Self {
        Self { flags: UDRE, ..Self::default() }
    }

    fn buffered(&self) -> bool {
        self.reg3 & 0x10 != 0
    }

    fn double_speed(&self) -> bool {
        self.reg3 & 0x08 != 0
    }

    /// Ticks (of the machine) one frame takes: start, data, parity, stop bits.
    fn frame_ticks(&self) -> u64 {
        let data = match (self.reg2 >> 1) & 0b111 {
            0 => 5,
            1 => 6,
            2 => 7,
            3 => 8,
            _ => 9
        };
        let parity = u64::from(self.reg2 & 0x60 != 0);
        let stop = if self.reg2 & 0x10 != 0 { 2 } else { 1 };
        let bits = 1 + data + parity + stop;
        let divisor = if self.double_speed() { 8 } else { 16 };
        // baud = clock / (divisor * (ubrr + 1)): a bit lasts divisor * (ubrr + 1) clocks of the AVR
        bits * divisor * (u64::from(self.ubrr) + 1) * TICK_HZ / CLOCK_HZ
    }
}

pub struct Booster {
    model: Model,
    /// Ticks since the board is on.
    now: u64,
    link: Box<dyn SerialLink>,
    uart: Uart,
    pwm: [u8; 2],
    /// The first value of a buffered stereo pair (&FF26), waiting for the second.
    pwm_pending: Option<u8>,
    eeprom: [u8; EEPROM_SIZE],
    eeprom_address: u16,
    eeprom_file: Option<PathBuf>,
    adc: AdcSource,
    adc_frequency: u8,
    adc_channel: u8,
    key_buffer: Vec<u8>,
    key_pressed: u8,
    flash: Vec<u8>,
    flash_file: Option<PathBuf>,
    flash_dirty: bool,
    /// The page buffer of the update, and its position (&FF15).
    page_buffer: [u8; PAGE_SIZE],
    page_buffer_position: u8,
    page: [u8; 2],
    rom_address: u16,
    port_direction: u8,
    port_latch: u8,
    pins: Pins,
    multiplier: [u8; 2],
    product: u16,
    version_position: usize,
    /// What the last read of a register put on the bus (an IN drives it for its whole length).
    read_latch: u8
}

impl Booster {
    pub fn new(model: Model) -> Self {
        let mut flash = vec![0xff; FLASH_SIZE];
        let text = model.version_text();
        flash[VERSION_ADDRESS..VERSION_ADDRESS + text.len()].copy_from_slice(text);
        Self {
            model,
            now: 0,
            link: Box::new(Unplugged),
            uart: Uart::new(),
            pwm: [0; 2],
            pwm_pending: None,
            eeprom: [0xff; EEPROM_SIZE],
            eeprom_address: 0,
            eeprom_file: None,
            adc: AdcSource::constant(128),
            adc_frequency: 0,
            adc_channel: 0,
            key_buffer: Vec::new(),
            key_pressed: 0,
            flash,
            flash_file: None,
            flash_dirty: false,
            page_buffer: [0; PAGE_SIZE],
            page_buffer_position: 0,
            page: [0; 2],
            rom_address: 0,
            port_direction: 0,
            port_latch: 0,
            pins: Pins::default(),
            multiplier: [0; 2],
            product: 0,
            version_position: 0,
            read_latch: 0
        }
    }

    pub fn model(&self) -> Model {
        self.model
    }

    /// The serial port's other end.
    pub fn with_link(mut self, link: Box<dyn SerialLink>) -> Self {
        self.link = link;
        self
    }

    /// Where the ADC samples come from.
    pub fn with_adc(mut self, adc: AdcSource) -> Self {
        self.adc = adc;
        self
    }

    /// Keep the EEPROM in `file` (read now if it exists, written when the board is dropped).
    pub fn with_eeprom_file(mut self, file: PathBuf) -> Result<Self, String> {
        match std::fs::read(&file) {
            Ok(data) => {
                let length = data.len().min(EEPROM_SIZE);
                self.eeprom[..length].copy_from_slice(&data[..length]);
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("{}: {e}", file.display()))
        }
        self.eeprom_file = Some(file);
        Ok(self)
    }

    /// Keep the program memory in `file`: a dump of the real one (16K), else what updates wrote.
    pub fn with_flash_file(mut self, file: PathBuf) -> Result<Self, String> {
        match std::fs::read(&file) {
            Ok(data) => {
                let length = data.len().min(FLASH_SIZE);
                self.flash[..length].copy_from_slice(&data[..length]);
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("{}: {e}", file.display()))
        }
        self.flash_file = Some(file);
        Ok(self)
    }

    /// The outside world drives the TTL pins `mask` to `level` (bits 0-4 of both); the other
    /// pins are left alone.
    pub fn drive_pins(&mut self, mask: u8, level: u8) {
        self.pins.driven = (self.pins.driven & !mask) | (mask & 0x1f);
        self.pins.level = (self.pins.level & !mask) | (level & mask & 0x1f);
    }

    /// Release the TTL pins `mask`.
    pub fn release_pins(&mut self, mask: u8) {
        self.pins.driven &= !mask;
    }

    /// What the board drives on its TTL pins: (the pins that are outputs, their level).
    pub fn pins_output(&self) -> (u8, u8) {
        (self.port_direction, self.port_latch & self.port_direction)
    }

    /// The PWM values of the 2 channels.
    pub fn pwm(&self) -> [u8; 2] {
        self.pwm
    }

    /// The EEPROM.
    pub fn eeprom(&self) -> &[u8] {
        &self.eeprom
    }

    /// The program memory.
    pub fn flash(&self) -> &[u8] {
        &self.flash
    }

    /// The RESET of the microcontroller (an OUT to &FF00 or &FF01, the pins of the CPC do not
    /// reset it).
    pub fn reset_board(&mut self) {
        let (model, link, adc) = (self.model, std::mem::replace(&mut self.link, Box::new(Unplugged)), self.adc.clone());
        let (eeprom, eeprom_file, flash, flash_file, flash_dirty) = (
            self.eeprom,
            self.eeprom_file.take(),
            std::mem::take(&mut self.flash),
            self.flash_file.take(),
            self.flash_dirty
        );
        let (pins, now) = (self.pins, self.now);
        *self = Self::new(model);
        self.link = link;
        self.adc = adc;
        self.eeprom = eeprom;
        self.eeprom_file = eeprom_file;
        self.flash = flash;
        self.flash_file = flash_file;
        self.flash_dirty = flash_dirty;
        self.pins = pins;
        self.now = now;
    }

    fn seconds(&self) -> f64 {
        self.now as f64 / TICK_HZ as f64
    }

    /// The frames that arrive and leave: called before each register access.
    fn run_uart(&mut self) {
        let now = self.now;
        if let Some(done) = self.uart.tx_busy_until {
            if now >= done {
                self.uart.tx_busy_until = None;
                self.uart.flags |= TXC | UDRE;
            }
        }
        // a frame comes in when the previous one is over
        if now >= self.uart.rx_busy_until && now >= self.uart.rx_not_before {
            if let Some(byte) = self.link.receive() {
                self.uart.rx_busy_until = now + self.uart.frame_ticks();
                self.byte_received(byte);
            }
        }
    }

    fn byte_received(&mut self, byte: u8) {
        if self.uart.buffered() {
            if self.uart.buffer.len() == UART_BUFFER_SIZE {
                // full: the oldest is lost
                self.uart.buffer.pop_front();
            }
            self.uart.buffer.push_back(byte);
        }
        else {
            if self.uart.flags & RXC != 0 {
                // the previous one was never read
                self.uart.flags |= DOR;
            }
            self.uart.received = byte;
            self.uart.flags |= RXC;
        }
    }

    fn transmit(&mut self, byte: u8) {
        self.link.send(byte);
        self.uart.flags &= !(TXC | UDRE);
        let done = self.now + self.uart.frame_ticks();
        self.uart.tx_busy_until = Some(done);
        if self.link.loops_back() {
            self.uart.rx_not_before = done;
        }
    }

    fn conversions_per_second(&self) -> f64 {
        // the ADC's clock is the crystal's over 2, 2, 4, ... 128; a conversion takes 13 of its clocks
        let divisor = 1u32 << self.adc_frequency.max(1);
        CLOCK_HZ as f64 / f64::from(divisor) / 13.0
    }

    fn pin_levels(&self) -> u8 {
        let input = self.port_latch & !self.port_direction & 0x1f; // the pull-ups
        let outputs = self.port_latch & self.port_direction;
        let pulled = (input & !self.pins.driven) | (self.pins.level & self.pins.driven);
        (outputs | (pulled & !self.port_direction)) & 0x1f
    }

    /// What an IN of register `register` gives (and its effects).
    pub fn read_register(&mut self, register: u8) -> u8 {
        self.run_uart();
        match register {
            0x00 => 0b1010_1010,
            0x01 => self.model.second_test_byte(),
            0x02 | 0x27 => self.pwm[0],
            0x03 => self.pwm[1],
            0x04 => self.uart.ubrr,
            0x05 => {
                self.uart.flags &= !RXC;
                self.uart.received
            },
            0x06 => self.uart.flags,
            0x07 => self.uart.reg2,
            0x09 => {
                // the BIOS waits for a character, up to the time out: 255 when there is one, else 0
                self.run_uart();
                if self.uart.flags & RXC != 0 || !self.uart.buffer.is_empty() { 0xff } else { 0 }
            },
            0x0a => self.uart.timeout,
            0x0b => self.uart.reg3,
            0x0c => (self.eeprom_address >> 8) as u8,
            0x0d => self.eeprom_address as u8,
            0x0e => self.eeprom[usize::from(self.eeprom_address) % EEPROM_SIZE],
            0x0f => self.adc_frequency,
            0x10 => self.adc_channel,
            0x11 => {
                let (seconds, rate) = (self.seconds(), self.conversions_per_second());
                self.adc.sample(usize::from(self.adc_channel), seconds, rate)
            },
            0x12 => {
                let key = self.key_pressed;
                self.key_buffer.clear();
                self.key_pressed = 0;
                key
            },
            0x15 => self.page_buffer_position,
            0x16 => self.page[0],
            0x17 => self.page[1],
            0x18 => {
                // the page mode: page * 128 + position; the position goes on, the page after it
                let address = usize::from(self.page[0]) * PAGE_SIZE + usize::from(self.page[1]);
                let byte = self.flash[address % FLASH_SIZE];
                let position = (self.page[1] + 1) & 0x7f;
                if position == 0 {
                    self.page[0] = (self.page[0] + 1) & 0x7f;
                }
                self.page[1] = position;
                byte
            },
            0x19 => (self.rom_address >> 8) as u8,
            0x1a => self.rom_address as u8,
            0x1b => {
                let byte = self.flash[usize::from(self.rom_address) % FLASH_SIZE];
                self.rom_address = (self.rom_address + 1) & 0x3fff;
                byte
            },
            0x1c => self.uart.buffer.len() as u8,
            0x1d => self.uart.buffer.pop_front().unwrap_or(0),
            0x1e => self.port_direction,
            0x1f => self.port_latch,
            0x20 => self.pin_levels(),
            0x21 => self.multiplier[0],
            0x22 => self.multiplier[1],
            0x23 => (self.product >> 8) as u8,
            0x24 => self.product as u8,
            0x25 => {
                let text = self.model.version_text();
                let byte = text[self.version_position.min(text.len() - 1)];
                // the terminator is sent, then it starts again
                self.version_position = if byte == 0 { 0 } else { self.version_position + 1 };
                byte
            },
            0x26 => {
                self.pwm_pending = None;
                0
            },
            // 08, 13, 14: nothing answers
            _ => 0
        }
    }

    /// What an OUT of `value` to register `register` does.
    pub fn write_register(&mut self, register: u8, value: u8) {
        self.run_uart();
        match register {
            0x00 | 0x01 => self.reset_board(),
            0x02 => self.pwm[0] = value,
            0x03 => self.pwm[1] = value,
            0x04 => self.uart.ubrr = value,
            0x05 => self.transmit(value),
            0x06 => {
                // writing a 1 to TXC clears it
                if value & TXC != 0 {
                    self.uart.flags &= !TXC;
                }
            },
            0x07 => self.uart.reg2 = value,
            0x08 => self.transmit(value),
            0x0a => self.uart.timeout = value,
            0x0b => self.uart.reg3 = value & 0x1f,
            0x0c => self.eeprom_address = (self.eeprom_address & 0x00ff) | u16::from(value & 1) << 8,
            0x0d => self.eeprom_address = (self.eeprom_address & 0x0100) | u16::from(value),
            0x0e => self.eeprom[usize::from(self.eeprom_address) % EEPROM_SIZE] = value,
            0x0f => self.adc_frequency = value & 0b111,
            0x10 => self.adc_channel = value & 1,
            0x12 => {
                self.key_buffer.push(value);
                if self.key_buffer.len() == 10 {
                    self.key_pressed = find_key(&self.key_buffer);
                    self.key_buffer.clear();
                }
            },
            0x13 => self.write_flash_page(value & 0x7f),
            0x14 => {
                self.page_buffer[usize::from(self.page_buffer_position)] = value;
                self.page_buffer_position = (self.page_buffer_position + 1) & 0x7f;
            },
            0x15 => self.page_buffer_position = value & 0x7f,
            0x16 => self.page[0] = value & 0x7f,
            0x17 => self.page[1] = value & 0x7f,
            0x19 => self.rom_address = (self.rom_address & 0x00ff) | u16::from(value & 0x3f) << 8,
            0x1a => self.rom_address = (self.rom_address & 0x3f00) | u16::from(value),
            0x1c => self.uart.buffer.clear(),
            0x1e => self.port_direction = value & 0x1f,
            0x1f => self.port_latch = value & 0x1f,
            0x21 => self.multiplier[0] = value,
            0x22 => {
                self.multiplier[1] = value;
                self.product = u16::from(self.multiplier[0]) * u16::from(value);
            },
            0x25 => self.version_position = 0,
            0x26 => {
                match self.pwm_pending.take() {
                    None => self.pwm_pending = Some(value),
                    Some(first) => self.pwm = [first, value]
                }
            },
            0x27 => self.pwm = [value; 2],
            _ => {}
        }
    }

    /// The BIOS update: the 128 bytes buffer written in the page, the buffer cleared.
    fn write_flash_page(&mut self, page: u8) {
        let start = usize::from(page) * PAGE_SIZE;
        self.flash[start..start + PAGE_SIZE].copy_from_slice(&self.page_buffer);
        self.flash_dirty = true;
        self.page_buffer = [0; PAGE_SIZE];
        self.page_buffer_position = 0;
    }

    /// Write the EEPROM and the flash to their files.
    pub fn save(&mut self) -> std::io::Result<()> {
        if let Some(path) = &self.eeprom_file {
            std::fs::write(path, self.eeprom)?;
        }
        if let (Some(path), true) = (&self.flash_file, self.flash_dirty) {
            std::fs::write(path, &self.flash)?;
            self.flash_dirty = false;
        }
        Ok(())
    }
}

impl Drop for Booster {
    fn drop(&mut self) {
        if let Err(e) = self.save() {
            eprintln!("CPC Booster: {e}");
        }
    }
}

/// The key the 10 lines of the CPC's keyboard scan show (a line is &FF when no key of it is down,
/// else the bit of the key is 0), as the BIOS finds it: lines in order, the first key of a line from
/// bit 7 down, with 128 added when SHIFT (line 2, bit 5) is down. 0 when no key is down.
pub fn find_key(lines: &[u8]) -> u8 {
    // [line][bit 0-7]: the code of the key (0: none, 255: shift)
    const KEYS: [[u8; 8]; 10] = [
        [61, 60, 62, 57, 54, 51, 66, 58],
        [59, 68, 55, 56, 53, 49, 50, 48],
        [63, 40, 65, 43, 52, 255, 47, 70],
        [38, 37, 39, 16, 42, 41, 46, 45],
        [36, 35, 15, 9, 12, 11, 13, 44],
        [34, 33, 21, 25, 8, 10, 14, 67],
        [32, 31, 18, 20, 7, 6, 2, 22],
        [30, 29, 5, 23, 19, 4, 3, 24],
        [27, 28, 72, 17, 71, 1, 69, 26],
        [0, 0, 0, 0, 0, 0, 0, 64]
    ];
    let shift = if lines.get(2).is_some_and(|line| line & 0x20 == 0) { 128 } else { 0 };
    for (line, keys) in lines.iter().zip(KEYS) {
        if *line == 0xff {
            continue;
        }
        for bit in (0..8).rev() {
            let key = keys[bit];
            if line & (1 << bit) == 0 && key != 0 && key != 255 {
                return key | shift;
            }
        }
    }
    0
}

impl ExpansionDevice for Booster {
    fn tick(&mut self, port: &mut ExpansionPort) {
        self.now += 1;

        let register = |address: u16| (address >> 8 == 0xff && address & 0xff < REGISTERS).then_some(address as u8);
        if let Some((address, value)) = port.io_write_started() {
            if let Some(register) = register(address) {
                self.write_register(register, value);
            }
        }
        if let Some(register) = port.io_read().and_then(register) {
            // once per IN, when it begins, and held on the bus until the Z80 latches it
            if port.io_read_started() {
                self.read_latch = self.read_register(register);
            }
            port.drive_data(self.read_latch);
        }
    }

    fn sound(&self) -> [f32; 2] {
        // the PWM through its low-pass filter: a level, 128 being the middle
        self.pwm.map(|value| (f32::from(value) - 128.0) / 128.0 * 0.5)
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

    fn board() -> Booster {
        Booster::new(Model::BoosterPlus)
    }

    #[test]
    fn the_test_bytes_tell_the_boards_apart() {
        let mut booster = board();
        assert_eq!((booster.read_register(0), booster.read_register(1)), (170, 85));
        let mut mini = Booster::new(Model::MiniBooster);
        assert_eq!((mini.read_register(0), mini.read_register(1)), (170, 238));
    }

    #[test]
    fn an_out_to_the_test_bytes_resets_the_board() {
        let mut booster = board();
        booster.write_register(0x21, 7);
        booster.write_register(0x02, 99);
        booster.write_register(0x00, 0);
        assert_eq!(booster.read_register(0x21), 0);
        assert_eq!(booster.read_register(0x02), 0);
    }

    #[test]
    fn the_multiplier_gives_16_bits() {
        let mut booster = board();
        booster.write_register(0x21, 200);
        booster.write_register(0x22, 100);
        let product = u16::from(booster.read_register(0x23)) << 8 | u16::from(booster.read_register(0x24));
        assert_eq!(product, 20_000);
    }

    #[test]
    fn the_version_text_is_read_byte_by_byte_and_starts_again() {
        let mut booster = board();
        booster.write_register(0x25, 0);
        let mut text = Vec::new();
        loop {
            let byte = booster.read_register(0x25);
            if byte == 0 {
                break;
            }
            text.push(byte);
        }
        assert!(text.starts_with(b"CPC BOOSTER+\r\n"));
        assert_eq!(booster.read_register(0x25), b'C', "it starts again");
    }

    #[test]
    fn the_eeprom_has_9_bits_of_address() {
        let mut booster = board();
        booster.write_register(0x0c, 3); // anything above 1 is 1
        booster.write_register(0x0d, 0x4e);
        booster.write_register(0x0e, 42);
        assert_eq!(booster.read_register(0x0c), 1);
        assert_eq!(booster.eeprom()[0x14e], 42);
        booster.write_register(0x0c, 0);
        booster.write_register(0x0d, 5);
        assert_eq!(booster.read_register(0x0e), 0xff, "blank");
    }

    #[test]
    fn pwm_values_are_buffered_in_pairs_and_the_buffer_is_cleared_by_an_in() {
        let mut booster = board();
        booster.write_register(0x26, 10);
        assert_eq!(booster.pwm(), [0, 0], "waiting for the second");
        booster.write_register(0x26, 20);
        assert_eq!(booster.pwm(), [10, 20]);
        booster.write_register(0x26, 99);
        booster.read_register(0x26);
        booster.write_register(0x26, 1);
        booster.write_register(0x26, 2);
        assert_eq!(booster.pwm(), [1, 2], "the lone value was dropped");
        booster.write_register(0x27, 7);
        assert_eq!(booster.pwm(), [7, 7]);
    }

    #[test]
    fn the_ttl_port_pull_ups_and_outputs() {
        let mut booster = board();
        booster.write_register(0x1e, 0b11001); // pins 1, 4, 5 are outputs
        booster.write_register(0x1f, 0b10111); // 1 and 5 high, 2 and 3 pulled up
        // pin 4 is an output driven low: 0; pins 2, 3 read their pull-up; pins 1 and 5 their level
        assert_eq!(booster.read_register(0x20), 0b10111);
        booster.drive_pins(0b00100, 0); // a switch to the ground on pin 3
        assert_eq!(booster.read_register(0x20) & 0b00100, 0);
        booster.release_pins(0b00100);
        assert_eq!(booster.read_register(0x20) & 0b00100, 0b00100);
        // without the pull-up an input floats low
        booster.write_register(0x1f, 0b10001);
        assert_eq!(booster.read_register(0x20) & 0b00110, 0);
    }

    #[test]
    fn keys_are_found_in_the_scan_of_the_keyboard() {
        let mut lines = [0xffu8; 10];
        assert_eq!(find_key(&lines), 0, "no key");
        // 'a': line 8 bit 5
        lines[8] = !(1 << 5);
        assert_eq!(find_key(&lines), 1);
        // SHIFT (line 2 bit 5) down: +128
        lines[2] = !(1 << 5);
        assert_eq!(find_key(&lines), 129);
        // DELETE: line 9 bit 7
        let mut lines = [0xffu8; 10];
        lines[9] = !(1 << 7);
        assert_eq!(find_key(&lines), 64);

        // through the register: the key is there after the 10th byte only
        let mut booster = board();
        for byte in &lines[..9] {
            booster.write_register(0x12, *byte);
        }
        assert_eq!(booster.read_register(0x12), 0);
        let mut booster = board();
        for byte in &lines {
            booster.write_register(0x12, *byte);
        }
        assert_eq!(booster.read_register(0x12), 64);
        assert_eq!(booster.read_register(0x12), 0, "read once");
    }

    #[test]
    fn the_flash_is_read_in_both_modes_and_updated_by_pages() {
        let mut booster = board();
        // the version text is at &100: page 2, offset 0
        booster.write_register(0x16, 2);
        booster.write_register(0x17, 0);
        assert_eq!(booster.read_register(0x18), b'C');
        assert_eq!(booster.read_register(0x18), b'P', "the position goes on");
        booster.write_register(0x19, 0x01);
        booster.write_register(0x1a, 0x00);
        assert_eq!(booster.read_register(0x1b), b'C');
        assert_eq!(booster.read_register(0x1b), b'P');

        // fill the buffer and write page 80
        for byte in 0..128u8 {
            booster.write_register(0x14, byte);
        }
        assert_eq!(booster.read_register(0x15), 0, "the buffer wrapped");
        booster.write_register(0x13, 80);
        booster.write_register(0x16, 80);
        booster.write_register(0x17, 5);
        assert_eq!(booster.read_register(0x18), 5);
        assert_eq!(booster.flash()[80 * 128 + 127], 127);
        // the page mode goes on to the next page after the 128th byte
        booster.write_register(0x17, 127);
        booster.read_register(0x18);
        assert_eq!(booster.read_register(0x16), 81);
        assert_eq!(booster.read_register(0x17), 0);
    }

    fn baud_9600(booster: &mut Booster) {
        booster.write_register(0x04, 71); // 9600 bauds
        booster.write_register(0x07, 0b0000_0110); // 8 bits, no parity, 1 stop bit
    }

    #[test]
    fn a_byte_sent_comes_back_through_a_loopback_after_its_frame() {
        let mut booster = board().with_link(Box::<Loopback>::default());
        baud_9600(&mut booster);
        assert_eq!(booster.read_register(0x06) & (UDRE | TXC | RXC), UDRE, "ready to send");
        booster.write_register(0x05, 0x41);
        assert_eq!(booster.read_register(0x06) & (UDRE | TXC), 0, "being sent");
        // a frame of 10 bits at 9600 bauds: a little more than a millisecond
        booster.now += TICK_HZ / 9600 * 10 + 10;
        let flags = booster.read_register(0x06);
        assert_eq!(flags & (UDRE | TXC), UDRE | TXC, "sent");
        assert_eq!(flags & RXC, RXC, "and there it is");
        assert_eq!(booster.read_register(0x05), 0x41);
        assert_eq!(booster.read_register(0x06) & RXC, 0, "reading clears RXC");
        booster.write_register(0x06, TXC);
        assert_eq!(booster.read_register(0x06) & TXC, 0, "a 1 clears TXC");
    }

    #[test]
    fn an_unread_byte_is_an_overrun() {
        let mut booster = board().with_link(Box::<Loopback>::default());
        baud_9600(&mut booster);
        booster.write_register(0x08, 1);
        booster.write_register(0x08, 2);
        booster.now += TICK_HZ / 9600 * 40;
        booster.read_register(0x06);
        booster.now += TICK_HZ / 9600 * 40;
        let flags = booster.read_register(0x06);
        assert_eq!(flags & (RXC | DOR), RXC | DOR);
        assert_eq!(booster.read_register(0x05), 2, "the last one stays");
    }

    #[test]
    fn the_receive_buffer_keeps_the_bytes_until_they_are_read() {
        let mut booster = board().with_link(Box::<Loopback>::default());
        baud_9600(&mut booster);
        booster.write_register(0x0b, 0x10); // buffer on
        booster.write_register(0x1c, 0); // reset it
        for byte in [10u8, 20, 30] {
            booster.write_register(0x08, byte);
        }
        booster.now += TICK_HZ / 9600 * 100;
        // the frames come one at a time: let them all in
        for _ in 0..3 {
            booster.read_register(0x1c);
            booster.now += TICK_HZ / 9600 * 11;
        }
        assert_eq!(booster.read_register(0x1c), 3);
        assert_eq!(booster.read_register(0x09), 0xff, "a character is there");
        assert_eq!(
            [booster.read_register(0x1d), booster.read_register(0x1d), booster.read_register(0x1d)],
            [10, 20, 30]
        );
        assert_eq!(booster.read_register(0x1c), 0);
        assert_eq!(booster.read_register(0x09), 0, "nothing, the time out is not waited");
        assert_eq!(booster.read_register(0x06) & RXC, 0, "the UDR is not used");
    }

    #[test]
    fn the_baud_rate_sets_the_frame_time() {
        let mut uart = Uart::new();
        uart.ubrr = 71;
        uart.reg2 = 0b0000_0110;
        let normal = uart.frame_ticks();
        assert!(normal.abs_diff(TICK_HZ / 9600 * 10) < 100, "{normal}");
        uart.reg3 = 0x08; // U2X: twice as fast
        assert!(uart.frame_ticks().abs_diff(normal / 2) < 10);
    }

    #[test]
    fn the_adc_reads_its_source_by_channel_and_time() {
        let mut booster = board().with_adc(AdcSource {
            channels: [vec![1, 2, 3, 4], vec![10, 20, 30, 40]],
            rate: Some(1)
        });
        assert_eq!(booster.read_register(0x11), 1);
        booster.write_register(0x10, 1);
        booster.now = TICK_HZ * 2; // 2 seconds later, at 1 sample a second
        assert_eq!(booster.read_register(0x11), 30);
        assert_eq!(booster.read_register(0x10), 1);
        // the channel is 1 bit
        booster.write_register(0x10, 2);
        assert_eq!(booster.read_register(0x10), 0);
        booster.write_register(0x0f, 3);
        assert_eq!(booster.read_register(0x0f), 3);
    }

    #[test]
    fn a_wav_is_a_source() {
        // 8 bits mono, 2 samples, 8000 Hz
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&38u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        for field in [1u16, 1] {
            wav.extend_from_slice(&field.to_le_bytes());
        }
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&2u32.to_le_bytes());
        wav.extend_from_slice(&[7, 9]);
        let source = AdcSource::wav(&wav).unwrap();
        assert_eq!(source.sample(0, 0.0, 1.0), 7);
        assert_eq!(source.sample(1, 1.0 / 8000.0, 1.0), 9);
        assert!(AdcSource::wav(b"nope").is_err());
    }

    #[test]
    fn the_registers_answer_on_the_ports_of_the_expansion_port() {
        use crate::expansion_port::CpcSide;

        // an IN of &FF01 as the motherboard's cycle shows it: IORQ and RD low on its first tick
        let mut port = ExpansionPort::default();
        port.plug(Box::new(Booster::new(Model::MiniBooster)));
        let idle = CpcSide {
            address: 0xff01,
            m1: true,
            mreq: true,
            iorq: true,
            rd: true,
            wr: true,
            rfsh: true,
            halt: true,
            busak: true,
            romen: true,
            ramrd: true,
            ..CpcSide::default()
        };
        let reading = CpcSide { iorq: false, rd: false, ..idle };
        port.tick_with(idle);
        port.tick_with(reading);
        assert_eq!(port.driven_data(), Some(238));
        // another port is left alone
        port.tick_with(idle);
        port.tick_with(CpcSide { address: 0xfe81, ..reading });
        assert_eq!(port.driven_data(), None);
        // an OUT: the PWM
        port.tick_with(idle);
        port.tick_with(CpcSide { address: 0xff27, data: 200, iorq: false, wr: false, ..idle });
        let booster = port.device::<Booster>().unwrap();
        assert_eq!(booster.pwm(), [200, 200]);
        assert!(booster.sound()[0] > 0.0);
    }
}
