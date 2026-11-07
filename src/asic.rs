//! The CPC+ ASIC (Amstrad 40489), apart from its CRTC (`crtc/asic.rs`, CRTC type 3), its PPI
//! (`ppi.rs`, set up for it by the motherboard) and the classic part of its Gate Array (`ga.rs`):
//! the cartridge, the lock of the new features, the register page mapped at &4000-&7FFF, the
//! programmable raster interrupt, the screen split, the vectored interrupts, the 3 sound "DMA"
//! channels, and on the pixel side ([`AsicVideo`], owned by the Gate Array) the 4096 colour
//! palette, the 16 hardware sprites and the soft scroll.
//!
//! References: the "Arnold V" specification issue 1.5 (Amstrad, 1990), Kevin Thacker's
//! "Extra CPC Plus Hardware Information" (tests on a real 6128+), Quasar CPC's ASIC articles
//! (OffseT, SNN, Zik).

use crate::memory::Bank;

/// A ROM cartridge: up to 32 pages of 16K (a 512K EPROM). A smaller EPROM leaves its upper
/// address lines unconnected: its pages repeat.
#[derive(Clone)]
pub struct Cartridge {
    pages: Vec<Box<Bank>>,
}

impl std::fmt::Debug for Cartridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cartridge({} pages)", self.pages.len())
    }
}

impl Cartridge {
    /// A cartridge image (`.cpr`: a RIFF "AMS!" file of `cbNN` chunks).
    pub fn from_cpr(bytes: &[u8]) -> Result<Self, String> {
        let cpr = cpclib_cpr::Cpr::from_buffer(bytes.to_vec())?;
        let count = cpr
            .banks()
            .iter()
            .map(|bank| usize::from(bank.number()) + 1)
            .max()
            .unwrap_or(0);
        if count == 0 {
            return Err("a cartridge without any page".to_owned());
        }
        // the EPROMs are 2^n pages: the missing pages of the image read as erased
        let mut pages = vec![Box::new(Bank::from_bytes(&[])); count.next_power_of_two()];
        for bank in cpr.banks() {
            *pages[usize::from(bank.number())] = Bank::from_bytes(bank.data());
        }
        Ok(Self { pages })
    }

    /// A cartridge image, or the first one (or `entry`) of a zip archive.
    pub fn from_bytes(bytes: &[u8], name: &str, entry: Option<&str>) -> Result<Self, String> {
        if bytes.starts_with(b"PK\x03\x04") {
            let (entry_name, data) =
                crate::disc_image::zip_entry_with(bytes, entry, &["cpr"], "cartridge (.cpr)")
                    .map_err(|e| format!("{name}: {e}"))?;
            return Self::from_cpr(&data).map_err(|e| format!("{name}#{entry_name}: {e}"));
        }
        Self::from_cpr(bytes).map_err(|e| format!("{name}: {e}"))
    }

    /// A cartridge file: `game.cpr`, `games.zip` (its first cartridge) or `games.zip#game.cpr`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_file(spec: &str) -> Result<Self, String> {
        let (path, entry) = crate::disc_image::split_disc_spec(spec);
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        Self::from_bytes(&bytes, path, entry)
    }

    /// The system cartridge of the 6128+ and 464+ (firmware, BASIC, AMSDOS, Burnin' Rubber),
    /// French (AZERTY firmware) or English.
    pub fn system(french: bool) -> Self {
        let image: &[u8] = if french {
            crate::roms::PLUS_SYSTEM_FR
        } else {
            crate::roms::PLUS_SYSTEM_UK
        };
        Self::from_cpr(image).expect("the embedded system cartridge")
    }

    /// Page `number` as the cartridge's address lines see it.
    pub fn page(&self, number: u8) -> &Bank {
        &self.pages[usize::from(number) % self.pages.len()]
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}

/// The sequence written to the CRTC's select register (&BCxx) that opens the lock, after a
/// non-zero byte then a zero (Arnold V 2.11). The same without its last byte closes it.
const UNLOCK_SEQUENCE: [u8; 15] = [
    0xff, 0x77, 0xb3, 0x51, 0xa8, 0xd4, 0x62, 0x39, 0x9c, 0x46, 0x2b, 0x15, 0x8a, 0xcd, 0xee,
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum LockSequence {
    /// Waiting for a non-zero byte.
    Idle,
    /// A non-zero byte: a zero then starts the sequence.
    NonZero,
    /// That many bytes of the sequence matched.
    Matched(usize),
}

/// What the analogue inputs read with nothing plugged (Kevin Thacker's 6128+).
const ADC_IDLE: [u8; 8] = [0x3f, 0x3f, 0x3f, 0x3f, 0x3f, 0x00, 0x3f, 0x00];

/// The size of a snapshot's "CPC+" chunk (Kevin Thacker's snapshot format, version 3).
pub const SNAPSHOT_CHUNK_LEN: usize = 0x8f8;

/// Microseconds from the moment the line is PRI's while HSYNC is on to the programmable raster
/// interrupt, whatever the HSYNC's width. ACE-DL and AMSpiriT agree (a HALT on PRI 1 with R2=46
/// wakes into &0038 at C0=52 of line 1 for widths 3, 6 and 14); CPCSE's min(width, 7) and the
/// "10 us" of "Extra CPC Plus Hardware Information" ("to be checked") are later.
const RASTER_INTERRUPT_DELAY: u8 = 2;

/// Microseconds a DMA LOAD lasts (Arnold V 2.6: "at least 8 cycles").
const DMA_LOAD_US: u8 = 8;

/// One sound "DMA" channel (Arnold V 2.6).
#[derive(Debug, Clone, Copy, Default)]
struct DmaChannel {
    /// SAR: the next instruction (in the base 64K, word aligned).
    address: u16,
    /// PPR: a pause tick is this + 1 HSYNCs.
    prescaler: u8,
    enabled: bool,
    /// Ticks of PAUSE left, and HSYNCs left in the current tick.
    pause: u16,
    pause_hsyncs: u8,
    /// REPEAT's counter and the instruction after it.
    loop_count: u16,
    loop_address: u16,
    /// The instruction fetched this line, and the microsecond it is executed at.
    fetched: Option<u16>,
    execute_at: u8,
}

/// The CPC+ ASIC's own registers and machinery (the cartridge is fitted to the memory, the video
/// side is the Gate Array's [`AsicVideo`]).
#[derive(Debug, Clone)]
pub struct Asic {
    lock: LockSequence,
    last_select: u8,
    unlocked: bool,
    /// PRI: the raster interrupt line (0: the Gate Array's interrupts instead).
    pri: u8,
    /// SPLT, SSA: the screen split line (0: none) and the address after it.
    splt: u8,
    ssa: u16,
    /// IVR: bits 7-3 of the interrupt vector; bit 0 clear, the DMA interrupts are cleared by the
    /// acknowledge.
    ivr: u8,
    dma: [DmaChannel; 3],
    /// DMA interrupt requests, bit n for channel n.
    dma_interrupts: u8,
    /// The last acknowledge was for the raster interrupt (DCSR bit 7).
    raster_acknowledged: bool,
    /// The Gate Array's interrupt request (R52 or PRI), as last seen outside an acknowledge.
    ga_interrupt: bool,
    /// The CRTC's HSYNC at the previous microsecond.
    hsync: bool,
    /// The line was PRI's while HSYNC was on, at the previous microsecond.
    raster_match: bool,
    /// Microseconds before the programmable raster interrupt.
    raster_countdown: Option<u8>,
    /// Microseconds since the HSYNC started, while the DMA channels work.
    dma_us: Option<u8>,
    /// The channels active on this line, and the microsecond their last execution ends.
    dma_active: [bool; 3],
    dma_end: u8,
    /// The analogue inputs (6 bits each).
    adc: [u8; 8],
}

impl Default for Asic {
    fn default() -> Self {
        Self {
            lock: LockSequence::Idle,
            last_select: 0,
            unlocked: false,
            pri: 0,
            splt: 0,
            ssa: 0,
            // "undefined at reset except that bit 0 will be set to 1"
            ivr: 0x01,
            dma: Default::default(),
            dma_interrupts: 0,
            raster_acknowledged: false,
            ga_interrupt: false,
            hsync: false,
            raster_match: false,
            raster_countdown: None,
            dma_us: None,
            dma_active: [false; 3],
            dma_end: 0,
            adc: ADC_IDLE,
        }
    }
}

/// The 8 bit line number the ASIC compares with PRI and SPLT: 5 bits of C4 and 3 of C9
/// ("Extra CPC Plus Hardware Information"), so a line n also matches at n + 256.
fn line_number(vcc: u8, vlc: u8) -> u8 {
    ((vcc & 0x1f) << 3) | (vlc & 7)
}

impl Asic {
    /// The reset line: the lock closes, the registers with a power-on reset are cleared (Arnold
    /// V appendix I), the DMA channels stop.
    pub fn reset(&mut self) {
        *self = Self {
            ivr: self.ivr | 1,
            ..Self::default()
        };
    }

    /// The new features are unlocked (RMR2 exists).
    pub fn unlocked(&self) -> bool {
        self.unlocked
    }

    /// A byte written to the CRTC's select register (&BCxx): the lock watches the sequence.
    /// Returns whether the lock changed.
    pub fn crtc_select_written(&mut self, value: u8) -> bool {
        let was = self.unlocked;
        let restart = |value: u8, last: u8| match (value, last) {
            (0, 0) => LockSequence::Idle,
            (0, _) => LockSequence::Matched(0),
            _ => LockSequence::NonZero,
        };
        self.lock = match self.lock {
            LockSequence::Matched(n) if n + 1 == UNLOCK_SEQUENCE.len() => {
                // the sequence played up to its last byte: EE opens the lock, anything else
                // closes it
                self.unlocked = value == UNLOCK_SEQUENCE[n];
                restart(value, self.last_select)
            }
            LockSequence::Matched(n) if value == UNLOCK_SEQUENCE[n] => LockSequence::Matched(n + 1),
            _ => restart(value, self.last_select),
        };
        self.last_select = value;
        was != self.unlocked
    }

    /// The screen split as programmed: the line after which the display goes on from SSA.
    pub fn split(&self) -> Option<(u8, u16)> {
        (self.splt != 0).then_some((self.splt, self.ssa))
    }

    /// Analogue input `channel` (0-7) reads `value` (0-&3F; the joysticks and paddles of the
    /// analogue port, &3F or 0 with nothing plugged).
    pub fn set_analogue_input(&mut self, channel: usize, value: u8) {
        self.adc[channel & 7] = value & 0x3f;
    }

    /// A raster interrupt line is programmed: the Gate Array's own interrupts are off.
    pub fn programmable_raster(&self) -> bool {
        self.pri != 0
    }

    /// A DMA channel requests an interrupt.
    pub fn dma_interrupt(&self) -> bool {
        self.dma_interrupts != 0
    }

    /// The Gate Array's interrupt request, sampled outside the acknowledge cycles (the Gate
    /// Array drops it as soon as the acknowledge begins).
    pub fn note_ga_interrupt(&mut self, requested: bool) {
        self.ga_interrupt = requested;
    }

    /// An interrupt acknowledge: the vector the ASIC puts on the bus (IVR's bits 7-3, then the
    /// source: raster 6, DMA 0 4, DMA 1 2, DMA 2 0 - "Extra CPC Plus Hardware Information"), the
    /// raster first, then DMA 2 down to 0. A DMA interrupt is cleared by it when IVR's bit 0 is
    /// clear.
    ///
    /// The vectored interrupt bug: the raster's vector comes out as DMA channel 0's (4) unless
    /// the Z80 runs in the upper half of an 8K block (`pc` bit 13 set). Quasar CPC 10 (OffseT)
    /// measured about 70% of the raster interrupts taking the DMA 0 vector on a 6128+, and
    /// CPCWiki ties it to "particular portions of memory"; the rule itself is not verified.
    pub fn acknowledge(&mut self, pc: u16) -> u8 {
        let source = if self.ga_interrupt {
            self.raster_acknowledged = true;
            if pc & 0x2000 != 0 {
                6
            } else {
                4
            }
        } else {
            self.raster_acknowledged = false;
            match (0..3)
                .rev()
                .find(|&channel| self.dma_interrupts & (1 << channel) != 0)
            {
                Some(channel) => {
                    if self.ivr & 1 == 0 {
                        self.dma_interrupts &= !(1 << channel);
                    }
                    (2 - channel) * 2
                }
                None => 6,
            }
        };
        (self.ivr & 0xf8) | source
    }

    /// DCSR as read: the last acknowledge was the raster's, the DMA interrupts (channel 0 on
    /// bit 6, 1 on 5, 2 on 4), the channels enabled.
    fn dcsr(&self) -> u8 {
        let interrupts = (0..3).fold(0, |bits, channel| {
            bits | if self.dma_interrupts & (1 << channel) != 0 {
                0x40 >> channel
            } else {
                0
            }
        });
        let enabled = (0..3).fold(0, |bits, channel| {
            bits | (u8::from(self.dma[channel].enabled) << channel)
        });
        (u8::from(self.raster_acknowledged) << 7) | interrupts | enabled
    }

    /// A read of the register page (&4000-&7FFF mapped by RMR2), `None` where no register
    /// answers: the RAM below is read there. Kevin Thacker read changing data (&B0/&B1, &7E/&7F)
    /// in those areas on his 6128+, but Barbar (Benediction, 2016) fills its sprites from a table
    /// in the RAM at &7800 with the page mapped, and shows them as on the real machine.
    pub fn read(&self, video: &AsicVideo, address: u16) -> Option<u8> {
        Some(match address {
            0x4000..=0x4fff => {
                video.sprite_data[usize::from(address >> 8 & 0x0f)][usize::from(address & 0xff)]
            }
            0x6000..=0x607f => video.read_sprite_register(address),
            0x6400..=0x643f => video.read_palette(address),
            0x6808..=0x680f => self.adc[usize::from(address & 7)],
            0x6c00..=0x6c0f => self.dcsr(),
            _ => return None,
        })
    }

    /// A write to the register page: what is not a register is lost (it does not reach the RAM
    /// below either).
    pub fn write(&mut self, video: &mut AsicVideo, address: u16, value: u8) {
        match address {
            0x4000..=0x4fff => video.write_sprite_data(address, value),
            0x6000..=0x607f => video.write_sprite_register(address, value),
            0x6400..=0x643f => video.write_palette(address, value),
            0x6800 => {
                self.pri = value;
                video.pri_active = value != 0;
            }
            0x6801 => self.splt = value,
            0x6802 => self.ssa = (self.ssa & 0x00ff) | (u16::from(value) << 8),
            0x6803 => self.ssa = (self.ssa & 0xff00) | u16::from(value),
            0x6804 => video.sscr = value,
            0x6805 => self.ivr = value,
            0x6c00..=0x6c0b => {
                let channel = &mut self.dma[usize::from((address >> 2) & 3)];
                match address & 3 {
                    0 => channel.address = (channel.address & 0xff00) | u16::from(value & 0xfe),
                    1 => channel.address = (channel.address & 0x00ff) | (u16::from(value) << 8),
                    // taken at once: the count in progress wraps at the next HSYNC ("L'unité de
                    // pause est prise en compte immédiatement", ASMtrad, DMA son)
                    2 => {
                        channel.prescaler = value;
                        channel.pause_hsyncs = 0;
                    }
                    _ => {}
                }
            }
            0x6c0f => {
                for channel in 0..3 {
                    let enable = value & (1 << channel) != 0;
                    let dma = &mut self.dma[channel];
                    if enable && !dma.enabled {
                        dma.pause = 0;
                    }
                    dma.enabled = enable;
                    // a 1 written to an interrupt bit clears it
                    if value & (0x40 >> channel) != 0 {
                        self.dma_interrupts &= !(1 << channel);
                    }
                }
            }
            _ => {}
        }
    }

    /// The snapshot's "CPC+" chunk ([`SNAPSHOT_CHUNK_LEN`] bytes): the register page as last
    /// written, the DMA channels' counters, RMR2, the lock and its sequence.
    pub fn snapshot_chunk(&self, video: &AsicVideo) -> Vec<u8> {
        let mut chunk = vec![0u8; SNAPSHOT_CHUNK_LEN];
        // the sprites, two pixels a byte
        for (sprite, data) in video.sprite_data.iter().enumerate() {
            for (pair, pixels) in data.chunks(2).enumerate() {
                chunk[sprite * 128 + pair] = (pixels[0] << 4) | (pixels[1] & 0x0f);
            }
        }
        for sprite in 0..16 {
            chunk[0x800 + sprite * 8..][..4].copy_from_slice(&video.sprite_position[sprite]);
            chunk[0x800 + sprite * 8 + 4] = video.sprite_magnification[sprite];
        }
        for (index, entry) in video.palette.iter().enumerate() {
            chunk[0x880 + index * 2] = *entry as u8;
            chunk[0x881 + index * 2] = (*entry >> 8) as u8;
        }
        chunk[0x8c0] = self.pri;
        chunk[0x8c1] = self.splt;
        chunk[0x8c2] = (self.ssa >> 8) as u8;
        chunk[0x8c3] = self.ssa as u8;
        chunk[0x8c4] = video.sscr;
        chunk[0x8c5] = self.ivr;
        chunk[0x8c8..0x8d0].copy_from_slice(&self.adc);
        for (channel, dma) in self.dma.iter().enumerate() {
            chunk[0x8d0 + channel * 4..][..2].copy_from_slice(&dma.address.to_le_bytes());
            chunk[0x8d2 + channel * 4] = dma.prescaler;
            let internal = &mut chunk[0x8e0 + channel * 7..][..7];
            internal[0..2].copy_from_slice(&dma.loop_count.to_le_bytes());
            internal[2..4].copy_from_slice(&dma.loop_address.to_le_bytes());
            internal[4..6].copy_from_slice(&dma.pause.to_le_bytes());
            internal[6] = dma.pause_hsyncs;
        }
        chunk[0x8df] = self.dcsr();
        chunk[0x8f5] = 0xa0 | video.rmr2;
        chunk[0x8f6] = u8::from(self.unlocked);
        chunk[0x8f7] = match self.lock {
            LockSequence::Idle => 0,
            LockSequence::NonZero => 1,
            LockSequence::Matched(n) => n as u8 + 2,
        };
        chunk
    }

    /// Restore the ASIC from a snapshot's "CPC+" chunk (see [`Self::snapshot_chunk`]).
    pub fn restore_snapshot_chunk(
        &mut self,
        video: &mut AsicVideo,
        chunk: &[u8],
    ) -> Result<(), String> {
        if chunk.len() < SNAPSHOT_CHUNK_LEN {
            return Err(format!(
                "a CPC+ chunk of {} bytes, not {SNAPSHOT_CHUNK_LEN}",
                chunk.len()
            ));
        }
        for (sprite, data) in video.sprite_data.iter_mut().enumerate() {
            for (pair, pixels) in data.chunks_mut(2).enumerate() {
                let byte = chunk[sprite * 128 + pair];
                pixels[0] = byte >> 4;
                pixels[1] = byte & 0x0f;
            }
        }
        for sprite in 0..16 {
            video.sprite_position[sprite].copy_from_slice(&chunk[0x800 + sprite * 8..][..4]);
            video.sprite_magnification[sprite] = chunk[0x800 + sprite * 8 + 4] & 0x0f;
        }
        for (index, entry) in video.palette.iter_mut().enumerate() {
            *entry = u16::from(chunk[0x880 + index * 2])
                | (u16::from(chunk[0x881 + index * 2] & 0x0f) << 8);
        }
        self.pri = chunk[0x8c0];
        video.pri_active = self.pri != 0;
        self.splt = chunk[0x8c1];
        self.ssa = (u16::from(chunk[0x8c2]) << 8) | u16::from(chunk[0x8c3]);
        video.sscr = chunk[0x8c4];
        self.ivr = chunk[0x8c5];
        for (channel, value) in chunk[0x8c8..0x8d0].iter().enumerate() {
            self.adc[channel] = value & 0x3f;
        }
        let dcsr = chunk[0x8df];
        self.raster_acknowledged = dcsr & 0x80 != 0;
        self.dma_interrupts = 0;
        for (channel, dma) in self.dma.iter_mut().enumerate() {
            dma.address =
                u16::from_le_bytes([chunk[0x8d0 + channel * 4], chunk[0x8d1 + channel * 4]])
                    & 0xfffe;
            dma.prescaler = chunk[0x8d2 + channel * 4];
            let internal = &chunk[0x8e0 + channel * 7..][..7];
            dma.loop_count = u16::from_le_bytes([internal[0], internal[1]]) & 0x0fff;
            dma.loop_address = u16::from_le_bytes([internal[2], internal[3]]);
            dma.pause = u16::from_le_bytes([internal[4], internal[5]]) & 0x0fff;
            dma.pause_hsyncs = internal[6];
            dma.enabled = dcsr & (1 << channel) != 0;
            dma.fetched = None;
            if dcsr & (0x40 >> channel) != 0 {
                self.dma_interrupts |= 1 << channel;
            }
        }
        self.dma_us = None;
        video.rmr2 = chunk[0x8f5] & 0x1f;
        self.unlocked = chunk[0x8f6] != 0;
        video.unlocked = self.unlocked;
        self.lock = match chunk[0x8f7] {
            0 => LockSequence::Idle,
            1 => LockSequence::NonZero,
            state => LockSequence::Matched(usize::from(state - 2).min(UNLOCK_SEQUENCE.len() - 1)),
        };
        video.line = None;
        Ok(())
    }

    /// One microsecond, after the CRTC counted its character: the HSYNC start (raster
    /// interrupt, DMA), the DMA channels' work.
    /// `read_ram` reads the base 64K, as the DMA does whatever the CPU maps.
    pub fn microsecond(
        &mut self,
        crtc: &crate::crtc::Crtc,
        ga: &mut crate::ga::GateArray,
        psg: &mut crate::psg::Psg,
        read_ram: impl Fn(u16) -> u8,
    ) {
        use crate::chip::Chip;
        use crate::crtc::CrtcPin;

        let counters = crtc.counters();
        let hsync = crtc.get_pin(CrtcPin::HS);
        if hsync && !self.hsync {
            self.start_dma_line();
        }
        self.hsync = hsync;

        // the raster interrupt is asked when the line is PRI's while HSYNC is on: from the HSYNC
        // start on that line, or from the line start when the previous line's HSYNC runs over
        // into it (ACE-DL: R2=51 and a width of 14, the HSYNC ending on C0=0; RE-WIRED, Benediction
        // 2026, writes its split line for each line just after this interrupt and relies on it)
        let matches = hsync && self.pri != 0 && line_number(counters.vcc, counters.vlc) == self.pri;
        if matches && !self.raster_match {
            self.raster_countdown = Some(RASTER_INTERRUPT_DELAY);
        }
        self.raster_match = matches;

        if let Some(left) = self.raster_countdown {
            if left <= 1 {
                self.raster_countdown = None;
                ga.raise_raster_interrupt();
            } else {
                self.raster_countdown = Some(left - 1);
            }
        }

        if let Some(us) = self.dma_us {
            let us = us + 1;
            self.dma_us = Some(us);
            self.dma_microsecond(us, psg, read_ram);
        }
    }

    /// The HSYNC starts: the pauses count down, the channels left active fetch an instruction
    /// each (Arnold V 2.6: one dead cycle, then one fetch per active channel, then the
    /// executions).
    fn start_dma_line(&mut self) {
        for (channel, active) in self.dma.iter_mut().zip(self.dma_active.iter_mut()) {
            *active = false;
            channel.fetched = None;
            if !channel.enabled {
                continue;
            }
            if channel.pause > 0 {
                if channel.pause_hsyncs == 0 {
                    channel.pause -= 1;
                    channel.pause_hsyncs = channel.prescaler;
                } else {
                    channel.pause_hsyncs -= 1;
                }
                if channel.pause > 0 {
                    continue;
                }
            }
            *active = true;
        }
        self.dma_us = self.dma_active.iter().any(|&active| active).then_some(0);
    }

    fn dma_microsecond(&mut self, us: u8, psg: &mut crate::psg::Psg, read_ram: impl Fn(u16) -> u8) {
        let active: Vec<usize> = (0..3).filter(|&channel| self.dma_active[channel]).collect();
        let fetches = active.len() as u8;
        if us <= fetches {
            let dma = &mut self.dma[active[usize::from(us) - 1]];
            let address = dma.address & 0xfffe;
            dma.fetched = Some(
                u16::from(read_ram(address)) | (u16::from(read_ram(address.wrapping_add(1))) << 8),
            );
            dma.address = address.wrapping_add(2);
        }
        if us == fetches {
            // all fetched: the executions follow, one after the other
            let mut at = fetches + 1;
            for &channel in &active {
                let dma = &mut self.dma[channel];
                dma.execute_at = at;
                at += match dma.fetched {
                    Some(instruction) if instruction & 0x7000 == 0 => DMA_LOAD_US,
                    _ => 1,
                };
            }
            self.dma_end = at - 1;
            return;
        }
        if us < fetches {
            return;
        }
        for &channel in &active {
            let dma = self.dma[channel];
            if let Some(instruction) = dma.fetched.filter(|_| dma.execute_at == us) {
                self.dma[channel].fetched = None;
                self.execute_dma(channel, instruction, psg);
            }
        }
        if us >= self.dma_end {
            self.dma_us = None;
        }
    }

    /// One DMA instruction (Arnold V 2.6; Quasar CPC 20 for the combinations: bit 15 is
    /// ignored, LOAD when bits 14-12 are 0, else bit 12 PAUSE, 13 REPEAT, 14 the control group
    /// whose bit 0 is LOOP, 4 INT and 5 STOP).
    fn execute_dma(&mut self, channel: usize, instruction: u16, psg: &mut crate::psg::Psg) {
        let dma = &mut self.dma[channel];
        let kind = (instruction >> 12) & 7;
        if kind == 0 {
            // the PSG's address register is put back as it was: the CPU's selection stays
            psg.set_register((instruction >> 8) as u8 & 0x0f, instruction as u8);
            return;
        }
        let argument = instruction & 0x0fff;
        if kind & 1 != 0 && argument != 0 {
            dma.pause = argument;
            dma.pause_hsyncs = dma.prescaler;
        }
        // REPEAT 0, as PAUSE 0, has no effect (Arnold V 2.6)
        if kind & 2 != 0 && argument != 0 {
            dma.loop_count = argument;
            dma.loop_address = dma.address;
        }
        if kind & 4 != 0 {
            if instruction & 0x0001 != 0 && dma.loop_count != 0 {
                dma.loop_count -= 1;
                dma.address = dma.loop_address;
            }
            if instruction & 0x0010 != 0 {
                self.dma_interrupts |= 1 << channel;
            }
            if instruction & 0x0020 != 0 {
                dma.enabled = false;
            }
        }
    }
}

/// What the Gate Array's colour logic turns a hardware ink number into on the CPC+: levels of
/// red, green and blue out of 15 ("Extra CPC Plus Hardware Information", measured).
const INK_LEVELS: [(u8, u8, u8); 32] = [
    (6, 6, 6),
    (6, 6, 6),
    (0, 15, 6),
    (15, 15, 6),
    (0, 0, 6),
    (15, 0, 6),
    (0, 6, 6),
    (15, 6, 6),
    (15, 0, 6),
    (15, 15, 6),
    (15, 15, 0),
    (15, 15, 15),
    (15, 0, 0),
    (15, 0, 15),
    (15, 6, 0),
    (15, 6, 15),
    (0, 0, 6),
    (0, 15, 6),
    (0, 15, 0),
    (0, 15, 15),
    (0, 0, 0),
    (0, 0, 15),
    (0, 6, 0),
    (0, 6, 15),
    (6, 0, 6),
    (6, 15, 6),
    (6, 15, 0),
    (6, 15, 15),
    (6, 0, 0),
    (6, 0, 15),
    (6, 6, 0),
    (6, 6, 15),
];

/// A palette entry as the register page holds it: red in bits 7-4, blue in 3-0, green in 11-8.
fn palette_entry((red, green, blue): (u8, u8, u8)) -> u16 {
    (u16::from(green) << 8) | (u16::from(red) << 4) | u16::from(blue)
}

/// The pixel side of the ASIC, which replaces the Gate Array's: the 32 entry palette of 4096
/// colours (16 pens, the border, 15 sprite colours), the 16 sprites, the soft scroll; and RMR2,
/// written through the Gate Array's port.
#[derive(Debug, Clone)]
pub struct AsicVideo {
    /// RMR2 can be written (the lock is open).
    pub(crate) unlocked: bool,
    /// RMR2: bits 4-3 where the lower ROM is (and the register page), bits 2-0 its page.
    pub(crate) rmr2: u8,
    pub(crate) palette: [u16; 32],
    sprite_data: [[u8; 256]; 16],
    /// The position registers as written (X, Y: low then high byte).
    sprite_position: [[u8; 4]; 16],
    sprite_magnification: [u8; 16],
    /// SSCR (its vertical scroll goes to the CRTC).
    pub(crate) sscr: u8,
    /// PRI is not 0: the R52 interrupts are off.
    pub(crate) pri_active: bool,
    /// What an `IN` from the Gate Array's or the CRTC's ports reads, and writes to them as an
    /// `OUT` would: &79 on a 6128+, &78 on a 464+ ("Extra CPC Plus Hardware Information": Into
    /// the Eagle's Nest's border).
    pub(crate) bus_value: u8,
    /// The sprite pens of the line being drawn (0: none), by X (10 bits), and that line.
    line_pens: Box<[u8; 1024]>,
    line: Option<u16>,
    /// The screen pens of the last ticks, newest first, for the horizontal soft scroll.
    scroll_ring: [u8; 16],
    /// Where the byte being displayed is: X of its first pixel, Y of its line.
    byte_x: u16,
    byte_y: u16,
    /// Pixels displayed since the border ended on this line (for the extended border).
    displayed: u16,
}

impl Default for AsicVideo {
    fn default() -> Self {
        Self {
            unlocked: false,
            rmr2: 0,
            palette: [palette_entry(INK_LEVELS[20]); 32],
            sprite_data: [[0; 256]; 16],
            sprite_position: [[0; 4]; 16],
            sprite_magnification: [0; 16],
            sscr: 0,
            pri_active: false,
            bus_value: 0x79,
            line_pens: Box::new([0; 1024]),
            line: None,
            scroll_ring: [0; 16],
            byte_x: 0,
            byte_y: 0,
            displayed: 0,
        }
    }
}

impl AsicVideo {
    /// The reset: the registers with a power-on reset (Arnold V appendix I: the magnifications,
    /// SSCR, PRI; RMR2 back to the first page at &0000).
    pub fn reset(&mut self) {
        self.unlocked = false;
        self.rmr2 = 0;
        self.sprite_magnification = [0; 16];
        self.sscr = 0;
        self.pri_active = false;
        self.line = None;
    }

    /// The Gate Array's port sets pen `pen` (16: the border) to the hardware ink `ink`.
    pub fn set_ink(&mut self, pen: u8, ink: u8) {
        self.palette[usize::from(pen)] = palette_entry(INK_LEVELS[usize::from(ink & 0x1f)]);
    }

    /// Palette entry `index` (0-15 pens, 16 border, 17-31 sprite colours 1-15) as levels for the
    /// monitor (0 to 100).
    pub fn rgb(&self, index: usize) -> crate::ga::RgbLevels {
        let entry = self.palette[index];
        let level = |nibble: u16| f32::from((nibble & 0x0f) as u8) * 100.0 / 15.0;
        (level(entry >> 4), level(entry >> 8), level(entry)).into()
    }

    fn read_palette(&self, address: u16) -> u8 {
        let entry = self.palette[usize::from((address >> 1) & 0x1f)];
        if address & 1 == 0 {
            entry as u8
        } else {
            (entry >> 8) as u8 & 0x0f
        }
    }

    fn write_palette(&mut self, address: u16, value: u8) {
        let entry = &mut self.palette[usize::from((address >> 1) & 0x1f)];
        *entry = if address & 1 == 0 {
            (*entry & 0x0f00) | u16::from(value)
        } else {
            (*entry & 0x00ff) | (u16::from(value & 0x0f) << 8)
        };
    }

    /// Sprite data keeps the 4 low bits ("Extra CPC Plus Hardware Information").
    fn write_sprite_data(&mut self, address: u16, value: u8) {
        let sprite = usize::from(address >> 8 & 0x0f);
        self.sprite_data[sprite][usize::from(address & 0xff)] = value & 0x0f;
        self.line = None;
    }

    /// The position (X 10 bits, Y 9 bits, two's complement) and the magnification (bits 3-2 X,
    /// 1-0 Y: 0 off, then x1, x2, x4), at offsets 0-3 and 4 (5-7 mirror 4) of each 8 bytes.
    fn write_sprite_register(&mut self, address: u16, value: u8) {
        let sprite = usize::from((address >> 3) & 0x0f);
        match address & 7 {
            offset @ 0..=3 => self.sprite_position[sprite][usize::from(offset)] = value,
            _ => self.sprite_magnification[sprite] = value & 0x0f,
        }
        self.line = None;
    }

    /// Offsets 4-7 read as 0-3; the high bytes as their sign extension.
    fn read_sprite_register(&self, address: u16) -> u8 {
        let position = self.sprite_position[usize::from((address >> 3) & 0x0f)];
        match address & 3 {
            1 => {
                if position[1] & 3 == 3 {
                    0xff
                } else {
                    position[1] & 3
                }
            }
            3 => {
                if position[3] & 1 == 1 {
                    0xff
                } else {
                    position[3] & 1
                }
            }
            offset => position[usize::from(offset)],
        }
    }

    fn sprite_x(&self, sprite: usize) -> u16 {
        let position = self.sprite_position[sprite];
        (u16::from(position[0]) | (u16::from(position[1] & 3) << 8)) & 0x3ff
    }

    fn sprite_y(&self, sprite: usize) -> u16 {
        let position = self.sprite_position[sprite];
        (u16::from(position[2]) | (u16::from(position[3] & 1) << 8)) & 0x1ff
    }

    /// The sprite pens of line `y`: sprite 0 in front of 1... (Arnold V 2.1).
    fn draw_line(&mut self, y: u16) {
        self.line_pens.fill(0);
        for sprite in (0..16).rev() {
            let magnification = self.sprite_magnification[sprite];
            let (mx, my) = (magnification >> 2, magnification & 3);
            if mx == 0 || my == 0 {
                continue;
            }
            let (zoom_x, zoom_y) = (1u16 << (mx - 1), 1u16 << (my - 1));
            let dy = y.wrapping_sub(self.sprite_y(sprite)) & 0x1ff;
            if dy >= 16 * zoom_y {
                continue;
            }
            let row = &self.sprite_data[sprite][usize::from(dy / zoom_y) * 16..][..16];
            let x0 = self.sprite_x(sprite);
            for (column, &pen) in row.iter().enumerate() {
                if pen == 0 {
                    continue;
                }
                for k in 0..zoom_x {
                    let x = x0.wrapping_add(column as u16 * zoom_x + k) & 0x3ff;
                    self.line_pens[usize::from(x)] = pen;
                }
            }
        }
        self.line = Some(y);
    }

    /// The byte about to be displayed was fetched for character `hcc` (its 2nd byte with
    /// `second`) on row `vcc`, line `vlc` of the CRTC: sprites are placed on the CRTC's counters
    /// ("Extra CPC Plus Hardware Information"), one X per mode 2 pixel, one Y per line.
    pub fn byte_fetched(&mut self, hcc: u8, second: bool, vcc: u8, vlc: u8) {
        self.byte_x = (u16::from(hcc) * 16 + if second { 8 } else { 0 }) & 0x3ff;
        self.byte_y = ((u16::from(vcc) << 3) | u16::from(vlc & 7)) & 0x1ff;
    }

    /// The horizontal soft scroll (SSCR bits 3-0): the screen pens come that many mode 2 pixels
    /// late. `pen` is the screen pen of this tick; the one to show is returned.
    pub fn scroll(&mut self, pen: u8) -> u8 {
        self.scroll_ring.copy_within(0..15, 1);
        self.scroll_ring[0] = pen;
        self.scroll_ring[usize::from(self.sscr & 0x0f)]
    }

    /// SSCR bit 7: the border covers the first 2 bytes (16 mode 2 pixels) of each line, counted
    /// from where the display starts. Called for every pixel, `display` the display enable as
    /// the Gate Array shows it (the character fetched is a character ahead of the one shown: by
    /// C0 the mask fell on border and Eerie Forest showed its 2 bytes of garbage on the left,
    /// which AMSpiriT masks).
    pub fn extended_border(&mut self, display: bool) -> bool {
        if !display {
            self.displayed = 0;
            return false;
        }
        self.displayed = self.displayed.saturating_add(1);
        self.sscr & 0x80 != 0 && self.displayed <= 16
    }

    /// The sprite pen (1-15, 0 for none) at pixel `pixel` of the byte being displayed.
    pub fn sprite_pen(&mut self, pixel: usize) -> u8 {
        if self.line != Some(self.byte_y) {
            self.draw_line(self.byte_y);
        }
        self.line_pens[usize::from((self.byte_x + pixel as u16) & 0x3ff)]
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn write_sequence(asic: &mut Asic, bytes: &[u8]) {
        for &byte in bytes {
            asic.crtc_select_written(byte);
        }
    }

    #[test]
    fn the_lock_opens_with_the_sequence_and_closes_without_its_last_byte() {
        let mut asic = Asic::default();
        write_sequence(
            &mut asic,
            &[
                0xff, 0x00, 0xff, 0x77, 0xb3, 0x51, 0xa8, 0xd4, 0x62, 0x39, 0x9c, 0x46, 0x2b, 0x15,
                0x8a, 0xcd, 0xee,
            ],
        );
        assert!(asic.unlocked());
        write_sequence(
            &mut asic,
            &[
                0x01, 0x00, 0xff, 0x77, 0xb3, 0x51, 0xa8, 0xd4, 0x62, 0x39, 0x9c, 0x46, 0x2b, 0x15,
                0x8a, 0xcd, 0xa5,
            ],
        );
        assert!(!asic.unlocked());
        // a wrong byte, then the sequence again
        write_sequence(
            &mut asic,
            &[
                0xff, 0x00, 0xff, 0x77, 0x00, 0xff, 0x77, 0xb3, 0x51, 0xa8, 0xd4, 0x62, 0x39, 0x9c,
                0x46, 0x2b, 0x15, 0x8a, 0xcd, 0xee,
            ],
        );
        assert!(asic.unlocked());
        // a register selection alone does nothing
        write_sequence(&mut asic, &[12, 13, 0, 1]);
        assert!(asic.unlocked());
    }

    #[test]
    fn the_register_page_reads_back_as_the_real_asic() {
        let mut asic = Asic::default();
        let mut video = AsicVideo::default();
        asic.write(&mut video, 0x4123, 0xa7);
        assert_eq!(
            asic.read(&video, 0x4123),
            Some(0x07),
            "sprite data keeps 4 bits"
        );
        asic.write(&mut video, 0x6008, 0x34);
        asic.write(&mut video, 0x6009, 0x03);
        asic.write(&mut video, 0x600b, 0x01);
        assert_eq!(
            asic.read(&video, 0x6009),
            Some(0xff),
            "X's high byte sign extended"
        );
        assert_eq!(
            asic.read(&video, 0x600c),
            Some(0x34),
            "the magnification reads X"
        );
        assert_eq!(asic.read(&video, 0x600b), Some(0xff));
        asic.write(&mut video, 0x6402, 0x5a);
        asic.write(&mut video, 0x6403, 0xf3);
        assert_eq!(
            (asic.read(&video, 0x6402), asic.read(&video, 0x6403)),
            (Some(0x5a), Some(0x03))
        );
        assert_eq!(asic.read(&video, 0x6808), Some(0x3f));
        asic.write(&mut video, 0x6c0f, 0x05);
        assert_eq!(
            asic.read(&video, 0x6c03),
            Some(0x05),
            "the DMA registers read as DCSR"
        );
        assert_eq!(
            asic.read(&video, 0x7820),
            None,
            "the RAM below is read where no register is"
        );
    }

    #[test]
    fn the_gate_array_inks_reach_the_new_palette() {
        let mut video = AsicVideo::default();
        video.set_ink(16, 0x14); // the border black
        video.set_ink(1, 0x0a); // bright yellow
        assert_eq!(video.palette[16], 0x000);
        assert_eq!(video.palette[1], 0xff0, "green 15, red 15, blue 0");
    }

    #[test]
    fn sprites_are_drawn_with_their_magnification_and_priority() {
        let mut asic = Asic::default();
        let mut video = AsicVideo::default();
        for offset in 0..256 {
            asic.write(&mut video, 0x4000 + offset, 1);
            asic.write(&mut video, 0x4100 + offset, 2);
        }
        // sprite 0 at (32, 8) x2 in X, sprite 1 at (40, 8) x1
        for (sprite, x) in [(0u16, 32u8), (1, 40)] {
            asic.write(&mut video, 0x6000 + sprite * 8, x);
            asic.write(&mut video, 0x6001 + sprite * 8, 0);
            asic.write(&mut video, 0x6002 + sprite * 8, 8);
            asic.write(&mut video, 0x6003 + sprite * 8, 0);
        }
        asic.write(&mut video, 0x6004, 0b1001);
        asic.write(&mut video, 0x600c, 0b0101);
        video.byte_fetched(2, false, 1, 0); // line 8, X 32
        assert_eq!(video.sprite_pen(0), 1);
        video.byte_fetched(2, true, 1, 0); // X 40: sprite 0 in front
        assert_eq!(video.sprite_pen(0), 1);
        video.byte_fetched(4, false, 1, 0); // X 64: sprite 0 (32 wide) ends, sprite 1 goes on
        assert_eq!(video.sprite_pen(0), 0, "sprite 1 ends at 56");
        video.byte_fetched(3, true, 1, 0); // X 56
        assert_eq!(video.sprite_pen(0), 1);
        video.byte_fetched(3, true, 3, 0); // line 24: both are 16 lines high
        assert_eq!(video.sprite_pen(0), 0);
    }

    #[test]
    fn dma_instructions_load_pause_repeat_and_stop() {
        let mut asic = Asic::default();
        let mut video = AsicVideo::default();
        let mut psg = crate::psg::Psg::default();
        // LOAD R8,&0F; REPEAT 1; LOAD R0,n; LOOP; INT|STOP
        let program: [u16; 5] = [0x080f, 0x2001, 0x0042, 0x4001, 0x4030];
        let ram: Vec<u8> = program.iter().flat_map(|w| w.to_le_bytes()).collect();
        let read = |address: u16| ram.get(usize::from(address) - 0x100).copied().unwrap_or(0);
        asic.write(&mut video, 0x6805, 0x00);
        asic.write(&mut video, 0x6c00, 0x00);
        asic.write(&mut video, 0x6c01, 0x01);
        asic.write(&mut video, 0x6c0f, 0x01);
        let mut lines = 0;
        while asic.dma[0].enabled && lines < 20 {
            asic.start_dma_line();
            for us in 1..40 {
                if asic.dma_us.is_some() {
                    asic.dma_microsecond(us, &mut psg, read);
                }
            }
            lines += 1;
        }
        assert_eq!(psg.register(8), 0x0f);
        assert_eq!(psg.register(0), 0x42);
        assert_eq!(lines, 7, "LOAD, REPEAT, (LOAD, LOOP) twice, INT|STOP");
        assert!(asic.dma_interrupt());
        assert_eq!(asic.acknowledge(0x2000) & 7, 4, "channel 0's vector");
        assert!(
            !asic.dma_interrupt(),
            "cleared by the acknowledge, IVR bit 0 being clear"
        );
    }

    #[test]
    fn the_snapshot_chunk_gives_the_asic_back() {
        let mut asic = Asic::default();
        let mut video = AsicVideo::default();
        write_sequence(
            &mut asic,
            &[
                0xff, 0x00, 0xff, 0x77, 0xb3, 0x51, 0xa8, 0xd4, 0x62, 0x39, 0x9c, 0x46, 0x2b, 0x15,
                0x8a, 0xcd, 0xee,
            ],
        );
        video.unlocked = true;
        video.rmr2 = 0x18;
        for (address, value) in [
            (0x4001, 0x0c),
            (0x4f00, 0x03),
            (0x6008, 0x40),
            (0x6009, 0x01),
            (0x600c, 0x0f),
            (0x6402, 0x5a),
            (0x6403, 0x0f),
            (0x6800, 0x80),
            (0x6801, 0x20),
            (0x6802, 0x30),
            (0x6803, 0x40),
            (0x6804, 0x8c),
            (0x6805, 0x40),
            (0x6c04, 0x20),
            (0x6c05, 0x81),
            (0x6c06, 0x07),
            (0x6c0f, 0x02),
        ] {
            asic.write(&mut video, address, value);
        }
        asic.set_analogue_input(2, 0x21);
        let chunk = asic.snapshot_chunk(&video);
        assert_eq!(chunk.len(), SNAPSHOT_CHUNK_LEN);
        assert_eq!(
            chunk[0], 0x0c,
            "the first two pixels of sprite 0 in one byte"
        );

        let mut restored = Asic::default();
        let mut restored_video = AsicVideo::default();
        restored
            .restore_snapshot_chunk(&mut restored_video, &chunk)
            .unwrap();
        assert_eq!(restored.snapshot_chunk(&restored_video), chunk);
        assert!(restored.unlocked() && restored_video.unlocked);
        assert_eq!(restored.read(&restored_video, 0x4001), Some(0x0c));
        assert_eq!(restored.read(&restored_video, 0x6009), Some(0x01));
        assert_eq!(restored.read(&restored_video, 0x680a), Some(0x21));
        assert_eq!(restored.read(&restored_video, 0x6c00), Some(0x02));
        assert_eq!(restored.split(), Some((0x20, 0x3040)));
    }

    #[test]
    fn a_raster_interrupt_takes_dma_0s_vector_in_the_lower_half_of_8k() {
        let mut asic = Asic::default();
        let mut video = AsicVideo::default();
        asic.write(&mut video, 0x6805, 0x40);
        asic.note_ga_interrupt(true);
        assert_eq!(asic.acknowledge(0x2345), 0x46);
        assert_eq!(asic.acknowledge(0x1345), 0x44);
    }

    #[test]
    fn repeat_0_does_nothing_and_a_prescaler_write_restarts_the_pause_unit() {
        let mut asic = Asic::default();
        let mut video = AsicVideo::default();
        let mut psg = crate::psg::Psg::default();
        asic.write(&mut video, 0x6c0f, 0x01);
        asic.dma[0].address = 0x0102;
        asic.execute_dma(0, 0x2003, &mut psg); // REPEAT 3
        asic.dma[0].address = 0x0200;
        asic.execute_dma(0, 0x2000, &mut psg); // REPEAT 0
        assert_eq!(
            (asic.dma[0].loop_count, asic.dma[0].loop_address),
            (3, 0x0102),
            "REPEAT 0 is a NOP"
        );

        asic.write(&mut video, 0x6c02, 5);
        asic.execute_dma(0, 0x1002, &mut psg); // PAUSE 2
        asic.start_dma_line();
        asic.start_dma_line();
        assert_eq!(asic.dma[0].pause_hsyncs, 3);
        asic.write(&mut video, 0x6c02, 1);
        asic.start_dma_line();
        assert_eq!(
            (asic.dma[0].pause, asic.dma[0].pause_hsyncs),
            (1, 1),
            "the count in progress wrapped at once"
        );
    }

    /// Where the raster interrupt is asked for PRI 1 (C4, C9, C0), with HSYNC at `r2` for 14
    /// characters on a 64 character line.
    fn raster_interrupt_position(r2: u8) -> (u8, u8, u8) {
        use crate::chip::Chip;
        use crate::crtc::{Crtc, CrtcType};
        use crate::ga::{GateArray, GateArrayPin};
        let mut crtc = Crtc::from(CrtcType::Crtc3);
        for (register, value) in [
            (0, 63),
            (1, 40),
            (2, r2),
            (3, 0x8e),
            (4, 38),
            (6, 25),
            (7, 30),
            (9, 7),
        ] {
            crtc.do_write_register(register, value);
        }
        let mut ga = GateArray::default();
        ga.set_plus(true);
        ga.reset();
        let mut psg = crate::psg::Psg::default();
        let mut asic = Asic::default();
        let mut video = AsicVideo::default();
        asic.write(&mut video, 0x6800, 1);
        for _ in 0..40_000 {
            crtc.tick();
            asic.microsecond(&crtc, &mut ga, &mut psg, |_| 0);
            if !ga.get_pin(GateArrayPin::INTERRUPT) {
                let c = crtc.counters();
                return (c.vcc, c.vlc, c.hcc);
            }
        }
        panic!("no raster interrupt");
    }

    #[test]
    fn the_raster_interrupt_is_asked_where_line_and_hsync_meet() {
        // INT on C0=47: the Z80 halted there is in &0038 on C0=52, as in ACE-DL and AMSpiriT
        assert_eq!(
            raster_interrupt_position(46),
            (0, 1, 47),
            "just after the HSYNC start of line 1"
        );
        let (vcc, vlc, hcc) = raster_interrupt_position(51);
        assert!(
            (vcc, vlc) == (0, 1) && hcc < 4,
            "line 0's HSYNC running over into line 1: {hcc}"
        );
    }
}
