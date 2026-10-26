//! The CPC Dandanator Mini (Dandare's): 512K of flash (an SST39SF040, 32 slots of 16K) on the
//! expansion port, which takes the place of the CPC's ROMs or RAM in two 16K zones. It has no
//! I/O port: it watches the bus, and a program commands it with `FD FD` and an `LD (IY+d),r`
//! (Dandare's macros, CPCDandanatorLoader's ddntr_macros.asm), the byte written being the
//! command's:
//! - `LD (IY+d),B`: zone 0's slot (bits 4-0; bit 5: the zone off);
//! - `LD (IY+d),C`: zone 1's slot, the same;
//! - `LD (IY+d),A`: the configuration. With bit 7 set: bit 6 wait for the next RET to apply it,
//!   bit 5 no more commands until a reset, bit 4 FollowRomEn (the cartridge in place of the
//!   CPC's ROMs, as they are paged), bits 3-2 zone 1 at &C000 instead of &4000 and zone 0 at
//!   &8000 instead of &0000, bits 1-0 the flash off for zone 1 and zone 0. With bit 7 clear:
//!   bits 4-3 FollowRomEn's lower ROM (slot 28-31), bit 1 the flash writable, bits 0 and 2 the
//!   USB serial port's (not emulated).
//!
//! At reset zone 0 is slot 0 at &0000 and zone 1 is off: the cartridge boots in place of the
//! firmware. The zones only answer reads (ROMDIS over a ROM, RAMDIS over RAM); writes go to the
//! RAM, and, the flash writable and zone 0 at &0000, to the flash's command decoder too (byte
//! program, 4K sector and chip erase, its ID), as Dandare's loader programs it.
//! The order of the checks follows CPCEC's and SugarDS' emulations, which the romsets need.

use std::any::Any;
use std::path::PathBuf;

use crate::chip::Chip;
use crate::expansion_port::{ExpansionDevice, ExpansionPin, ExpansionPort};

pub const SIZE: usize = 512 * 1024;
const SLOT: usize = 0x4000;

/// The flash's command decoder (SST39SF040).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Flash {
    #[default]
    Read,
    /// AA written to 5555.
    Unlock1,
    /// then 55 to 2AAA.
    Unlock2,
    /// A0: the next write programs a byte.
    Program,
    /// 80: an erase, unlocked again next.
    Erase,
    EraseUnlock1,
    EraseUnlock2,
    /// 90: the ID read at 0 (&BF, SST) and 1 (&B7, the 39SF040).
    Id
}

pub struct Dandanator {
    flash: Vec<u8>,
    /// Kept in this file when written (`writable`).
    file: Option<PathBuf>,
    writable: bool,
    dirty: bool,
    /// The configuration asked (0: function 0, 1: function 1, 2: zone 0, 3: zone 1) and the one
    /// in effect.
    pending: [u8; 4],
    current: [u8; 4],
    /// The last two opcode fetches, the one being fetched (/M1 low) and the command waiting for
    /// its LD's write (the register: 2, 3 or 0/1 for A).
    fetched: [u8; 2],
    fetching: Option<u8>,
    command: Option<u8>,
    state: Flash
}

impl Dandanator {
    /// A romset: up to 512K, the rest erased (&FF).
    pub fn new(romset: &[u8]) -> Self {
        let mut flash = romset[..romset.len().min(SIZE)].to_vec();
        flash.resize(SIZE, 0xff);
        let mut card = Self {
            flash,
            file: None,
            writable: false,
            dirty: false,
            pending: [0; 4],
            current: [0; 4],
            fetched: [0; 2],
            fetching: None,
            command: None,
            state: Flash::Read
        };
        card.power_on();
        card
    }

    /// The romset in `path`; with `writable`, what the CPC programs into the flash is written back.
    pub fn open(path: impl Into<PathBuf>, writable: bool) -> Result<Self, String> {
        let path = path.into();
        let romset = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if romset.len() > SIZE {
            return Err(format!("{}: {} bytes, more than the 512K of the flash", path.display(), romset.len()));
        }
        let mut card = Self::new(&romset);
        card.file = Some(path);
        card.writable = writable;
        Ok(card)
    }

    pub fn flash(&self) -> &[u8] {
        &self.flash
    }

    /// The configuration in effect: function 0, function 1, zone 0, zone 1.
    pub fn configuration(&self) -> [u8; 4] {
        self.current
    }

    fn power_on(&mut self) {
        self.pending = [0, 0, 0, 0x20];
        self.current = self.pending;
        self.fetched = [0; 2];
        self.fetching = None;
        self.command = None;
        self.state = Flash::Read;
    }

    fn commands_off(&self) -> bool {
        self.current[1] & 0x20 != 0
    }

    fn apply(&mut self) {
        self.current = self.pending;
    }

    /// A command's byte (what a trigger's `LD (IY+d),r` writes): to zone 0 (2), zone 1 (3) or
    /// the configuration (0).
    pub fn command(&mut self, register: u8, value: u8) {
        if self.commands_off() {
            return;
        }
        match register {
            2 | 3 => self.pending[usize::from(register)] = value,
            _ if value & 0x80 != 0 => self.pending[1] = value,
            _ => self.pending[0] = value
        }
        if self.pending[1] & 0x40 == 0 {
            self.apply();
        }
    }

    /// A RET: a configuration waiting for it applies.
    fn ret(&mut self) {
        if self.pending[1] & 0x40 != 0 {
            self.pending[1] &= !0x40;
            if !self.commands_off() {
                self.apply();
            }
        }
    }

    /// The flash's byte read at `address`, if the cartridge answers it (`rom`: the CPC's /ROMEN
    /// low there, `upper` which ROM).
    fn read(&self, address: u16, rom: bool) -> Option<u8> {
        let [function0, function1, zone0, zone1] = self.current;
        let window = usize::from(address >> 14);
        let slot = if function1 & 0x20 == 0 {
            // the zones (zone 0 at &8000 is not gated by its /CE; zone 1 never is: CPCEC)
            let zone0 = (zone0 & 0x20 == 0)
                .then_some(())
                .and_then(|()| if function1 & 0x04 != 0 { (window == 2).then_some(zone0) } else { (window == 0 && function1 & 0x01 == 0).then_some(zone0) });
            let zone1 = (zone1 & 0x20 == 0).then_some(()).and_then(|()| (window == if function1 & 0x08 != 0 { 3 } else { 1 }).then_some(zone1));
            zone0.or(zone1)
        }
        else if function1 & 0x10 != 0 && rom {
            // FollowRomEn: the CPC's ROMs as they are paged, the lower one slot 28-31
            match window {
                0 => Some(28 + ((function0 >> 3) & 3)),
                3 => Some(zone1),
                _ => None
            }
        }
        else {
            None
        };
        slot.map(|slot| self.flash[usize::from(slot & 0x1f) * SLOT + usize::from(address & 0x3fff)])
            .map(|byte| if self.state == Flash::Id { [0xbf, 0xb7][usize::from(address & 1)] } else { byte })
    }

    /// A CPU write: to the flash's decoder when it is writable and zone 0 is at &0000.
    pub fn write(&mut self, address: u16, value: u8) {
        let [function0, function1, zone0, _] = self.current;
        let mapped = function1 & 0x20 == 0 && zone0 & 0x20 == 0 && function1 & 0x05 == 0 && address < 0x4000;
        if !(mapped && function0 & 0x02 != 0) {
            return;
        }
        let offset = usize::from(zone0 & 0x1f) * SLOT + usize::from(address);
        let command = offset & 0x7fff;
        self.state = match (self.state, command, value) {
            (_, _, 0xf0) => Flash::Read,
            (Flash::Read | Flash::Id, 0x5555, 0xaa) => Flash::Unlock1,
            (Flash::Unlock1, 0x2aaa, 0x55) => Flash::Unlock2,
            (Flash::Unlock2, 0x5555, 0xa0) => Flash::Program,
            (Flash::Unlock2, 0x5555, 0x80) => Flash::Erase,
            (Flash::Unlock2, 0x5555, 0x90) => Flash::Id,
            (Flash::Erase, 0x5555, 0xaa) => Flash::EraseUnlock1,
            (Flash::EraseUnlock1, 0x2aaa, 0x55) => Flash::EraseUnlock2,
            (Flash::EraseUnlock2, _, 0x30) => {
                let sector = offset & !0xfff;
                self.flash[sector..sector + 0x1000].fill(0xff);
                self.dirty = true;
                Flash::Read
            },
            (Flash::EraseUnlock2, 0x5555, 0x10) => {
                self.flash.fill(0xff);
                self.dirty = true;
                Flash::Read
            },
            (Flash::Program, _, _) => {
                // programming only clears bits
                self.flash[offset] &= value;
                self.dirty = true;
                Flash::Read
            },
            (Flash::Id, _, _) => Flash::Id,
            _ => Flash::Read
        };
    }

    /// What the CPC programmed, back in the romset's file.
    pub fn save(&mut self) -> Result<(), String> {
        if let (true, true, Some(file)) = (self.dirty, self.writable, &self.file) {
            std::fs::write(file, &self.flash).map_err(|e| format!("{}: {e}", file.display()))?;
            self.dirty = false;
        }
        Ok(())
    }
}

impl ExpansionDevice for Dandanator {
    fn tick(&mut self, port: &mut ExpansionPort) {
        let mreq = !port.get_pin(ExpansionPin::Neg_MREQ);
        let m1 = !port.get_pin(ExpansionPin::Neg_M1);
        // a read the cartridge answers (the opcode fetches too)
        if let Some(address) = port.memory_read() {
            let rom = !port.get_pin(ExpansionPin::Neg_ROMEN);
            if let Some(byte) = self.read(address, rom) {
                port.assert(if rom { ExpansionPin::ROMDIS } else { ExpansionPin::RAMDIS });
                port.drive_data(byte);
            }
        }
        // the opcodes fetched: the byte on the bus while /M1 and /MREQ are low (the last tick's: the
        // byte read is on the bus after /RD), taken as /M1 rises
        if m1 && mreq {
            self.fetching = Some(port.data());
        }
        if port.m1_ends() {
            if let Some(opcode) = self.fetching.take() {
                if self.fetched == [0xfd, 0xfd] {
                    self.command = match opcode {
                        0x70 => Some(2),
                        0x71 => Some(3),
                        0x77 => Some(0),
                        _ => None
                    };
                }
                else if opcode == 0xc9 {
                    self.ret();
                }
                self.fetched = [self.fetched[1], opcode];
            }
        }
        if let Some((address, value)) = port.memory_write_started() {
            match self.command.take() {
                Some(register) => self.command(register, value),
                None => self.write(address, value)
            }
        }
    }

    fn reset(&mut self) {
        let _ = self.save();
        self.power_on();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for Dandanator {
    fn drop(&mut self) {
        if let Err(e) = self.save() {
            eprintln!("the Dandanator: {e}");
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// A romset whose slot `n` is filled with `n`.
    fn card() -> Dandanator {
        let romset: Vec<u8> = (0..SIZE).map(|i| (i / SLOT) as u8).collect();
        Dandanator::new(&romset)
    }

    #[test]
    fn it_boots_in_place_of_the_firmware() {
        let card = card();
        assert_eq!(card.read(0x0000, true), Some(0), "zone 0: slot 0 at &0000");
        assert_eq!(card.read(0x0000, false), Some(0), "over the RAM too");
        assert_eq!(card.read(0x4000, false), None, "zone 1 off");
        assert_eq!(card.read(0xc000, true), None);
    }

    #[test]
    fn zones_are_switched_and_moved() {
        let mut card = card();
        card.command(2, 5);
        card.command(3, 7);
        assert_eq!(card.read(0x1234, true), Some(5));
        assert_eq!(card.read(0x4000, false), Some(7), "zone 1 at &4000");
        // zone 0 at &8000, zone 1 at &C000
        card.command(0, 0x80 | 0x08 | 0x04);
        assert_eq!(card.read(0x0000, true), None);
        assert_eq!(card.read(0x8000, false), Some(5));
        assert_eq!(card.read(0xffff, true), Some(7));
        assert_eq!(card.read(0x4000, false), None);
        // zone 0 off
        card.command(2, 0x20);
        assert_eq!(card.read(0x8000, false), None);
    }

    #[test]
    fn a_configuration_can_wait_for_a_ret() {
        let mut card = card();
        card.command(0, 0x80 | 0x40 | 0x01);
        assert_eq!(card.read(0x0000, true), Some(0), "not yet");
        card.command(2, 3);
        assert_eq!(card.read(0x0000, true), Some(0), "the zones wait too");
        card.ret();
        assert_eq!(card.read(0x0000, true), None, "zone 0's flash off");
        assert_eq!(card.configuration()[2], 3);
    }

    #[test]
    fn commands_can_be_turned_off_until_reset() {
        let mut card = card();
        card.command(0, 0x80 | 0x20);
        card.command(2, 9);
        assert_eq!(card.configuration()[2], 0, "ignored");
        assert_eq!(card.read(0x0000, true), None, "its own map off");
        card.power_on();
        card.command(2, 9);
        assert_eq!(card.read(0x0000, true), Some(9));
    }

    #[test]
    fn follow_rom_en_takes_the_place_of_the_roms() {
        let mut card = card();
        card.command(3, 12);
        card.command(0, 0x10); // function 0: FollowRomEn's lower ROM 28 + 2
        card.command(0, 0x80 | 0x20 | 0x10);
        assert_eq!(card.read(0x0100, true), Some(30), "the lower ROM");
        assert_eq!(card.read(0xc100, true), Some(12), "the upper ROM: zone 1's slot");
        assert_eq!(card.read(0x0100, false), None, "the RAM is the CPC's");
        assert_eq!(card.read(0x4100, false), None);
    }

    /// Dandare's loader's sequences (sstwriter_CPC_v3.asm): 5555 in slot 1, 2AAA in slot 0.
    fn unlock(card: &mut Dandanator, last: u8) {
        card.command(2, 1);
        card.write(0x1555, 0xaa);
        card.command(2, 0);
        card.write(0x2aaa, 0x55);
        card.command(2, 1);
        card.write(0x1555, last);
    }

    #[test]
    fn the_flash_is_erased_and_programmed() {
        let mut card = card();
        card.command(0, 0x02); // writable
        // sector 5 (slot 1, its second 4K)
        unlock(&mut card, 0x80);
        card.write(0x1555, 0xaa);
        card.command(2, 0);
        card.write(0x2aaa, 0x55);
        card.command(2, 1);
        card.write(0x1000, 0x30);
        assert!(card.flash()[SLOT + 0x1000..SLOT + 0x2000].iter().all(|&b| b == 0xff));
        assert_eq!(card.flash()[SLOT + 0x0fff], 1, "the sector before kept");
        // a byte
        unlock(&mut card, 0xa0);
        card.command(2, 1);
        card.write(0x1234, 0x42);
        assert_eq!(card.flash()[SLOT + 0x1234], 0x42);
        // the ID
        unlock(&mut card, 0x90);
        assert_eq!((card.read(0x0000, true), card.read(0x0001, true)), (Some(0xbf), Some(0xb7)));
        card.write(0x0000, 0xf0);
        assert_eq!(card.read(0x0000, true), Some(1));
        // not writable: nothing changes
        card.command(0, 0x00);
        unlock(&mut card, 0xa0);
        card.write(0x0000, 0x00);
        assert_eq!(card.flash()[SLOT], 1);
    }
}
