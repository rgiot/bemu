//! The Nova (PulkoTronics): 32K of non-volatile RAM and a real-time clock, on the expansion
//! port, as its documentation describes it.
//!
//! An OUT to &FE82 maps the NVRAM: the high 4 bits give where (a multiple of &2000), the low 4
//! bits which 8K page (8-&B; anything else unmaps it). Mapped, it masks the RAM or ROM there
//! (RAMDIS or ROMDIS) for the reads, and takes the writes. The last 8 bytes of the fourth page
//! are the clock (an M48T35's): &1FF8 the control (bit 6: read, the registers frozen; bit 7:
//! write, the clock set when it is cleared), then the seconds, minutes, hours, day of the week,
//! day, month and year, in BCD.
//!
//! The NVRAM is kept in a file of the host (when given one); the clock runs on the host's (UTC),
//! with the offset the CPC set.

use std::any::Any;
use std::path::PathBuf;

use crate::chip::Chip;
use crate::expansion_port::{ExpansionDevice, ExpansionPin, ExpansionPort};

const PAGE: usize = 0x2000;
const SIZE: usize = 4 * PAGE;
/// The clock's registers, at the end of the fourth page.
const CLOCK: usize = SIZE - 8;

/// The clock: its registers as last set (write mode) or read (read mode), and how far from the
/// host's time it was set.
#[derive(Debug, Clone, Default)]
struct Clock {
    control: u8,
    registers: [u8; 7],
    offset_seconds: i64
}

pub struct Nova {
    ram: Vec<u8>,
    /// The last byte written to &FE82.
    mapping: u8,
    clock: Clock,
    /// Where the NVRAM is kept.
    file: Option<PathBuf>,
    dirty: bool
}

impl Nova {
    /// The card, its NVRAM read from `file` (created when missing), or blank without one.
    pub fn new(file: Option<PathBuf>) -> Result<Self, String> {
        let mut ram = vec![0u8; SIZE];
        if let Some(path) = &file {
            match std::fs::read(path) {
                Ok(data) => {
                    let length = data.len().min(SIZE);
                    ram[..length].copy_from_slice(&data[..length]);
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(e) => return Err(format!("{}: {e}", path.display()))
            }
        }
        Ok(Self { ram, mapping: 0, clock: Clock::default(), file, dirty: false })
    }

    /// The NVRAM, the clock's registers as they read now.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// Write the NVRAM to its file.
    pub fn save(&mut self) -> std::io::Result<()> {
        if let (Some(path), true) = (&self.file, self.dirty) {
            std::fs::write(path, &self.ram)?;
            self.dirty = false;
        }
        Ok(())
    }

    /// The NVRAM offset of a CPU address, when mapped there.
    fn mapped(&self, address: u16) -> Option<usize> {
        let page = self.mapping & 0x0f;
        if !(8..=0xb).contains(&page) || address & 0xe000 != u16::from(self.mapping & 0xe0) << 8 {
            return None;
        }
        Some(usize::from(page - 8) * PAGE + usize::from(address & 0x1fff))
    }

    fn read(&self, offset: usize) -> u8 {
        if offset < CLOCK {
            return self.ram[offset];
        }
        if offset == CLOCK {
            return self.clock.control;
        }
        let registers = if self.clock.control & 0xc0 != 0 { self.clock.registers } else { self.now() };
        registers[offset - CLOCK - 1]
    }

    fn write(&mut self, offset: usize, value: u8) {
        if offset < CLOCK {
            self.ram[offset] = value;
            self.dirty = true;
            return;
        }
        if offset > CLOCK {
            if self.clock.control & 0x80 != 0 {
                self.clock.registers[offset - CLOCK - 1] = value;
            }
            return;
        }
        let previous = self.clock.control;
        if value & 0xc0 != 0 && previous & 0xc0 == 0 {
            // read or write mode: the registers stop following the time
            self.clock.registers = self.now();
        }
        if previous & 0x80 != 0 && value & 0x80 == 0 {
            // the time written is set
            if let Some(set) = seconds_of(&self.clock.registers) {
                self.clock.offset_seconds = set - host_seconds();
            }
        }
        self.clock.control = value;
    }

    /// The registers for the time now: seconds, minutes, hours, day of the week (1-7, Monday
    /// 1), day, month, year (00-99), in BCD.
    fn now(&self) -> [u8; 7] {
        let seconds = host_seconds() + self.clock.offset_seconds;
        let (days, time) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
        let (year, month, day) = crate::calendar::civil_from_days(days);
        // 1970-01-01 was a Thursday
        let weekday = (days + 3).rem_euclid(7) + 1;
        [
            bcd((time % 60) as u8),
            bcd((time / 60 % 60) as u8),
            bcd((time / 3600) as u8),
            bcd(weekday as u8),
            bcd(day as u8),
            bcd(month as u8),
            bcd(year.rem_euclid(100) as u8)
        ]
    }
}

impl Drop for Nova {
    fn drop(&mut self) {
        if let Err(e) = self.save() {
            eprintln!("Nova NVRAM: {e}");
        }
    }
}

impl ExpansionDevice for Nova {
    fn tick(&mut self, port: &mut ExpansionPort) {
        if let Some((address, value)) = port.io_write_started() {
            if address == 0xfe82 {
                self.mapping = value;
            }
        }
        if let Some((address, value)) = port.memory_write_started() {
            if let Some(offset) = self.mapped(address) {
                self.write(offset, value);
            }
        }
        if let Some(offset) = port.memory_read().and_then(|address| self.mapped(address)) {
            // in place of the ROM or the RAM there
            let rom = !port.get_pin(ExpansionPin::Neg_ROMEN);
            port.assert(if rom { ExpansionPin::ROMDIS } else { ExpansionPin::RAMDIS });
            port.drive_data(self.read(offset));
        }
    }

    fn reset(&mut self) {
        self.mapping = 0;
        let _ = self.save();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

fn host_seconds() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn bcd(value: u8) -> u8 {
    (value / 10) << 4 | value % 10
}

fn from_bcd(value: u8) -> i64 {
    i64::from(value >> 4) * 10 + i64::from(value & 0x0f)
}

/// The time the registers hold, in seconds since 1970 (the years 2000-2099).
fn seconds_of(registers: &[u8; 7]) -> Option<i64> {
    let second = from_bcd(registers[0] & 0x7f);
    let minute = from_bcd(registers[1] & 0x7f);
    let hour = from_bcd(registers[2] & 0x3f);
    let day = from_bcd(registers[4] & 0x3f);
    let month = from_bcd(registers[5] & 0x1f);
    let year = 2000 + from_bcd(registers[6]);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // days from the civil date (Howard Hinnant's algorithm)
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn pages_map_where_asked() {
        let mut nova = Nova::new(None).unwrap();
        assert_eq!(nova.mapped(0x6000), None);
        nova.mapping = 0x69;
        assert_eq!(nova.mapped(0x6000), Some(PAGE));
        assert_eq!(nova.mapped(0x7fff), Some(2 * PAGE - 1));
        assert_eq!(nova.mapped(0x8000), None);
        nova.mapping = 0x60;
        assert_eq!(nova.mapped(0x6000), None, "unmapped");
    }

    #[test]
    fn the_clock_reads_and_is_set() {
        let mut nova = Nova::new(None).unwrap();
        // read mode: frozen registers, a plausible date
        nova.write(CLOCK, 0x40);
        let month = from_bcd(nova.read(CLOCK + 6));
        assert!((1..=12).contains(&month));
        nova.write(CLOCK, 0x00);
        // set 2021-02-03 04:05:06
        nova.write(CLOCK, 0x80);
        for (register, value) in [(1, 0x06), (2, 0x05), (3, 0x04), (5, 0x03), (6, 0x02), (7, 0x21)] {
            nova.write(CLOCK + register, value);
        }
        nova.write(CLOCK, 0x00);
        nova.write(CLOCK, 0x40);
        assert_eq!(nova.read(CLOCK + 7), 0x21);
        assert_eq!(nova.read(CLOCK + 6), 0x02);
        assert_eq!(nova.read(CLOCK + 5), 0x03);
        assert_eq!(nova.read(CLOCK + 3), 0x04);
        // 2021-02-03 was a Wednesday
        assert_eq!(nova.read(CLOCK + 4), 3);
    }

    #[test]
    fn the_nvram_is_kept() {
        let path = std::env::temp_dir().join(format!("bemu-nova-{}.bin", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let mut nova = Nova::new(Some(path.clone())).unwrap();
            nova.write(10, 0x42);
        }
        let nova = Nova::new(Some(path.clone())).unwrap();
        assert_eq!(nova.read(10), 0x42);
        let _ = std::fs::remove_file(&path);
    }
}
