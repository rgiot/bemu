//! Sound expansions demos use: the PlayCity (two more AY chips and a Z80 CTC) and the
//! printer-port DAC (Digiblaster).
//!
//! The PlayCity, as its documentation and the emulators that have it (CPCEC, MAME) wire it:
//! - the CTC at &F880-&F883, clocked by the CPC's 4 MHz, its interrupt on the CPC's /INT (with
//!   its IM 2 vector) and channel 1's zero count on /NMI;
//! - channel 0 makes the AY clock: a count of n gives 4 MHz / n, the YMZ294 halving it
//!   (2 MHz while channel 0 does not run);
//! - channel 1 counts the lines: its trigger is the CRTC's cursor signal, approximated here by
//!   HSYNC (as CPCEC does), which a raster NMI programs it for;
//! - channel 2's zero count triggers channel 3;
//! - the right AY: select at &F984, data at &F884; the left one: &F988 and &F888;
//! - &F8FF resets the card (the CTC and both AYs).
//!
//! The Digiblaster is an 8-bit DAC on the printer port (A12 low): the byte written is a signed
//! sample.

use crate::psg::Psg;

/// The CPC's CPU clock, the CTC's.
const CTC_CLOCK_HZ: u32 = 4_000_000;

#[derive(Clone, Debug, Default)]
struct CtcChannel {
    control: u8,
    /// 1-256 (a written 0 is 256).
    time_constant: u16,
    counter: u16,
    /// Clocks left before the next count, in timer mode.
    prescaler: u16,
    /// Counting (a time constant was given, and in triggered timer mode the trigger came).
    running: bool,
    /// The next byte written is the time constant.
    expects_constant: bool,
    interrupt_pending: bool,
    in_service: bool,
    /// The trigger input's level, for its edges.
    trigger: bool
}

impl CtcChannel {
    fn interrupt_enabled(&self) -> bool {
        self.control & 0x80 != 0
    }

    fn counter_mode(&self) -> bool {
        self.control & 0x40 != 0
    }

    fn prescale(&self) -> u16 {
        if self.control & 0x20 != 0 { 256 } else { 16 }
    }

    fn rising_edge(&self) -> bool {
        self.control & 0x10 != 0
    }

    /// The counter reached 0: reload, interrupt; true for the ZC/TO pulse.
    fn count(&mut self) -> bool {
        self.counter = self.counter.saturating_sub(1);
        if self.counter == 0 {
            self.counter = self.time_constant;
            if self.interrupt_enabled() {
                self.interrupt_pending = true;
            }
            return true;
        }
        false
    }
}

/// A Z80 CTC (Z84C30): four counter/timer channels.
#[derive(Clone, Debug, Default)]
pub struct Ctc {
    channels: [CtcChannel; 4],
    vector: u8
}

impl Ctc {
    pub fn write(&mut self, channel: usize, value: u8) {
        let ch = &mut self.channels[channel];
        if ch.expects_constant {
            ch.expects_constant = false;
            ch.time_constant = if value == 0 { 256 } else { u16::from(value) };
            if !ch.running && (ch.counter_mode() || ch.control & 0x08 == 0) {
                ch.counter = ch.time_constant;
                ch.prescaler = ch.prescale();
                ch.running = true;
            }
            else if !ch.running {
                // a timer started by its trigger: the constant waits
                ch.counter = ch.time_constant;
            }
            return;
        }
        if value & 1 == 0 {
            // an interrupt vector (written to channel 0): bits 7-3
            self.vector = value & 0xf8;
            return;
        }
        ch.control = value;
        if value & 0x02 != 0 {
            // software reset: stops, until a new time constant
            ch.running = false;
            ch.interrupt_pending = false;
        }
        if value & 0x80 == 0 {
            ch.interrupt_pending = false;
        }
        ch.expects_constant = value & 0x04 != 0;
    }

    /// The down-counter's value.
    pub fn read(&self, channel: usize) -> u8 {
        self.channels[channel].counter as u8
    }

    /// The channel's ZC/TO frequency in timer mode, or for a counter of a clock `input_hz`.
    fn output_hz(&self, channel: usize, input_hz: u32) -> Option<f64> {
        let ch = &self.channels[channel];
        if !ch.running || ch.time_constant == 0 {
            return None;
        }
        let divider = if ch.counter_mode() { 1 } else { u32::from(ch.prescale()) };
        Some(f64::from(input_hz) / f64::from(divider * u32::from(ch.time_constant)))
    }

    /// `clocks` cycles of the 4 MHz: the timers count. Returns the channels whose ZC/TO pulsed.
    fn clock(&mut self, clocks: u16) -> [bool; 4] {
        let mut pulses = [false; 4];
        for (channel, ch) in self.channels.iter_mut().enumerate() {
            if !ch.running || ch.counter_mode() {
                continue;
            }
            let mut left = clocks;
            while left > 0 {
                let step = left.min(ch.prescaler.max(1));
                ch.prescaler = ch.prescaler.saturating_sub(step);
                left -= step;
                if ch.prescaler == 0 {
                    ch.prescaler = ch.prescale();
                    pulses[channel] |= ch.count();
                }
            }
        }
        pulses
    }

    /// The level of channel `channel`'s CLK/TRG input: an active edge counts (counter mode)
    /// or starts a triggered timer. True for the ZC/TO pulse.
    fn trigger(&mut self, channel: usize, level: bool) -> bool {
        let ch = &mut self.channels[channel];
        let previous = std::mem::replace(&mut ch.trigger, level);
        let edge = if ch.rising_edge() { level && !previous } else { !level && previous };
        if !edge {
            return false;
        }
        if ch.counter_mode() && ch.running {
            return ch.count();
        }
        if !ch.counter_mode() && !ch.running && ch.control & 0x08 != 0 && ch.time_constant != 0 {
            ch.running = true;
            ch.prescaler = ch.prescale();
        }
        false
    }

    /// Whether the CTC pulls /INT: a pending interrupt with none of higher priority in
    /// service.
    pub fn interrupt(&self) -> Option<u8> {
        for (channel, ch) in self.channels.iter().enumerate() {
            if ch.in_service {
                return None;
            }
            if ch.interrupt_pending {
                return Some(self.vector | (channel as u8) << 1);
            }
        }
        None
    }

    /// The CPU acknowledged the interrupt: the channel that asked is in service.
    pub fn acknowledge(&mut self) {
        if let Some(ch) = self.channels.iter_mut().find(|ch| ch.interrupt_pending) {
            ch.interrupt_pending = false;
            ch.in_service = true;
        }
    }

    /// A RETI: the channel in service is done.
    pub fn return_from_interrupt(&mut self) {
        if let Some(ch) = self.channels.iter_mut().find(|ch| ch.in_service) {
            ch.in_service = false;
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The PlayCity card.
pub struct PlayCity {
    pub ctc: Ctc,
    /// Right then left.
    ays: [Psg; 2],
    /// AY clocks owed, in millionths of a clock.
    ay_phase: u64,
    hsync: bool
}

impl Default for PlayCity {
    fn default() -> Self {
        Self { ctc: Ctc::default(), ays: [Psg::default(), Psg::default()], ay_phase: 0, hsync: false }
    }
}

/// What the card asks of the CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CardLines {
    pub nmi: bool
}

impl PlayCity {
    /// A write of the CPU at `port`; false when not the card's.
    pub fn write(&mut self, port: u16, value: u8) -> bool {
        match port {
            0xf880..=0xf883 => self.ctc.write(usize::from(port & 3), value),
            0xf884 => {
                self.ays[0].bus_cycle(crate::psg::BusControl::Write, value);
                self.ays[0].bus_cycle(crate::psg::BusControl::Inactive, 0);
            },
            0xf888 => {
                self.ays[1].bus_cycle(crate::psg::BusControl::Write, value);
                self.ays[1].bus_cycle(crate::psg::BusControl::Inactive, 0);
            },
            0xf984 => {
                self.ays[0].bus_cycle(crate::psg::BusControl::LatchAddress, value);
            },
            0xf988 => {
                self.ays[1].bus_cycle(crate::psg::BusControl::LatchAddress, value);
            },
            0xf8ff => self.reset(),
            _ => return false
        }
        true
    }

    pub fn read(&self, port: u16) -> Option<u8> {
        match port {
            0xf880..=0xf883 => Some(self.ctc.read(usize::from(port & 3))),
            _ => None
        }
    }

    pub fn reset(&mut self) {
        self.ctc.reset();
        for ay in &mut self.ays {
            ay.reset();
        }
    }

    /// The AYs' clock: channel 0's output halved, 2 MHz when it does not run.
    pub fn ay_clock_hz(&self) -> f64 {
        self.ctc.output_hz(0, CTC_CLOCK_HZ).map_or(2_000_000.0, |hz| hz / 2.0)
    }

    /// One microsecond, `hsync` the CRTC's: the CTC counts, the AYs run. Returns the lines
    /// the card drives.
    pub fn tick_us(&mut self, hsync: bool) -> CardLines {
        let mut pulses = self.ctc.clock(4);
        // channel 1 counts the lines
        if hsync != self.hsync {
            self.hsync = hsync;
            pulses[1] |= self.ctc.trigger(1, hsync);
        }
        if pulses[2] {
            pulses[3] |= self.ctc.trigger(3, true);
            self.ctc.trigger(3, false);
        }
        // the AYs' own clock: `Psg::clock` is one cycle of a 1 MHz input
        self.ay_phase += (self.ay_clock_hz() * 1.0) as u64;
        while self.ay_phase >= 1_000_000 {
            self.ay_phase -= 1_000_000;
            for ay in &mut self.ays {
                ay.clock();
            }
        }
        CardLines { nmi: pulses[1] }
    }

    /// The two AYs' sound, right then left, 0..1 each.
    pub fn levels(&self) -> [f32; 2] {
        let mono = |ay: &Psg| ay.channel_levels().iter().sum::<f32>() / 3.0;
        [mono(&self.ays[0]), mono(&self.ays[1])]
    }

    pub fn ays_mut(&mut self) -> &mut [Psg; 2] {
        &mut self.ays
    }
}

/// The expansions plugged that make sound.
#[derive(Default)]
pub struct SoundCards {
    pub playcity: Option<Box<PlayCity>>,
    /// The Digiblaster's last sample, -1..1, or none when not plugged.
    pub dac: Option<f32>
}

impl SoundCards {
    pub fn is_empty(&self) -> bool {
        self.playcity.is_none() && self.dac.is_none()
    }

    /// A CPU write; true when a card took it.
    pub fn write(&mut self, port: u16, value: u8) -> bool {
        let mut taken = false;
        if let Some(dac) = &mut self.dac {
            if port & 0x1000 == 0 {
                *dac = f32::from(value as i8) / 128.0;
                taken = true;
            }
        }
        if let Some(playcity) = &mut self.playcity {
            taken |= playcity.write(port, value);
        }
        taken
    }

    pub fn read(&self, port: u16) -> Option<u8> {
        self.playcity.as_ref().and_then(|playcity| playcity.read(port))
    }

    /// One microsecond: the cards run; their stereo sound (to add to the CPC's) and lines.
    pub fn tick_us(&mut self, hsync: bool) -> ([f32; 2], CardLines) {
        let mut sound = [0.0f32; 2];
        let mut lines = CardLines::default();
        if let Some(playcity) = &mut self.playcity {
            lines = playcity.tick_us(hsync);
            let [right, left] = playcity.levels();
            sound[0] += left * 0.5;
            sound[1] += right * 0.5;
        }
        if let Some(dac) = self.dac {
            sound[0] += dac * 0.5;
            sound[1] += dac * 0.5;
        }
        (sound, lines)
    }

    /// The CTC's interrupt: its vector when it pulls /INT.
    pub fn interrupt(&self) -> Option<u8> {
        self.playcity.as_ref().and_then(|playcity| playcity.ctc.interrupt())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_ctc_timer_interrupts_with_its_vector() {
        let mut ctc = Ctc::default();
        ctc.write(0, 0x40); // vector &40
        // channel 2: timer, prescaler 16, interrupts, a constant follows
        ctc.write(2, 0x85);
        ctc.write(2, 10);
        assert_eq!(ctc.interrupt(), None);
        // 16 * 10 clocks
        let mut pulses = 0;
        for _ in 0..40 {
            pulses += usize::from(ctc.clock(4)[2]);
        }
        assert_eq!(pulses, 1);
        assert_eq!(ctc.interrupt(), Some(0x44));
        ctc.acknowledge();
        assert_eq!(ctc.interrupt(), None, "in service");
        ctc.return_from_interrupt();
        assert_eq!(ctc.interrupt(), None);
    }

    #[test]
    fn channel_0_sets_the_ay_clock() {
        let mut playcity = PlayCity::default();
        assert_eq!(playcity.ay_clock_hz(), 2_000_000.0);
        // counter mode, time constant 1: 4 MHz, halved
        playcity.write(0xf880, 0x47);
        playcity.write(0xf880, 1);
        assert_eq!(playcity.ay_clock_hz(), 2_000_000.0);
        playcity.write(0xf880, 0x47);
        playcity.write(0xf880, 2);
        assert_eq!(playcity.ay_clock_hz(), 1_000_000.0);
    }

    #[test]
    fn channel_1_counts_lines_to_an_nmi() {
        let mut playcity = PlayCity::default();
        // counter mode, rising edges, a constant: every 4 lines
        playcity.write(0xf881, 0x57);
        playcity.write(0xf881, 4);
        let mut nmis = 0;
        for us in 0..64 * 16 {
            nmis += usize::from(playcity.tick_us(us % 64 < 4).nmi);
        }
        assert_eq!(nmis, 4);
    }

    #[test]
    fn the_ays_play_their_registers() {
        let mut playcity = PlayCity::default();
        // left AY: channel A's tone off, volume 15
        for (reg, value) in [(7u8, 0x3f), (8, 15)] {
            playcity.write(0xf988, reg);
            playcity.write(0xf888, value);
        }
        for _ in 0..64 {
            playcity.tick_us(false);
        }
        let [right, left] = playcity.levels();
        assert_eq!(right, 0.0);
        assert!(left > 0.2, "one channel of three at full volume: {left}");
    }

    #[test]
    fn the_dac_takes_the_printer_port() {
        let mut cards = SoundCards { dac: Some(0.0), ..Default::default() };
        assert!(cards.write(0xef00, 0x40));
        assert_eq!(cards.tick_us(false).0, [0.25, 0.25]);
        assert!(!cards.write(0x7f00, 0x80), "the gate array's port");
    }
}
