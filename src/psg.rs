//! AY-3-8912 Programmable Sound Generator, as wired into the CPC.
//!
//! Bus protocol cross-checked against the General Instrument AY-3-8910/
//!8912/8913 datasheet (`doc/psg.ay-3-8910.general.instrument.pdf`): on the
//! CPC, BC2 is hardwired high, which collapses the datasheet's BDIR/BC2/BC1
//! truth table to the well-known BDIR/BC1 pair: 00 inactive, 01 read, 10
//! write, 11 latch address.
//!
//! Like the FDC (see `fdc.rs`), this isn't a pin-level `Chip`: the PSG's
//! data bus is the *same* physical bus as PPI port A (the CPC never gives
//! it its own latch), and the bus-control lines are just two bits of PPI
//! port C - there is nothing here that needs sub-instruction pin timing,
//! only "what does the bus look like right now". `Motherboard` drives it
//! by calling [`Psg::bus_cycle`] with the current BDIR/BC1/data-bus state
//! once per PPI port A or C write, exactly the way it drives the FDC's
//! registers from raw port addresses.
//!
//! Register 14 (I/O port A) is what the firmware reads the keyboard matrix
//! back through: `Motherboard` is expected to keep it in sync with the
//! currently-selected [`crate::keyboard::KeyboardMatrix`] row (selected via
//! PPI port C bits 0-3) before a read reaches here - this module has no
//! notion of a keyboard itself, only of "whatever byte register 14 holds".
//!
//! Actual tone/noise/envelope sound synthesis is out of scope for now (this
//! only needs to be correct enough to serve as CSL's keyboard-read path);
//! the register array already holds everything synthesis would need later
//! without any API change.

/// BDIR/BC1 bus state, decoded from PPI port C bits 7 and 6 - see the
/// module doc comment for the truth table this collapses from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusControl {
    Inactive,
    Read,
    Write,
    LatchAddress
}

impl BusControl {
    pub fn from_bdir_bc1(bdir: bool, bc1: bool) -> Self {
        match (bdir, bc1) {
            (false, false) => BusControl::Inactive,
            (false, true) => BusControl::Read,
            (true, false) => BusControl::Write,
            (true, true) => BusControl::LatchAddress
        }
    }
}

pub const NB_REGISTERS: usize = 16;

/// I/O port A - what the keyboard matrix is read back through.
pub const REG_IO_PORT_A: u8 = 14;
/// Mixer/IO-enable register: bit 6 = port A direction (1 = input).
pub const REG_MIXER: u8 = 7;

pub struct Psg {
    registers: [u8; NB_REGISTERS],
    selected_register: Option<u8>
}

impl Default for Psg {
    fn default() -> Self {
        Self {
            registers: [0; NB_REGISTERS],
            selected_register: None
        }
    }
}

impl Psg {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn register(&self, idx: u8) -> u8 {
        self.registers[idx as usize & 0x0f]
    }

    pub fn set_register(&mut self, idx: u8, value: u8) {
        self.registers[idx as usize & 0x0f] = value;
    }

    pub fn selected_register(&self) -> Option<u8> {
        self.selected_register
    }

    /// Whether port A (register 14) is currently configured as an input
    /// (i.e. the keyboard can be read through it) - bit 6 of register 7,
    /// the mixer/IO-enable register (0 = input, per the datasheet's I/O
    /// enable convention).
    pub fn port_a_is_input(&self) -> bool {
        self.register(REG_MIXER) & (1 << 6) == 0
    }

    /// Advance the PSG's bus interface by one CPU-driven bus cycle.
    ///
    /// `data_bus` is the current value of the shared PPI-port-A/PSG data
    /// bus. On a write or latch-address cycle this is what the PSG reads;
    /// on a read cycle, the returned value is what `Motherboard` must then
    /// drive back onto that same bus (mirroring the datasheet's own
    /// tristate-until-driven read behaviour - `None` in every other mode).
    pub fn bus_cycle(&mut self, control: BusControl, data_bus: u8) -> Option<u8> {
        match control {
            BusControl::Inactive => None,
            BusControl::LatchAddress => {
                // Out-of-range addresses are simply ignored by the real
                // chip (mask to the 16 implemented registers).
                self.selected_register = Some(data_bus & 0x0f);
                None
            },
            BusControl::Write => {
                if let Some(reg) = self.selected_register {
                    self.set_register(reg, data_bus);
                }
                None
            },
            BusControl::Read => self.selected_register.map(|reg| self.register(reg))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn inactive_bus_does_nothing() {
        let mut psg = Psg::default();
        assert_eq!(psg.bus_cycle(BusControl::Inactive, 0x42), None);
        assert_eq!(psg.selected_register(), None);
    }

    #[test]
    fn latch_write_read_roundtrip() {
        let mut psg = Psg::default();
        assert_eq!(psg.bus_cycle(BusControl::LatchAddress, 6), None);
        assert_eq!(psg.selected_register(), Some(6));

        assert_eq!(psg.bus_cycle(BusControl::Write, 0x99), None);
        assert_eq!(psg.register(6), 0x99);

        assert_eq!(psg.bus_cycle(BusControl::Read, 0xAA /* ignored */), Some(0x99));
    }

    #[test]
    fn latch_address_is_masked_to_16_registers() {
        let mut psg = Psg::default();
        psg.bus_cycle(BusControl::LatchAddress, 0xFF);
        assert_eq!(psg.selected_register(), Some(0x0f));
    }

    #[test]
    fn write_without_a_latched_register_is_ignored() {
        let mut psg = Psg::default();
        psg.bus_cycle(BusControl::Write, 0x55);
        assert_eq!(psg.registers, [0; NB_REGISTERS]);
    }

    #[test]
    fn read_without_a_latched_register_returns_none() {
        let mut psg = Psg::default();
        assert_eq!(psg.bus_cycle(BusControl::Read, 0), None);
    }

    #[test]
    fn port_a_direction_from_mixer_register() {
        let mut psg = Psg::default();
        // Register 7 reset state is 0 -> bit6 clear -> input.
        assert!(psg.port_a_is_input());

        psg.set_register(REG_MIXER, 1 << 6);
        assert!(!psg.port_a_is_input());
    }

    #[test]
    fn bus_control_decode_matches_cpc_bdir_bc1_convention() {
        assert_eq!(BusControl::from_bdir_bc1(false, false), BusControl::Inactive);
        assert_eq!(BusControl::from_bdir_bc1(false, true), BusControl::Read);
        assert_eq!(BusControl::from_bdir_bc1(true, false), BusControl::Write);
        assert_eq!(BusControl::from_bdir_bc1(true, true), BusControl::LatchAddress);
    }
}
