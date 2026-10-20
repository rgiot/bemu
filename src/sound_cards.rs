//! Sound expansions demos use: the PlayCity (two more AY chips and a Z80 CTC) and the
//! printer-port DAC (Digiblaster).
//!
//! Both work from their port's pins, as the real cards do.
//!
//! The PlayCity sits on the expansion port; as its documentation and the emulators that have it
//! (CPCEC, MAME) wire it:
//! - the CTC at &F880-&F883, clocked by CLK4, its /INT on the port's /INT (the IM 2 vector on the
//!   data bus during the interrupt acknowledge, RETI decoded from the bus) and channel 1's ZC/TO
//!   on /NMI;
//! - channel 0's ZC/TO, halved by a flip-flop, clocks the two YMZ294: a count of n of CLK4 gives
//!   4 MHz / n / 2 (2 MHz, CLK4 halved, while channel 0 does not run);
//! - channel 1's trigger is the CRTC's CURSOR: R14/R15 and R10/R11 place the NMIs on the screen;
//! - channel 2's ZC/TO triggers channel 3;
//! - the right AY: select at &F984, data at &F884; the left one: &F988 and &F888;
//! - &F8FF resets the card (the CTC and both AYs).
//!
//! The Digiblaster is an 8-bit DAC on the printer port: D0-D6 and /STROBE (bit 7 of the byte
//! written, inverted by the CPC) make an unsigned sample, so the byte written is a signed one.

use std::any::Any;

use crate::chip::Chip;
use crate::expansion_port::{ExpansionDevice, ExpansionPin, ExpansionPort, PrinterDevice, PrinterPin, PrinterPort};
use crate::psg::Psg;

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

/// What the CTC follows of the bus.
#[derive(Clone, Debug, Default)]
struct BusWatch {
    in_acknowledge: bool,
    /// The vector given during this acknowledge.
    vector: Option<u8>,
    last_opcode: u8
}

/// A Z80 CTC (Z84C30): four counter/timer channels.
#[derive(Clone, Debug, Default)]
pub struct Ctc {
    channels: [CtcChannel; 4],
    vector: u8,
    zc_to: [bool; 4],
    bus: BusWatch
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

    /// Whether channel `channel` counts (a time constant was given, and a triggered timer got
    /// its trigger).
    pub fn is_running(&self, channel: usize) -> bool {
        self.channels[channel].running
    }

    /// ZC/TO of channel `channel`: high for half a clock when the channel's count reaches 0
    /// (channel 3's is internal: the chip has no pin for it).
    pub fn zc_to(&self, channel: usize) -> bool {
        self.zc_to[channel]
    }

    /// An edge of the system clock: the ZC/TO pulses end; on a rising one the timers'
    /// prescalers count.
    pub fn clock_edge(&mut self, rising: bool) {
        self.zc_to = [false; 4];
        if !rising {
            return;
        }
        for (channel, ch) in self.channels.iter_mut().enumerate() {
            if !ch.running || ch.counter_mode() {
                continue;
            }
            ch.prescaler = ch.prescaler.saturating_sub(1);
            if ch.prescaler == 0 {
                ch.prescaler = ch.prescale();
                self.zc_to[channel] |= ch.count();
            }
        }
    }

    /// The level of channel `channel`'s CLK/TRG input: an active edge counts (counter mode)
    /// or starts a triggered timer.
    pub fn trigger(&mut self, channel: usize, level: bool) {
        let ch = &mut self.channels[channel];
        let previous = std::mem::replace(&mut ch.trigger, level);
        let edge = if ch.rising_edge() { level && !previous } else { !level && previous };
        if !edge {
            return;
        }
        if ch.counter_mode() && ch.running {
            self.zc_to[channel] |= ch.count();
        }
        else if !ch.counter_mode() && !ch.running && ch.control & 0x08 != 0 && ch.time_constant != 0 {
            ch.running = true;
            ch.prescaler = ch.prescale();
        }
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

    /// The CPU acknowledges the interrupt: the channel that asks is in service; its vector.
    pub fn acknowledge(&mut self) -> Option<u8> {
        let vector = self.interrupt()?;
        let ch = &mut self.channels[usize::from(vector >> 1 & 3)];
        ch.interrupt_pending = false;
        ch.in_service = true;
        Some(vector)
    }

    /// A RETI: the channel in service is done.
    pub fn return_from_interrupt(&mut self) {
        if let Some(ch) = self.channels.iter_mut().find(|ch| ch.in_service) {
            ch.in_service = false;
        }
    }

    /// The Z80 bus, as a Z80 peripheral watches it: during an interrupt acknowledge the CTC
    /// puts its vector on the data bus; the opcodes fetched (at the end of each M1) spell the
    /// RETI that ends the interrupt in service.
    pub fn watch_bus(&mut self, port: &mut ExpansionPort) {
        if port.interrupt_acknowledge() {
            if !self.bus.in_acknowledge {
                self.bus.in_acknowledge = true;
                self.bus.vector = self.acknowledge();
            }
            if let Some(vector) = self.bus.vector {
                port.drive_data(vector);
            }
        }
        if port.m1_ends() {
            if std::mem::take(&mut self.bus.in_acknowledge) {
                self.bus.vector = None;
            }
            else {
                let opcode = port.data();
                if self.bus.last_opcode == 0xed && opcode == 0x4d {
                    self.return_from_interrupt();
                }
                self.bus.last_opcode = opcode;
            }
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
    /// The flip-flop halving the AYs' clock.
    ay_clock: bool,
    /// The level of the source of [`Self::ay_clock`], for its edges.
    ay_clock_source: bool
}

impl Default for PlayCity {
    fn default() -> Self {
        Self { ctc: Ctc::default(), ays: [Psg::default(), Psg::default()], ay_clock: false, ay_clock_source: false }
    }
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
            0xf8ff => self.reset_card(),
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

    fn reset_card(&mut self) {
        self.ctc.reset();
        for ay in &mut self.ays {
            ay.reset();
        }
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

impl ExpansionDevice for PlayCity {
    fn tick(&mut self, port: &mut ExpansionPort) {
        // the CPU's accesses
        if let Some((address, value)) = port.io_write_started() {
            self.write(address, value);
        }
        if let Some(value) = port.io_read().and_then(|address| self.read(address)) {
            port.drive_data(value);
        }
        self.ctc.watch_bus(port);

        // the clocks: CLK4 is the CTC's system clock and channel 0's CLK/TRG
        let clk4 = port.get_pin(ExpansionPin::CLK4);
        if port.is_upfront(ExpansionPin::CLK4) || port.is_downfront(ExpansionPin::CLK4) {
            self.ctc.clock_edge(clk4);
            self.ctc.trigger(0, clk4);
        }
        self.ctc.trigger(1, port.get_pin(ExpansionPin::CURSOR));
        self.ctc.trigger(3, self.ctc.zc_to(2));

        // the AYs' clock: channel 0's ZC/TO (CLK4 while it does not count), halved
        let source = if self.ctc.is_running(0) { self.ctc.zc_to(0) } else { clk4 };
        if source && !self.ay_clock_source {
            self.ay_clock = !self.ay_clock;
            if self.ay_clock {
                for ay in &mut self.ays {
                    ay.clock();
                }
            }
        }
        self.ay_clock_source = source;

        if self.ctc.interrupt().is_some() {
            port.assert(ExpansionPin::Neg_INT);
        }
        if self.ctc.zc_to(1) {
            port.assert(ExpansionPin::Neg_NMI);
        }
    }

    fn reset(&mut self) {
        self.reset_card();
    }

    fn sound(&self) -> [f32; 2] {
        let [right, left] = self.levels();
        [left * 0.5, right * 0.5]
    }

    fn sound_chips(&mut self) -> Vec<(&'static str, &mut Psg)> {
        let [right, left] = &mut self.ays;
        vec![("right", right), ("left", left)]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The Digiblaster: an 8-bit DAC on the printer port.
#[derive(Clone, Debug, Default)]
pub struct Digiblaster {
    /// -1..1.
    sample: f32
}

impl Digiblaster {
    pub fn sample(&self) -> f32 {
        self.sample
    }
}

impl PrinterDevice for Digiblaster {
    fn update(&mut self, port: &PrinterPort) {
        let unsigned = port.data() | u8::from(port.get_pin(PrinterPin::Neg_STROBE)) << 7;
        self.sample = (f32::from(unsigned) - 128.0) / 128.0;
    }

    fn sound(&self) -> [f32; 2] {
        [self.sample * 0.5; 2]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::expansion_port::CpcSide;

    /// The bus at rest, CLK4 at `clk4`.
    fn idle(clk4: bool) -> CpcSide {
        CpcSide {
            mreq: true,
            m1: true,
            rfsh: true,
            iorq: true,
            rd: true,
            wr: true,
            halt: true,
            busak: true,
            romen: true,
            ramrd: true,
            clk4,
            ..Default::default()
        }
    }

    fn port_with_playcity() -> ExpansionPort {
        let mut port = ExpansionPort::default();
        port.plug(Box::new(PlayCity::default()));
        port
    }

    /// An OUT, as the port sees it: a few ticks of /IORQ and /WR low.
    fn out(port: &mut ExpansionPort, address: u16, value: u8) {
        for tick in 0..4 {
            port.tick_with(CpcSide { address, data: value, iorq: false, wr: false, ..idle(tick & 2 != 0) });
        }
        port.tick_with(idle(false));
    }

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
        for _ in 0..160 {
            ctc.clock_edge(true);
            pulses += usize::from(ctc.zc_to(2));
            ctc.clock_edge(false);
        }
        assert_eq!(pulses, 1);
        assert_eq!(ctc.interrupt(), Some(0x44));
        assert_eq!(ctc.acknowledge(), Some(0x44));
        assert_eq!(ctc.interrupt(), None, "in service");
        ctc.return_from_interrupt();
        assert_eq!(ctc.interrupt(), None);
    }

    #[test]
    fn the_ctc_answers_the_acknowledge_and_watches_for_reti() {
        let mut port = port_with_playcity();
        out(&mut port, 0xf880, 0x20); // vector &20
        out(&mut port, 0xf880, 0x85); // channel 0: timer, interrupts, a constant
        out(&mut port, 0xf880, 1);
        let mut tick = 0;
        while port.get_pin(ExpansionPin::Neg_INT) {
            port.tick_with(idle(tick & 2 != 0));
            tick += 1;
            assert!(tick < 1000, "no interrupt");
        }
        // the acknowledge: /M1 and /IORQ low, the vector on the bus
        port.tick_with(CpcSide { m1: false, iorq: false, rd: false, ..idle(false) });
        assert_eq!(port.driven_data(), Some(0x20));
        port.tick_with(idle(false));
        assert!(port.get_pin(ExpansionPin::Neg_INT), "in service: /INT released");
        assert!(port.device::<PlayCity>().unwrap().ctc.channels[0].in_service);
        // ED 4D fetched: RETI
        for opcode in [0xed, 0x4d] {
            port.tick_with(CpcSide { m1: false, mreq: false, rd: false, ..idle(false) });
            port.tick_with(CpcSide { data: opcode, ..idle(false) });
        }
        assert!(!port.device::<PlayCity>().unwrap().ctc.channels[0].in_service, "RETI ends it");
    }

    #[test]
    fn channel_1_counts_the_cursor_to_an_nmi() {
        let mut port = port_with_playcity();
        // counter mode, rising edges, a constant: every 4 cursors
        out(&mut port, 0xf881, 0x57);
        out(&mut port, 0xf881, 4);
        let mut nmis = 0;
        for tick in 0..16 * 64 * 16 {
            let cursor = tick % (64 * 16) < 16;
            port.tick_with(CpcSide { cursor, ..idle(tick & 2 != 0) });
            nmis += usize::from(port.is_downfront(ExpansionPin::Neg_NMI));
        }
        assert_eq!(nmis, 4);
    }

    #[test]
    fn the_ays_play_their_registers() {
        let mut port = port_with_playcity();
        // left AY: channel A's tone off, volume 15
        for (reg, value) in [(7u8, 0x3f), (8, 15)] {
            out(&mut port, 0xf988, reg);
            out(&mut port, 0xf888, value);
        }
        for tick in 0..64 * 16 {
            port.tick_with(idle(tick & 2 != 0));
        }
        let [right, left] = port.device::<PlayCity>().unwrap().levels();
        assert_eq!(right, 0.0);
        assert!(left > 0.2, "one channel of three at full volume: {left}");
    }

    #[test]
    fn the_digiblaster_takes_the_printer_port() {
        let mut printer = PrinterPort::default();
        printer.plug(Box::new(Digiblaster::default()));
        printer.latch(0x40);
        assert_eq!(printer.sound(), [0.25, 0.25]);
        printer.latch(0xc0);
        assert_eq!(printer.sound(), [-0.25, -0.25]);
    }
}
