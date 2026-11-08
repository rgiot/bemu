//! The CPC's 50-pin expansion port, where the cards plug, and its printer port.
//!
//! A card sees the port's pins as a real one does: the CPU's buses and control lines, the gate
//! array's ROM/RAM selects, the CRTC's CURSOR, the 4 MHz clock. It drives the lines a card can
//! drive: /INT and /NMI (open collector), the data bus (when it answers a read or an interrupt
//! acknowledge), ROMDIS and RAMDIS (to take the place of the CPC's ROM or RAM), /EXP, /BUSRQ,
//! /RESET, READY, LPEN. The motherboard mirrors the CPC's side of the pins into the port every
//! tick (16 MHz), lets the cards drive theirs, and wires the result back (/INT is wired-or with
//! the gate array's, a falling /NMI is latched by the CPU, a driven data bus is what an IN or an
//! IM 2 acknowledge reads).
//!
//! The pins, as numbered on the connector:
//!
//! | pin | signal | pin | signal | pin | signal |
//! |---|---|---|---|---|---|
//! | 1 | SOUND | 28 | /MREQ | 40 | /BRST |
//! | 2 | GND | 29 | /M1 | 41 | /RESET |
//! | 3-18 | A15-A0 | 30 | /RFSH | 42 | /ROMEN |
//! | 19-26 | D7-D0 | 31 | /IORQ | 43 | ROMDIS |
//! | 27 | +5V | 32 | /RD | 44 | /RAMRD |
//! | | | 33 | /WR | 45 | RAMDIS |
//! | | | 34 | /HALT | 46 | CURSOR |
//! | | | 35 | /INT | 47 | LPEN |
//! | | | 36 | /NMI | 48 | /EXP |
//! | | | 37 | /BUSRQ | 49 | GND |
//! | | | 38 | /BUSAK | 50 | CLK4 |
//! | | | 39 | READY | | |
//!
//! SOUND (an input of the CPC's amplifier) is the cards' [`ExpansionDevice::sound`]: stereo
//! here, as the PlayCity has its own stereo output.

use std::any::Any;

use crate::chip::{Chip, PinsBuffer};
use crate::psg::Psg;

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum ExpansionPin {
    A0 = 1 << 0,
    A1 = 1 << 1,
    A2 = 1 << 2,
    A3 = 1 << 3,
    A4 = 1 << 4,
    A5 = 1 << 5,
    A6 = 1 << 6,
    A7 = 1 << 7,
    A8 = 1 << 8,
    A9 = 1 << 9,
    A10 = 1 << 10,
    A11 = 1 << 11,
    A12 = 1 << 12,
    A13 = 1 << 13,
    A14 = 1 << 14,
    A15 = 1 << 15,
    D0 = 1 << 16,
    D1 = 1 << 17,
    D2 = 1 << 18,
    D3 = 1 << 19,
    D4 = 1 << 20,
    D5 = 1 << 21,
    D6 = 1 << 22,
    D7 = 1 << 23,
    Neg_MREQ = 1 << 24,
    Neg_M1 = 1 << 25,
    Neg_RFSH = 1 << 26,
    Neg_IORQ = 1 << 27,
    Neg_RD = 1 << 28,
    Neg_WR = 1 << 29,
    Neg_HALT = 1 << 30,
    Neg_INT = 1 << 31,
    Neg_NMI = 1 << 32,
    Neg_BUSRQ = 1 << 33,
    Neg_BUSAK = 1 << 34,
    READY = 1 << 35,
    Neg_BRST = 1 << 36,
    Neg_RESET = 1 << 37,
    Neg_ROMEN = 1 << 38,
    ROMDIS = 1 << 39,
    Neg_RAMRD = 1 << 40,
    RAMDIS = 1 << 41,
    CURSOR = 1 << 42,
    LPEN = 1 << 43,
    Neg_EXP = 1 << 44,
    CLK4 = 1 << 45,
}

impl From<ExpansionPin> for u64 {
    fn from(pin: ExpansionPin) -> u64 {
        pin as u64
    }
}

const ADDRESS_MASK: u64 = 0xffff;
const DATA_SHIFT: u32 = 16;
const DATA_MASK: u64 = 0xff << DATA_SHIFT;

/// The lines a card drives that idle high (the open-collector ones, READY); ROMDIS, RAMDIS and
/// LPEN idle low.
const CARD_LINES_IDLE: u64 = ExpansionPin::Neg_INT as u64
    | ExpansionPin::Neg_NMI as u64
    | ExpansionPin::Neg_BUSRQ as u64
    | ExpansionPin::Neg_EXP as u64
    | ExpansionPin::Neg_RESET as u64
    | ExpansionPin::READY as u64;

/// A card plugged on the expansion port.
pub trait ExpansionDevice: Any {
    /// One tick (16 MHz): the card sees the port's pins (their levels of the previous tick too,
    /// for the edges) and pulls or drives the lines it drives.
    fn tick(&mut self, port: &mut ExpansionPort);

    /// The CPC's /RESET.
    fn reset(&mut self) {}

    /// The card's sound, left and right, 0 when silent; read once a microsecond.
    fn sound(&self) -> [f32; 2] {
        [0.0; 2]
    }

    /// The sound chips of the card, named, for the sound export.
    fn sound_chips(&mut self) -> Vec<(&'static str, &mut Psg)> {
        Vec::new()
    }

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// The expansion port's pins, and the cards plugged on it (daisy-chained: each sees the same
/// pins).
pub struct ExpansionPort {
    pins: PinsBuffer,
    /// A card drives the data bus this tick.
    data_driven: bool,
    devices: Vec<Box<dyn ExpansionDevice>>,
}

impl Default for ExpansionPort {
    /// No card pulls a line yet.
    fn default() -> Self {
        Self {
            pins: PinsBuffer(CARD_LINES_IDLE, CARD_LINES_IDLE),
            data_driven: false,
            devices: Vec::new(),
        }
    }
}

impl Chip for ExpansionPort {
    type PinType = ExpansionPin;

    fn must_execute_tick(&self) -> bool {
        true
    }

    fn execute_tick(&mut self) {
        let mut devices = std::mem::take(&mut self.devices);
        for device in &mut devices {
            device.tick(self);
        }
        self.devices = devices;
    }

    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }

    fn pins_buffer(&self) -> &PinsBuffer {
        &self.pins
    }

    fn reset(&mut self) {
        for device in &mut self.devices {
            device.reset();
        }
    }
}

/// What the CPC puts on the port for one tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpcSide {
    pub address: u16,
    pub data: u8,
    pub mreq: bool,
    pub m1: bool,
    pub rfsh: bool,
    pub iorq: bool,
    pub rd: bool,
    pub wr: bool,
    pub halt: bool,
    pub busak: bool,
    pub romen: bool,
    pub ramrd: bool,
    pub cursor: bool,
    pub clk4: bool,
}

impl ExpansionPort {
    pub fn plug(&mut self, device: Box<dyn ExpansionDevice>) {
        self.devices.push(device);
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// Take the first card of type `T` off the port (the cards after it move up the chain).
    pub fn unplug<T: ExpansionDevice>(&mut self) -> Option<Box<dyn ExpansionDevice>> {
        let index = self
            .devices
            .iter()
            .position(|device| device.as_any().is::<T>())?;
        Some(self.devices.remove(index))
    }

    pub fn devices(&self) -> impl Iterator<Item = &dyn ExpansionDevice> {
        self.devices.iter().map(|device| device.as_ref())
    }

    pub fn devices_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn ExpansionDevice>> {
        self.devices.iter_mut()
    }

    /// The first card of type `T`.
    pub fn device<T: ExpansionDevice>(&self) -> Option<&T> {
        self.devices
            .iter()
            .find_map(|device| device.as_any().downcast_ref::<T>())
    }

    pub fn device_mut<T: ExpansionDevice>(&mut self) -> Option<&mut T> {
        self.devices
            .iter_mut()
            .find_map(|device| device.as_any_mut().downcast_mut::<T>())
    }

    /// One tick: the CPC's side of the pins (the CPU's levels: the `Neg_` ones are the
    /// electrical levels), the cards' lines released, then the cards run.
    pub fn tick_with(&mut self, cpc: CpcSide) {
        self.save_pins_state();
        let levels = [
            (ExpansionPin::Neg_MREQ, cpc.mreq),
            (ExpansionPin::Neg_M1, cpc.m1),
            (ExpansionPin::Neg_RFSH, cpc.rfsh),
            (ExpansionPin::Neg_IORQ, cpc.iorq),
            (ExpansionPin::Neg_RD, cpc.rd),
            (ExpansionPin::Neg_WR, cpc.wr),
            (ExpansionPin::Neg_HALT, cpc.halt),
            (ExpansionPin::Neg_BUSAK, cpc.busak),
            (ExpansionPin::Neg_ROMEN, cpc.romen),
            (ExpansionPin::Neg_RAMRD, cpc.ramrd),
            (ExpansionPin::CURSOR, cpc.cursor),
            (ExpansionPin::CLK4, cpc.clk4),
        ];
        let mut pins = u64::from(cpc.address) | u64::from(cpc.data) << DATA_SHIFT;
        for (pin, level) in levels {
            if level {
                pins |= pin as u64;
            }
        }
        self.pins.1 = pins | CARD_LINES_IDLE;
        self.data_driven = false;
        self.execute_tick();
    }

    /// A15-A0.
    pub fn address(&self) -> u16 {
        (self.pins.1 & ADDRESS_MASK) as u16
    }

    /// D7-D0.
    pub fn data(&self) -> u8 {
        ((self.pins.1 & DATA_MASK) >> DATA_SHIFT) as u8
    }

    /// The previous tick's D7-D0.
    pub fn previous_data(&self) -> u8 {
        ((self.pins.0 & DATA_MASK) >> DATA_SHIFT) as u8
    }

    /// A card puts `value` on the data bus (several at once: the bus fights, the zeroes win).
    pub fn drive_data(&mut self, value: u8) {
        let value = if self.data_driven {
            self.data() & value
        } else {
            value
        };
        self.pins.1 = (self.pins.1 & !DATA_MASK) | u64::from(value) << DATA_SHIFT;
        self.data_driven = true;
    }

    /// What the cards put on the data bus this tick.
    pub fn driven_data(&self) -> Option<u8> {
        self.data_driven.then(|| self.data())
    }

    /// A card pulls a line that idles high (/INT, /NMI, /EXP, READY for a wait, ...) or raises
    /// one that idles low (ROMDIS, RAMDIS, LPEN).
    pub fn assert(&mut self, pin: ExpansionPin) {
        let active_low = CARD_LINES_IDLE & pin as u64 != 0;
        self.set_pin(pin, !active_low);
    }

    fn was(&self, pin: ExpansionPin) -> bool {
        self.pins.0 & pin as u64 != 0
    }

    fn io_write_level(pins: u64) -> bool {
        let low = ExpansionPin::Neg_IORQ as u64 | ExpansionPin::Neg_WR as u64;
        pins & low == 0 && pins & ExpansionPin::Neg_M1 as u64 != 0
    }

    /// The CPU begins an OUT this tick: its port and byte.
    pub fn io_write_started(&self) -> Option<(u16, u8)> {
        (Self::io_write_level(self.pins.1) && !Self::io_write_level(self.pins.0))
            .then(|| (self.address(), self.data()))
    }

    /// The CPU reads a port (an IN): its address.
    pub fn io_read(&self) -> Option<u16> {
        let low = ExpansionPin::Neg_IORQ as u64 | ExpansionPin::Neg_RD as u64;
        (self.pins.1 & low == 0 && self.get_pin(ExpansionPin::Neg_M1)).then(|| self.address())
    }

    /// The CPU begins a memory write this tick: the address and the byte.
    pub fn memory_write_started(&self) -> Option<(u16, u8)> {
        let low = ExpansionPin::Neg_MREQ as u64 | ExpansionPin::Neg_WR as u64;
        let writing = |pins: u64| pins & low == 0;
        (writing(self.pins.1) && !writing(self.pins.0)).then(|| (self.address(), self.data()))
    }

    /// A memory read (an opcode fetch included): its address.
    pub fn memory_read(&self) -> Option<u16> {
        let low = ExpansionPin::Neg_MREQ as u64 | ExpansionPin::Neg_RD as u64;
        (self.pins.1 & low == 0).then(|| self.address())
    }

    /// The CPU begins an IN this tick (a device answering it reads its register once).
    pub fn io_read_started(&self) -> bool {
        let low = ExpansionPin::Neg_IORQ as u64 | ExpansionPin::Neg_RD as u64;
        let reading = |pins: u64| pins & low == 0 && pins & ExpansionPin::Neg_M1 as u64 != 0;
        reading(self.pins.1) && !reading(self.pins.0)
    }

    /// An interrupt acknowledge (/M1 and /IORQ low).
    pub fn interrupt_acknowledge(&self) -> bool {
        !self.get_pin(ExpansionPin::Neg_M1) && !self.get_pin(ExpansionPin::Neg_IORQ)
    }

    /// /M1 rises this tick.
    pub fn m1_ends(&self) -> bool {
        self.get_pin(ExpansionPin::Neg_M1) && !self.was(ExpansionPin::Neg_M1)
    }
}

/// What a printer-port device sees: the CPC's latch (&EFxx, A12 low) puts bits 0-6 on D0-D6 and
/// bit 7, inverted, on /STROBE.
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum PrinterPin {
    D0 = 1 << 0,
    D1 = 1 << 1,
    D2 = 1 << 2,
    D3 = 1 << 3,
    D4 = 1 << 4,
    D5 = 1 << 5,
    D6 = 1 << 6,
    Neg_STROBE = 1 << 7,
    /// To the PPI's port B bit 6.
    BUSY = 1 << 8,
}

impl From<PrinterPin> for u64 {
    fn from(pin: PrinterPin) -> u64 {
        pin as u64
    }
}

/// A device on the printer port.
pub trait PrinterDevice: Any {
    /// The port's pins changed.
    fn update(&mut self, port: &PrinterPort);

    fn sound(&self) -> [f32; 2] {
        [0.0; 2]
    }

    /// The BUSY line, when the device drives it (a printer): the PPI's port B bit 6 follows.
    /// With `None` the line is left high, as with no printer.
    fn busy(&self) -> Option<bool> {
        None
    }

    /// A microsecond has passed.
    fn advance_us(&mut self) {}

    /// Whether the device makes sound (the AY then gives it half of the mix).
    fn has_sound(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// The printer port: the CPC's 74LS273 latch.
#[derive(Default)]
pub struct PrinterPort {
    pins: PinsBuffer,
    device: Option<Box<dyn PrinterDevice>>,
}

impl PrinterPort {
    pub fn plug(&mut self, device: Box<dyn PrinterDevice>) {
        self.device = Some(device);
        self.latch(0);
    }

    /// Take the device off the port.
    pub fn unplug(&mut self) -> Option<Box<dyn PrinterDevice>> {
        self.device.take()
    }

    pub fn device_mut(&mut self) -> Option<&mut dyn PrinterDevice> {
        self.device.as_deref_mut()
    }

    /// The device, if it is a `T`.
    pub fn device_as<T: PrinterDevice>(&self) -> Option<&T> {
        self.device.as_deref()?.as_any().downcast_ref()
    }

    pub fn device_as_mut<T: PrinterDevice>(&mut self) -> Option<&mut T> {
        self.device.as_deref_mut()?.as_any_mut().downcast_mut()
    }

    pub fn device(&self) -> Option<&dyn PrinterDevice> {
        self.device.as_deref()
    }

    /// The CPU writes the latch (an OUT with A12 low).
    pub fn latch(&mut self, value: u8) {
        self.pins.0 = self.pins.1;
        // bit 7 is inverted to /STROBE
        let strobe = u64::from(value & 0x80 == 0) * PrinterPin::Neg_STROBE as u64;
        self.pins.1 = (self.pins.1 & PrinterPin::BUSY as u64) | u64::from(value & 0x7f) | strobe;
        if let Some(mut device) = self.device.take() {
            device.update(self);
            self.device = Some(device);
        }
    }

    pub fn get_pin(&self, pin: PrinterPin) -> bool {
        self.pins.1 & pin as u64 != 0
    }

    /// D0-D6.
    pub fn data(&self) -> u8 {
        (self.pins.1 & 0x7f) as u8
    }

    pub fn sound(&self) -> [f32; 2] {
        self.device
            .as_ref()
            .map_or([0.0; 2], |device| device.sound())
    }

    /// The device's BUSY line, if it drives it.
    pub fn busy(&self) -> Option<bool> {
        self.device.as_ref().and_then(|device| device.busy())
    }

    /// Whether the device makes sound.
    pub fn has_sound(&self) -> bool {
        self.device
            .as_ref()
            .is_some_and(|device| device.has_sound())
    }

    /// A microsecond passes for the device.
    pub fn advance_us(&mut self) {
        if let Some(device) = &mut self.device {
            device.advance_us();
        }
    }
}
