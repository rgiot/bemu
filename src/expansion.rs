//! Cards plugged into the CPC's expansion port.
//!
//! On real hardware, the expansion port taps directly into the Z80's bus -
//! address, data, /IORQ, /WR, ... - exactly like any of the motherboard's
//! own chips do. So a plugged-in card is modelled the same way every other
//! chip in this codebase is: a [`Chip`] impl wired into [`Motherboard`]'s
//! pin-sync step, not a special case bolted onto [`Pal`]/[`Memory`].
//!
//! [`Motherboard`] doesn't use `dyn Chip` for these (`Chip::PinType` is an
//! associated type, so `Chip` isn't object-safe, and every other chip on
//! the board is already a concrete field wired at compile time, not a
//! trait object either) - a new expansion is added the same way `pal`/`ppi`/
//! `crtc` are: a new optional concrete field on `Motherboard`, plus one
//! wiring block in `sync_chips_pins()`. What *is* abstracted here is the
//! logical side other code needs: [`RamBankExpansion`] is a small,
//! `Chip`-independent trait so `Motherboard::memory_config()` doesn't need
//! to know which specific card (if any) is plugged in, only that it can
//! answer "what extra high bits does the current RAM-select write carry,
//! and how many banks do you actually have".
//!
//! [`Motherboard`]: crate::motherboard::Motherboard
//! [`Pal`]: crate::pal::Pal
//! [`Memory`]: crate::memory::Memory

use crate::chip::*;

/// The logical interface a RAM-bank-extending expansion card exposes to the
/// rest of the emulator, independent of how it decodes its own pins.
pub trait RamBankExpansion {
    /// The extra high bits this card currently contributes to the
    /// effective RAM bank number (see [`LargeRamExpansion`] for the
    /// standard >512K protocol these come from). Combined by
    /// `Motherboard::memory_config()` with the gate array/PAL's own D3-D5
    /// bits, which it always computes regardless of whether a card is
    /// plugged in.
    fn ram_bank_high_bits(&self) -> u8;

    /// How many distinct 64kb banks this card provides. A card plugged
    /// into the expansion port physically contends with - and wins over -
    /// the CPC's own internal extra 64K (they're wired to the same bus
    /// lines), so this *replaces* the machine's default single extra page
    /// rather than adding to it: a 64K card still means 64+64=128K total,
    /// not 64+128.
    fn ram_bank_count(&self) -> usize;
}

/// An expansion card implementing the standard ">512K" RAM banking
/// protocol used by real Amstrad-compatible expansions (Yarek's CPC4MB,
/// RAM7, and others) - documented at
/// <https://www.cpcwiki.eu/index.php/Standard_Memory_Expansions#Extended_1M-4M_Expansions_.28RAM7.2FYarek-style.29>.
///
/// The CPC6128-style RAM select (port &7Fxx, value `11bbbccc`: `ccc` the
/// configuration, `bbb` the bank in 64K units, max 512K) is extended by
/// giving meaning to address bits the base machine's own decode already
/// ignores (see `Pal::must_execute_tick`'s `!A15 && A14` - it doesn't look
/// at A8-A10 at all, so on a stock machine any port in &4000-&7FFF behaves
/// identically): a card watching those same bits can layer 3 more bits of
/// bank selection on top without conflicting with the base machine. `bbb`
/// (D3-D5) becomes the *low* 3 bits of a 6-bit bank number; the *high* 3
/// bits come from address lines A8-A10, **inverted** (a card responds when
/// its assigned address line is driven low, standard practice for sharing
/// a bus with cards that may not be fitted) - so port &7Fxx (A8-A10 all
/// high) always means high-bits=0, keeping the first 512K numbered exactly
/// as plain dk'tronics hardware would, and successive ports &7Exx, &7Dxx,
/// ... address the next 512K blocks up to 4MB (64 banks) total.
pub struct LargeRamExpansion {
    pins: PinsBuffer,
    /// Total banks actually fitted on this card (<=64): `bank_count()`.
    bank_count: usize,
    /// Latched high bits (A8-A10, inverted) from the last RAM-select write
    /// this card saw - persists between writes exactly like `Pal`'s own
    /// `MMR` register does.
    high_bits: u8
}

impl LargeRamExpansion {
    /// `bank_count` 64kb banks actually fitted (1-64: e.g. 8 for a 512K
    /// card, 64 for a full 4MB one).
    pub fn new(bank_count: usize) -> Self {
        assert!((1..=64).contains(&bank_count), "a real card has 1 to 64 banks (64kb-4MB), got {bank_count}");
        LargeRamExpansion { pins: (0, 0).into(), bank_count, high_bits: 0 }
    }
}

impl RamBankExpansion for LargeRamExpansion {
    fn ram_bank_high_bits(&self) -> u8 {
        self.high_bits
    }

    fn ram_bank_count(&self) -> usize {
        self.bank_count
    }
}

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum LargeRamExpansionPin {
    A8 = 1 << 0,
    A9 = 1 << 1,
    A10 = 1 << 2,
    A14 = 1 << 3,
    A15 = 1 << 4,
    /// Derived, active-high `!(/IORQ | /WR)` - see how `Pal::IOWR` is fed
    /// in `Motherboard::sync_chips_pins`.
    IOWR = 1 << 5,

    D0 = 1 << 8,
    D1 = 1 << 9,
    D2 = 1 << 10,
    D3 = 1 << 11,
    D4 = 1 << 12,
    D5 = 1 << 13,
    D6 = 1 << 14,
    D7 = 1 << 15
}

impl Into<u64> for LargeRamExpansionPin {
    fn into(self) -> u64 {
        self as u64
    }
}

// `Chip::get_value_from_pins` reads this array MSB-first (see `Pal`'s own
// `PAL_PIN_DATA_BUS`, ordered the same way) - getting this backwards
// silently reverses every data byte read, so both directions are covered
// by `execute_tick`'s own tests via a round trip through `set_pin`/`tick`.
const DATA_PINS: [LargeRamExpansionPin; 8] = {
    use LargeRamExpansionPin::*;
    [D7, D6, D5, D4, D3, D2, D1, D0]
};

impl DataChip for LargeRamExpansion {
    fn pins_data() -> &'static [Self::PinType; 8] {
        &DATA_PINS
    }
}

impl ReadDataChip for LargeRamExpansion {}

impl Chip for LargeRamExpansion {
    type PinType = LargeRamExpansionPin;

    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }

    fn pins_buffer(&self) -> &PinsBuffer {
        &self.pins
    }

    /// Same partial decode as `Pal::must_execute_tick` - a real card can
    /// only ever see what the base machine's own (already-partial) address
    /// decode lets through the bus in the first place.
    fn must_execute_tick(&self) -> bool {
        use LargeRamExpansionPin::*;
        !self.get_pin(A15) && self.get_pin(A14) && self.get_pin(IOWR)
    }

    fn execute_tick(&mut self) {
        use LargeRamExpansionPin::*;
        let data = self.get_data_from_pins();
        // Bits 7-6 must be 11 for this to be a RAM-select write at all
        // (matches `Pal::execute_tick`'s own check).
        if data & 0b1100_0000 != 0b1100_0000 {
            return;
        }
        let raw = (self.get_pin(A10) as u8) << 2 | (self.get_pin(A9) as u8) << 1 | (self.get_pin(A8) as u8);
        self.high_bits = (!raw) & 0b111;
    }

    fn reset(&mut self) {
        self.high_bits = 0;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn select(card: &mut LargeRamExpansion, port: u16, data: u8) {
        use LargeRamExpansionPin::*;
        card.set_pin(A15, (port & (1 << 15)) != 0);
        card.set_pin(A14, (port & (1 << 14)) != 0);
        card.set_pin(A10, (port & (1 << 10)) != 0);
        card.set_pin(A9, (port & (1 << 9)) != 0);
        card.set_pin(A8, (port & (1 << 8)) != 0);
        for (i, pin) in [D0, D1, D2, D3, D4, D5, D6, D7].into_iter().enumerate() {
            card.set_pin(pin, (data & (1 << i)) != 0);
        }
        card.set_pin(IOWR, true);
        card.tick();
        card.set_pin(IOWR, false);
        card.tick();
    }

    /// Port &7Fxx (all of A8-A10 high) must keep behaving exactly like
    /// plain dk'tronics hardware: high bits 0, so the effective bank number
    /// is just `bbb` (D3-D5) unchanged - explicitly required by the spec
    /// ("The first 512K (Port 7Fxxh) are fully compatible with dk'tronics").
    #[test]
    fn port_7f_reproduces_plain_dktronics_numbering() {
        let mut card = LargeRamExpansion::new(64);
        select(&mut card, 0x7Fff, 0b1101_1001); // ccc=001, bbb=011
        assert_eq!(card.ram_bank_high_bits(), 0);
    }

    /// Successive ports 7E, 7D, 7C, ... address the next 512K blocks, per
    /// the spec's own worked example.
    #[test]
    fn successive_ports_address_successive_512k_blocks() {
        let mut card = LargeRamExpansion::new(64);
        for (port, expected_high_bits) in [
            (0x7Fff, 0u8),
            (0x7Eff, 1),
            (0x7Dff, 2),
            (0x7Cff, 3),
            (0x7Bff, 4),
            (0x7Aff, 5),
            (0x79ff, 6),
            (0x78ff, 7)
        ] {
            select(&mut card, port, 0b1100_0000);
            assert_eq!(card.ram_bank_high_bits(), expected_high_bits, "port {port:04x}");
        }
    }

    /// A write whose data bits 7-6 aren't `11` isn't a RAM-select write at
    /// all (same convention `Pal` follows) and must not disturb the latch.
    #[test]
    fn non_ram_select_writes_are_ignored() {
        let mut card = LargeRamExpansion::new(64);
        select(&mut card, 0x7Fff, 0b1101_1000); // establishes high_bits=0
        select(&mut card, 0x78ff, 0b0000_0000); // NOT a RAM-select write (bits 7-6 aren't 11)
        assert_eq!(card.ram_bank_high_bits(), 0, "a non-RAM-select write must not change the latch");
    }
}
