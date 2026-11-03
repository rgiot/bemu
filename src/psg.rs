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
//! # Sound
//!
//! Tone, noise, envelope and the logarithmic DAC are emulated by the
//! [`ym2149`] crate. That crate is sample-oriented (it runs its generators
//! at master clock / 8 and averages them down to a host rate); here it is
//! built with a "sample rate" equal to that internal rate, so each
//! [`Ym2149::compute_next_sample`] call is exactly one step of the chip's
//! /8 prescaler and nothing is averaged inside the crate. The PSG's CLOCK
//! pin is fed by the gate array's 1MHz output (the same signal as the
//! CRTC's CCLK on the CPC) through [`Psg::set_clock_pin`], so every
//! period follows the datasheet formulas with fclock = 1MHz:
//!
//! - tone: f = fclock / (16 * TP), TP = 12 bits of R0-R5;
//! - noise: shift clock f = fclock / (16 * NP), NP = 5 bits of R6;
//! - envelope: one full ramp lasts 256 * EP / fclock, EP = R11/R12 (the
//!   YM2149 splits it into 32 steps of 8 * EP clocks, the AY into 16 of
//!   16 * EP - see "Known differences" below).
//!
//! Every write cycle seen on the bus is forwarded to the chip once (on the
//! leading edge of the write pulse, or again if the data/register changes
//! while the pulse is held): `Motherboard` repeats the same `bus_cycle` on
//! every tick of an I/O cycle, and forwarding R13 more than once would
//! restart the envelope on each of them. Registers 14/15 (I/O ports) are
//! never forwarded: they have no effect on sound. Reads keep being served
//! from this module's own register copy, unmasked, exactly as before.
//!
//! The per-channel levels (0.0 = silent, 1.0 = loudest envelope step)
//! change at most once per 8us. When an [`AudioConfig`] is set, they are
//! mixed and resampled to the requested rate by exact box-filter
//! integration of the 1MHz stream (see [`AudioConfig`]), then collected
//! with [`Psg::take_samples`].
//!
//! ## Known differences with the CPC's AY-3-8912
//!
//! The emulation uses the YM2149's DAC: 32 envelope steps of 1.5dB (the
//! AY has 16 of 3dB, so its envelope is coarser) and fixed volumes mapped
//! onto even YM steps (3dB per volume unit, as on the AY). The crate also
//! halves the amplitude of a channel whose tone period is 0 or 1.

use ym2149::{Ym2149, Ym2149Backend};

/// The bits of each register the AY-3-8912 keeps (the others read back 0): 12-bit tone periods,
/// 5-bit noise period and volumes, 4-bit envelope shape (Kevin Thacker's acid test "reg
/// write/read mask test", on a CPC).
const REGISTER_MASKS: [u8; 16] = [0xff, 0x0f, 0xff, 0x0f, 0xff, 0x0f, 0x1f, 0xff, 0x1f, 0x1f, 0x1f, 0xff, 0xff, 0x0f, 0xff, 0xff];

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
/// Registers 0-13 drive the sound generators; 14 and 15 are the I/O ports.
const NB_SOUND_REGISTERS: u8 = 14;

/// Frequency of the PSG's CLOCK pin on the CPC (the gate array's 1MHz).
pub const PSG_CLOCK_HZ: u32 = 1_000_000;
/// The generators advance once every 8 input clocks.
pub const CLOCKS_PER_STEP: u8 = 8;
const STEP_RATE_HZ: u32 = PSG_CLOCK_HZ / CLOCKS_PER_STEP as u32;

/// What [`Psg::take_samples`] produces.
///
/// Resampling: the channel levels are a 1MHz piecewise-constant signal;
/// each output sample is the exact average of that signal over its own
/// 1/`sample_rate` window (box filter, the fractional microsecond at each
/// window edge is split between the two samples it straddles). This keeps
/// the energy of tones above the output Nyquist frequency from folding
/// back at full strength and is drift-free (integer phase accumulator).
///
/// Mixing:
/// - mono: (A + B + C) / 3, what reaches the CPC's internal speaker;
/// - stereo (interleaved left, right): the CPC's stereo socket wires
///   channel A to the left, C to the right and B to both sides through a
///   resistor network; this is modelled as left = (A + B/2) / 1.5 and
///   right = (C + B/2) / 1.5 (an approximation of the resistor ratios).
///
/// Without `dc_filter` samples are in 0.0..=1.0 (the DAC output, silence
/// at 0). With it, a first-order high-pass at [`DC_FILTER_CUTOFF_HZ`]
/// (the role of the output coupling capacitor) centres them around 0.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub stereo: bool,
    pub dc_filter: bool
}

impl AudioConfig {
    pub fn mono(sample_rate: u32) -> Self {
        Self { sample_rate, stereo: false, dc_filter: true }
    }

    pub fn stereo(sample_rate: u32) -> Self {
        Self { sample_rate, stereo: true, dc_filter: true }
    }

    pub fn nb_channels(&self) -> usize {
        if self.stereo { 2 } else { 1 }
    }
}

/// Cut-off frequency of the optional DC-blocking high-pass filter.
pub const DC_FILTER_CUTOFF_HZ: f64 = 10.0;

/// First-order DC blocker: y[n] = x[n] - x[n-1] + r * y[n-1].
#[derive(Debug, Clone, Default)]
struct DcBlocker {
    previous_input: f64,
    previous_output: f64
}

impl DcBlocker {
    fn process(&mut self, input: f64, r: f64) -> f64 {
        let output = input - self.previous_input + r * self.previous_output;
        self.previous_input = input;
        self.previous_output = output;
        output
    }
}

/// Mixer + resampler from the 1MHz level stream to `config.sample_rate`.
#[derive(Debug, Clone)]
struct AudioOutput {
    config: AudioConfig,
    /// Time accumulated in the sample being built, in units of
    /// 1 / (PSG_CLOCK_HZ * sample_rate) seconds: each microsecond adds
    /// `sample_rate`, a sample is complete at `PSG_CLOCK_HZ`.
    phase: u32,
    /// Level integrated over the current window, in the same unit.
    sums: [f64; 2],
    dc_filter_r: f64,
    dc_blockers: [DcBlocker; 2],
    samples: Vec<f32>
}

impl AudioOutput {
    fn new(config: AudioConfig) -> Self {
        assert!(
            (1..=PSG_CLOCK_HZ).contains(&config.sample_rate),
            "sample rate must be between 1Hz and the PSG's 1MHz clock, got {}",
            config.sample_rate
        );
        let dc_filter_r =
            (-2.0 * std::f64::consts::PI * DC_FILTER_CUTOFF_HZ / config.sample_rate as f64).exp();
        Self {
            config,
            phase: 0,
            sums: [0.0; 2],
            dc_filter_r,
            dc_blockers: Default::default(),
            samples: Vec::new()
        }
    }

    fn mix(&self, [a, b, c]: [f32; 3]) -> [f64; 2] {
        let (a, b, c) = (a as f64, b as f64, c as f64);
        if self.config.stereo {
            [(a + b / 2.0) / 1.5, (c + b / 2.0) / 1.5]
        }
        else {
            let mono = (a + b + c) / 3.0;
            [mono, mono]
        }
    }

    /// Integrate one microsecond of `levels`, the AY's, plus `extra` (left, right), the sound
    /// cards' already mixed, the AY taking `gain` of the whole.
    fn push_microsecond(&mut self, levels: [f32; 3], extra: [f32; 2], gain: f32) {
        let mut frame = self.mix(levels);
        if gain != 1.0 || extra != [0.0; 2] {
            let gain = f64::from(gain);
            if self.config.stereo {
                frame = [frame[0] * gain + f64::from(extra[0]), frame[1] * gain + f64::from(extra[1])];
            }
            else {
                let extra = f64::from(extra[0] + extra[1]) / 2.0;
                frame = [frame[0] * gain + extra, frame[1] * gain + extra];
            }
        }
        let rate = self.config.sample_rate;
        self.phase += rate;
        if self.phase < PSG_CLOCK_HZ {
            for (sum, level) in self.sums.iter_mut().zip(frame) {
                *sum += level * rate as f64;
            }
            return;
        }

        // This microsecond straddles the end of the window: its first part
        // completes the sample, the rest starts the next one.
        let overflow = self.phase - PSG_CLOCK_HZ;
        let inside = (rate - overflow) as f64;
        #[allow(clippy::needless_range_loop)] // the channel indexes four arrays
        for channel in 0..self.config.nb_channels() {
            let sum = self.sums[channel] + frame[channel] * inside;
            let mut sample = sum / PSG_CLOCK_HZ as f64;
            if self.config.dc_filter {
                sample = self.dc_blockers[channel].process(sample, self.dc_filter_r);
            }
            self.samples.push(sample as f32);
        }
        for (sum, level) in self.sums.iter_mut().zip(frame) {
            *sum = level * overflow as f64;
        }
        self.phase = overflow;
    }
}

pub struct Psg {
    registers: [u8; NB_REGISTERS],
    selected_register: Option<u8>,
    /// The (register, value) already forwarded to the chip during the
    /// write pulse currently on the bus - `None` outside of a write pulse.
    forwarded_write: Option<(u8, u8)>,
    chip: Ym2149,
    clock_pin: bool,
    /// Input clocks since the last generator step (0..CLOCKS_PER_STEP).
    prescaler: u8,
    /// Output level of channels A, B, C (0.0..=1.0) after the last step.
    levels: [f32; 3],
    audio: Option<AudioOutput>,
    /// The registers once per frame, when logged (a YM export).
    frame_log: Option<FrameLog>,
    /// The sound cards' sound for the coming microsecond (left, right), mixed with the AY's.
    extra: [f32; 2],
    /// The AY's share of the mix: 1 alone, less when sound cards add theirs.
    gain: f32
}

/// The registers frame by frame, as a YM file keeps them.
#[derive(Default)]
struct FrameLog {
    frames: Vec<[u8; 16]>,
    /// R13 written since the last frame: the envelope restarts (a YM file marks "not written"
    /// with &FF).
    r13_written: bool
}

impl Default for Psg {
    fn default() -> Self {
        let mut psg = Self {
            registers: [0; NB_REGISTERS],
            selected_register: None,
            forwarded_write: None,
            // One "sample" per internal step: see the module doc comment.
            chip: Ym2149::with_clocks(PSG_CLOCK_HZ, STEP_RATE_HZ),
            clock_pin: false,
            prescaler: 0,
            levels: [0.0; 3],
            audio: None,
            frame_log: None,
            extra: [0.0; 2],
            gain: 1.0
        };
        // The datasheet's reset clears every register; the crate's own
        // reset sets R7 to 0x3F instead. Keep the chip in line with
        // `registers`.
        for reg in 0..NB_SOUND_REGISTERS {
            psg.chip.write_register(reg, 0);
        }
        psg
    }
}

impl Psg {
    /// Reset the chip (the CPC's RESET line). The audio output settings and
    /// the samples not taken yet are kept: the host's sound stream goes on.
    pub fn reset(&mut self) {
        let audio = self.audio.take();
        let frame_log = self.frame_log.take();
        let gain = self.gain;
        *self = Self::default();
        self.audio = audio;
        self.frame_log = frame_log;
        self.gain = gain;
    }

    /// The sound cards' sound for the next microsecond (left, right), and the AY's share of the
    /// mix.
    pub fn set_extra_sound(&mut self, extra: [f32; 2], gain: f32) {
        self.extra = extra;
        self.gain = gain;
    }

    /// Keep (`true`) or not the registers once per frame, for [`Self::take_logged_frames`].
    pub fn log_frames(&mut self, log: bool) {
        self.frame_log = log.then(FrameLog::default);
    }

    pub fn logs_frames(&self) -> bool {
        self.frame_log.is_some()
    }

    /// A frame begins (the CRTC's VSYNC): note the registers, R13 as &FF when not written
    /// since the last frame.
    pub fn log_frame(&mut self) {
        if let Some(log) = &mut self.frame_log {
            let mut frame = [0u8; 16];
            frame[..14].copy_from_slice(&self.registers[..14]);
            if !std::mem::take(&mut log.r13_written) {
                frame[13] = 0xff;
            }
            log.frames.push(frame);
        }
    }

    /// The frames noted since the last call.
    pub fn take_logged_frames(&mut self) -> Vec<[u8; 16]> {
        self.frame_log.as_mut().map(|log| std::mem::take(&mut log.frames)).unwrap_or_default()
    }

    /// The register as the CPU reads it back.
    pub fn register(&self, idx: u8) -> u8 {
        self.registers[idx as usize & 0x0f]
    }

    /// Set a register directly (snapshot loading, register 14 mirroring
    /// the keyboard): sound registers are forwarded to the chip too.
    /// Select the register the next access reaches (a snapshot's), as a latch-address would.
    pub fn select_register(&mut self, idx: u8) {
        self.selected_register = Some(idx & 0x0f);
    }

    pub fn set_register(&mut self, idx: u8, value: u8) {
        let idx = idx & 0x0f;
        // the AY-3-8912 keeps only the bits it uses: the others read back 0
        let value = value & REGISTER_MASKS[idx as usize];
        self.registers[idx as usize] = value;
        if idx == 13 {
            if let Some(log) = &mut self.frame_log {
                log.r13_written = true;
            }
        }
        if idx < NB_SOUND_REGISTERS {
            self.chip.write_register(idx, value);
        }
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
        if control != BusControl::Write {
            self.forwarded_write = None;
        }
        match control {
            BusControl::Inactive => None,
            BusControl::LatchAddress => {
                // The chip answers an address only when DA7-DA4 are 0: with
                // any of them set it is deselected, and the data cycles that
                // follow reach no register (music players rely on it, e.g.
                // to park a write nowhere) - they must not hit register
                // `address & 0x0f`.
                self.selected_register = (data_bus & 0xf0 == 0).then_some(data_bus & 0x0f);
                None
            },
            BusControl::Write => {
                if let Some(reg) = self.selected_register {
                    // Same pulse, same write: already done.
                    if self.forwarded_write != Some((reg, data_bus)) {
                        self.forwarded_write = Some((reg, data_bus));
                        self.set_register(reg, data_bus);
                    }
                }
                None
            },
            BusControl::Read => self.selected_register.map(|reg| self.register(reg))
        }
    }

    /// Drive the CLOCK pin: the chip advances on each rising edge.
    pub fn set_clock_pin(&mut self, level: bool) {
        let rising = level && !self.clock_pin;
        self.clock_pin = level;
        if rising {
            self.clock();
        }
    }

    /// One cycle of the PSG's 1MHz input clock.
    pub fn clock(&mut self) {
        self.prescaler += 1;
        if self.prescaler == CLOCKS_PER_STEP {
            self.prescaler = 0;
            self.step();
        }
        if let Some(audio) = &mut self.audio {
            audio.push_microsecond(self.levels, self.extra, self.gain);
        }
    }

    /// One step of the tone/noise/envelope generators (every 8 clocks).
    fn step(&mut self) {
        // The mixed, DC-filtered sample the crate returns is not used: the
        // per-channel outputs are needed for stereo, and the resampling and
        // filtering are done in `AudioOutput`.
        let _ = self.chip.compute_next_sample();
        // Bipolar (+level when the channel's tone/noise gate is high,
        // -level when it is low) - the DAC itself outputs 0 when low.
        let (a, b, c) = self.chip.get_channel_outputs();
        let mut levels = [a.max(0.0), b.max(0.0), c.max(0.0)];

        // The crate halves a channel whose tone period is 0 or 1 even when
        // its tone is disabled in R7. A disabled tone does not reach the
        // channel's output (datasheet mixer: the volume/envelope is output
        // as is), which is how sample players use a channel as a DAC -
        // often leaving its period at 0.
        let mixer = self.register(REG_MIXER);
        for (channel, level) in levels.iter_mut().enumerate() {
            let tone_disabled = mixer & (1 << channel) != 0;
            if tone_disabled && self.tone_period(channel) <= 1 {
                *level = (*level * 2.0).min(1.0);
            }
        }
        self.levels = levels;
    }

    /// The 12-bit tone period of `channel` (0 = A).
    pub fn tone_period(&self, channel: usize) -> u16 {
        let fine = self.registers[channel * 2] as u16;
        let coarse = (self.registers[channel * 2 + 1] & 0x0f) as u16;
        (coarse << 8) | fine
    }

    /// The current output of channels A, B and C: 0.0 is silence, 1.0 the
    /// loudest envelope step (YM2149 logarithmic DAC).
    pub fn channel_levels(&self) -> [f32; 3] {
        self.levels
    }

    /// The sound chip emulation itself.
    pub fn chip(&self) -> &Ym2149 {
        &self.chip
    }

    /// Start (`Some`) or stop (`None`) producing audio samples. Changing
    /// the configuration drops the samples not taken yet.
    pub fn set_audio_output(&mut self, config: Option<AudioConfig>) {
        self.audio = config.map(AudioOutput::new);
    }

    pub fn audio_config(&self) -> Option<AudioConfig> {
        self.audio.as_ref().map(|audio| audio.config)
    }

    /// The samples produced since the last call, as f32 (interleaved
    /// left/right in stereo). Empty when no audio output is configured.
    pub fn take_samples(&mut self) -> Vec<f32> {
        self.audio
            .as_mut()
            .map(|audio| std::mem::take(&mut audio.samples))
            .unwrap_or_default()
    }

    /// Number of samples waiting in [`Self::take_samples`] (counting each
    /// channel of a stereo frame).
    pub fn pending_samples(&self) -> usize {
        self.audio.as_ref().map_or(0, |audio| audio.samples.len())
    }

    /// Take over the audio output (settings, samples not taken yet) of a
    /// PSG being replaced - e.g. when a whole machine is rebuilt to reset
    /// it - so the host's sound stream is not interrupted.
    pub fn take_audio_output_from(&mut self, previous: &mut Psg) {
        self.audio = previous.audio.take();
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
        // the noise period keeps 5 bits
        assert_eq!(psg.register(6), 0x19);

        assert_eq!(psg.bus_cycle(BusControl::Read, 0xAA /* ignored */), Some(0x19));
    }

    #[test]
    fn latch_address_with_upper_bits_set_deselects_the_chip() {
        let mut psg = Psg::default();
        psg.bus_cycle(BusControl::LatchAddress, 0x0f);
        assert_eq!(psg.selected_register(), Some(0x0f));
        psg.bus_cycle(BusControl::LatchAddress, 0x10);
        assert_eq!(psg.selected_register(), None);
    }

    #[test]
    fn writes_after_a_deselecting_latch_reach_no_register() {
        let mut psg = Psg::default();
        psg.bus_cycle(BusControl::LatchAddress, 0x10);
        psg.bus_cycle(BusControl::Inactive, 0x10);
        psg.bus_cycle(BusControl::Write, 0xEF);
        assert_eq!(psg.register(0), 0);
        psg.bus_cycle(BusControl::Inactive, 0);
        psg.bus_cycle(BusControl::LatchAddress, 0x00);
        psg.bus_cycle(BusControl::Inactive, 0);
        psg.bus_cycle(BusControl::Write, 0x12);
        assert_eq!(psg.register(0), 0x12);
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

    // ------------------------------------------------------------ sound

    /// Write a register the way the CPC firmware does it through the PPI:
    /// latch the address, release, write, release - each bus state held
    /// for several bus cycles, as `Motherboard` repeats them every tick.
    fn write(psg: &mut Psg, reg: u8, value: u8) {
        for (control, data) in [
            (BusControl::LatchAddress, reg),
            (BusControl::Inactive, reg),
            (BusControl::Write, value),
            (BusControl::Inactive, value)
        ] {
            for _ in 0..4 {
                psg.bus_cycle(control, data);
            }
        }
    }

    fn write_all(psg: &mut Psg, writes: &[(u8, u8)]) {
        for &(reg, value) in writes {
            write(psg, reg, value);
        }
    }

    /// Clock the PSG for `us` microseconds, through its CLOCK pin, keeping
    /// the levels seen after each microsecond.
    fn run(psg: &mut Psg, us: usize) -> Vec<[f32; 3]> {
        (0..us)
            .map(|_| {
                psg.set_clock_pin(true);
                psg.set_clock_pin(false);
                psg.channel_levels()
            })
            .collect()
    }

    fn channel(levels: &[[f32; 3]], channel: usize) -> Vec<f32> {
        levels.iter().map(|l| l[channel]).collect()
    }

    /// Before the first generator step (8 clocks) the levels are still the
    /// reset ones: edges there say nothing about the waveform.
    const FIRST_STEP: usize = CLOCKS_PER_STEP as usize;

    fn rising_edges(levels: &[f32]) -> Vec<usize> {
        (FIRST_STEP + 1..levels.len())
            .filter(|&i| levels[i] > 0.0 && levels[i - 1] == 0.0)
            .collect()
    }

    /// R7 with only the given tone (bits 0-2) / noise (bits 3-5) enabled
    /// - the datasheet's enable bits are active low.
    fn mixer(tones: u8, noises: u8) -> u8 {
        !(tones | (noises << 3)) & 0x3f
    }

    #[test]
    fn clock_pin_advances_on_rising_edges_only() {
        let mut psg = Psg::default();
        write_all(&mut psg, &[(7, mixer(0b001, 0)), (0, 2), (8, 15)]);
        // TP = 2: the output toggles every 2 steps of 8 clocks, i.e. 10
        // times in 160 clocks.
        run(&mut psg, FIRST_STEP);
        let mut toggles = 0;
        let mut previous = psg.channel_levels()[0];
        for _ in 0..16 * 10 {
            psg.set_clock_pin(true);
            psg.set_clock_pin(true); // still high: not an edge
            psg.set_clock_pin(false);
            let level = psg.channel_levels()[0];
            toggles += (level != previous) as usize;
            previous = level;
        }
        assert_eq!(toggles, 10);
    }

    #[test]
    fn tone_period_is_16_tp_clocks() {
        // f = fclock / (16 * TP), fclock = 1MHz: one period lasts 16*TP us.
        for tp in [2u16, 3, 100, 0x11c, 0xfff] {
            let mut psg = Psg::default();
            write_all(&mut psg, &[
                (7, mixer(0b001, 0)),
                (0, (tp & 0xff) as u8),
                (1, (tp >> 8) as u8),
                (8, 15)
            ]);
            let levels = run(&mut psg, 16 * tp as usize * 6 + 64);
            let a = channel(&levels, 0);
            let edges = rising_edges(&a);
            assert!(edges.len() >= 5, "TP={tp}: {} edges", edges.len());
            for pair in edges.windows(2) {
                assert_eq!(pair[1] - pair[0], 16 * tp as usize, "TP={tp}");
            }
            // 50% duty cycle: high for 8*TP us.
            let high = a[edges[0]..edges[1]].iter().filter(|&&l| l > 0.0).count();
            assert_eq!(high, 8 * tp as usize, "TP={tp}");
            // The other channels are disabled with volume 0: silent.
            assert!(levels.iter().all(|l| l[1] == 0.0 && l[2] == 0.0));
        }
    }

    #[test]
    fn tone_period_uses_only_the_low_nibble_of_the_coarse_register() {
        let mut psg = Psg::default();
        write_all(&mut psg, &[(7, mixer(0b010, 0)), (2, 0x00), (3, 0xf1), (9, 15)]);
        let edges = rising_edges(&channel(&run(&mut psg, 16 * 0x100 * 4), 1));
        assert_eq!(edges[1] - edges[0], 16 * 0x100);
    }

    /// The level a channel outputs as a plain DAC (tone and noise disabled
    /// in R7: the datasheet's mixer then passes the amplitude as is). The
    /// tone periods are kept above 1 so the crate's own table is measured,
    /// not the half-amplitude workaround of `step`.
    fn dac_level(volume_register: u8) -> [f32; 3] {
        let mut psg = Psg::default();
        write_all(&mut psg, &[
            (0, 0xff),
            (2, 0xff),
            (4, 0xff),
            (7, mixer(0, 0)),
            (8, volume_register),
            (9, volume_register),
            (10, volume_register)
        ]);
        *run(&mut psg, 16).last().unwrap()
    }

    #[test]
    fn fixed_volume_follows_the_logarithmic_table() {
        // Datasheet DAC: volume 0 is silent, each unit below 15 is -3dB,
        // i.e. a factor 1/sqrt(2) in amplitude.
        assert_eq!(dac_level(0), [0.0; 3]);
        let loudest = dac_level(15)[0];
        assert!(loudest > 0.5, "{loudest}");
        let mut previous = 0.0;
        for volume in 1..=15u8 {
            let level = dac_level(volume);
            assert!(level[0] == level[1] && level[1] == level[2]);
            let expected = loudest * 2f32.powf(-((15 - volume) as f32) / 2.0);
            assert!(
                (level[0] / expected - 1.0).abs() < 0.02,
                "volume {volume}: {} instead of {expected}",
                level[0]
            );
            assert!(level[0] > previous);
            previous = level[0];
        }
        // Bits 5-7 of the amplitude registers are not implemented.
        assert_eq!(dac_level(0xef), dac_level(0x0f));
    }

    #[test]
    fn a_disabled_tone_with_period_0_does_not_lower_the_dac_level() {
        let mut psg = Psg::default();
        write_all(&mut psg, &[(7, mixer(0, 0)), (8, 15), (0, 0xff)]);
        let with_period = run(&mut psg, 16)[15][0];
        write(&mut psg, 0, 0);
        let without_period = run(&mut psg, 16)[15][0];
        assert!((with_period - without_period).abs() < 1e-3);
    }

    #[test]
    fn noise_shifts_every_16_np_clocks() {
        for np in [1u8, 7, 31] {
            let mut psg = Psg::default();
            write_all(&mut psg, &[(7, mixer(0, 0b001)), (6, np | 0xe0 /* 5 bits */), (8, 15)]);
            let shifts = 4000;
            let a = channel(&run(&mut psg, 16 * np as usize * shifts), 0);
            let transitions: Vec<usize> = (1..a.len()).filter(|&i| a[i] != a[i - 1]).collect();
            for &t in &transitions {
                assert_eq!((t - transitions[0]) % (16 * np as usize), 0, "NP={np}");
            }
            // A pseudo-random bit changes on about half the shifts and is
            // high about half of the time.
            let ratio = transitions.len() as f64 / shifts as f64;
            assert!((0.35..0.65).contains(&ratio), "NP={np}: {ratio}");
            let high = a.iter().filter(|&&l| l > 0.0).count() as f64 / a.len() as f64;
            assert!((0.4..0.6).contains(&high), "NP={np}: {high}");
            // ... and is not a short periodic pattern.
            let first = &a[..a.len() / 4];
            assert_ne!(first, &a[a.len() / 4..a.len() / 2]);
        }
    }

    #[test]
    fn noise_and_tone_are_anded_by_the_mixer() {
        let mut psg = Psg::default();
        write_all(&mut psg, &[(7, mixer(0b001, 0b001)), (0, 50), (6, 3), (8, 15)]);
        let mut tone_only = Psg::default();
        write_all(&mut tone_only, &[(7, mixer(0b001, 0)), (0, 50), (8, 15)]);
        let both = channel(&run(&mut psg, 20_000), 0);
        let tone = channel(&run(&mut tone_only, 20_000), 0);
        // High only while both the tone and the noise are high.
        assert!(both.iter().zip(&tone).all(|(b, t)| *b == 0.0 || *t > 0.0));
        let high = |levels: &[f32]| levels.iter().filter(|&&l| l > 0.0).count();
        assert!(high(&both) < high(&tone) * 3 / 4);
        assert!(high(&both) > high(&tone) / 4);
    }

    /// Envelope step (0-31) from a channel level: the YM2149 DAC has 32
    /// steps of 1.5dB (factor 2^(1/4)), step 0 being silent.
    fn envelope_step(level: f32) -> u8 {
        if level <= 0.0 {
            0
        }
        else {
            (31.0 + 4.0 * level.log2()).round() as u8
        }
    }

    /// The envelope shapes as drawn in the datasheet, from the four bits of
    /// R13 (CONTinue, ATTack, ALTernate, HOLD), at step `step` (32 steps per
    /// ramp) after R13 was written.
    fn reference_envelope(shape: u8, step: usize) -> u8 {
        let (cont, att, alt, hold) =
            (shape & 8 != 0, shape & 4 != 0, shape & 2 != 0, shape & 1 != 0);
        let cycle = step / 32;
        let position = (step % 32) as u8;
        let ramp = |up: bool| if up { position } else { 31 - position };
        if cycle == 0 {
            ramp(att)
        }
        else if !cont {
            0
        }
        else if hold {
            // Holds the end of the first ramp, inverted when alternating.
            if att ^ alt { 31 } else { 0 }
        }
        else {
            ramp(att ^ (alt && cycle % 2 == 1))
        }
    }

    #[test]
    fn envelope_shapes_follow_the_datasheet() {
        for shape in 0..16u8 {
            let mut psg = Psg::default();
            write_all(&mut psg, &[(7, mixer(0, 0)), (8, 0x10), (11, 1), (12, 0), (13, shape)]);
            // EP = 1: one step every 8 clocks. Sample once per step.
            let steps: Vec<u8> = (0..4 * 32)
                .map(|_| envelope_step(run(&mut psg, 8)[7][0]))
                .collect();
            // The first step is taken during the first sampled period.
            let expected: Vec<u8> = (1..=steps.len()).map(|s| reference_envelope(shape, s)).collect();
            assert_eq!(steps, expected, "shape {shape:#x}");
        }
    }

    #[test]
    fn envelope_ramp_lasts_256_ep_clocks() {
        for ep in [1u16, 3, 0x123] {
            let mut psg = Psg::default();
            write_all(&mut psg, &[
                (7, mixer(0, 0)),
                (8, 0x10),
                (11, (ep & 0xff) as u8),
                (12, (ep >> 8) as u8),
                (13, 0x08) // repeated falling sawtooth
            ]);
            let steps: Vec<u8> = channel(&run(&mut psg, 256 * ep as usize * 4 + 16), 0)
                .into_iter()
                .map(envelope_step)
                .collect();
            // Jumps from the bottom of a ramp back to the top.
            let restarts: Vec<usize> =
                (FIRST_STEP + 1..steps.len()).filter(|&i| steps[i] == 31 && steps[i - 1] == 0).collect();
            assert_eq!(restarts.len(), 4, "EP={ep}");
            for pair in restarts.windows(2) {
                assert_eq!(pair[1] - pair[0], 256 * ep as usize, "EP={ep}");
            }
        }
    }

    #[test]
    fn a_write_pulse_held_over_several_clocks_restarts_the_envelope_once() {
        let mut psg = Psg::default();
        write_all(&mut psg, &[(7, mixer(0, 0)), (8, 0x10), (11, 1), (12, 0)]);
        for _ in 0..4 {
            psg.bus_cycle(BusControl::LatchAddress, 13);
        }
        psg.bus_cycle(BusControl::Inactive, 13);
        // A long write pulse (BDIR held high), seen on every 1/16us tick as
        // `Motherboard` does, while the chip keeps being clocked.
        let mut steps = Vec::new();
        for _ in 0..8 * 20 {
            for _ in 0..16 {
                psg.bus_cycle(BusControl::Write, 0x08);
            }
            psg.clock();
            steps.push(envelope_step(psg.channel_levels()[0]));
        }
        psg.bus_cycle(BusControl::Inactive, 0x08);
        // The ramp went on during the pulse instead of being held at 31.
        assert_eq!(*steps.last().unwrap(), 31 - 20);
        // A new pulse with the same value is a new write: restart.
        write(&mut psg, 13, 0x08);
        assert_eq!(envelope_step(run(&mut psg, 8)[7][0]), 30);
    }

    #[test]
    fn i_o_port_registers_do_not_reach_the_sound_chip() {
        let mut psg = Psg::default();
        write_all(&mut psg, &[(14, 0x12), (15, 0x34)]);
        assert_eq!(psg.register(14), 0x12);
        assert_eq!(psg.register(15), 0x34);
        assert_eq!(psg.chip().read_register(14), 0);
    }

    #[test]
    fn register_reads_keep_only_the_bits_the_ay_uses() {
        // Kevin Thacker's acid test "reg write/read mask test", on a CPC
        let mut psg = Psg::default();
        for (register, mask) in REGISTER_MASKS.iter().enumerate() {
            write(&mut psg, register as u8, 0xff);
            psg.bus_cycle(BusControl::LatchAddress, register as u8);
            assert_eq!(psg.bus_cycle(BusControl::Read, 0), Some(*mask), "register {register}");
        }
    }

    // -------------------------------------------------------- resampling

    fn run_audio(psg: &mut Psg, us: usize) -> Vec<f32> {
        run(psg, us);
        psg.take_samples()
    }

    #[test]
    fn no_samples_without_an_audio_output() {
        let mut psg = Psg::default();
        assert!(run_audio(&mut psg, 1000).is_empty());
        assert_eq!(psg.audio_config(), None);
    }

    #[test]
    fn one_second_gives_exactly_sample_rate_frames() {
        for rate in [44_100, 48_000, 22_050] {
            for stereo in [false, true] {
                let mut psg = Psg::default();
                psg.set_audio_output(Some(AudioConfig { sample_rate: rate, stereo, dc_filter: true }));
                let samples = run_audio(&mut psg, 1_000_000);
                let channels = if stereo { 2 } else { 1 };
                assert_eq!(samples.len(), rate as usize * channels, "{rate}Hz stereo={stereo}");
                assert_eq!(psg.pending_samples(), 0);
            }
        }
    }

    fn undc(mut config: AudioConfig) -> Option<AudioConfig> {
        config.dc_filter = false;
        Some(config)
    }

    #[test]
    fn a_constant_level_is_kept_by_the_resampler() {
        let mut psg = Psg::default();
        psg.set_audio_output(undc(AudioConfig::mono(44_100)));
        write_all(&mut psg, &[(7, mixer(0, 0)), (8, 15), (9, 7)]);
        let samples = run_audio(&mut psg, 100_000);
        let [a, b, _] = psg.channel_levels();
        for sample in &samples[2..] {
            assert!((*sample - (a + b) / 3.0).abs() < 1e-5, "{sample}");
        }
    }

    #[test]
    fn a_resampled_tone_keeps_its_frequency_and_mean() {
        let mut psg = Psg::default();
        psg.set_audio_output(undc(AudioConfig::mono(48_000)));
        // TP = 125: 1MHz / (16 * 125) = 500Hz, above and below the
        // multiples of the output rate.
        write_all(&mut psg, &[(7, mixer(0b001, 0)), (0, 125), (8, 15)]);
        let samples = run_audio(&mut psg, 1_000_000);
        let level = dac_level(15)[0] / 3.0;
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        assert!((mean - level / 2.0).abs() < level * 0.01, "{mean}");
        let crossings = (1..samples.len())
            .filter(|&i| samples[i - 1] < mean && samples[i] >= mean)
            .count();
        assert!((499..=501).contains(&crossings), "{crossings}");
        assert!(samples.iter().all(|s| (-1e-6..=level + 1e-6).contains(s)));
    }

    #[test]
    fn an_ultrasonic_tone_is_averaged_out() {
        let mut psg = Psg::default();
        psg.set_audio_output(undc(AudioConfig::mono(44_100)));
        // TP = 2: 31.25kHz, above the output's Nyquist frequency.
        write_all(&mut psg, &[(7, mixer(0b001, 0)), (0, 2), (8, 15)]);
        let samples = run_audio(&mut psg, 100_000);
        let level = dac_level(15)[0] / 3.0;
        // A point-sampled square wave would swing between 0 and `level`.
        for sample in &samples[2..] {
            assert!((sample - level / 2.0).abs() < level * 0.25, "{sample}");
        }
    }

    #[test]
    fn stereo_puts_a_left_c_right_and_b_in_the_middle() {
        let level = dac_level(15)[0];
        for (volumes, left, right) in [
            ([15, 0, 0], level / 1.5, 0.0),
            ([0, 0, 15], 0.0, level / 1.5),
            ([0, 15, 0], level / 3.0, level / 3.0)
        ] {
            let mut psg = Psg::default();
            psg.set_audio_output(undc(AudioConfig::stereo(44_100)));
            write_all(&mut psg, &[(7, mixer(0, 0)), (8, volumes[0]), (9, volumes[1]), (10, volumes[2])]);
            let samples = run_audio(&mut psg, 10_000);
            let last = &samples[samples.len() - 2..];
            assert!((last[0] - left).abs() < 1e-5, "{volumes:?}: {last:?}");
            assert!((last[1] - right).abs() < 1e-5, "{volumes:?}: {last:?}");
        }
    }

    #[test]
    fn the_dc_filter_centres_the_output() {
        let mut psg = Psg::default();
        psg.set_audio_output(Some(AudioConfig::mono(44_100)));
        write_all(&mut psg, &[(7, mixer(0b001, 0)), (0, 125), (8, 15)]);
        let samples = run_audio(&mut psg, 1_000_000);
        let tail = &samples[samples.len() / 2..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(mean.abs() < 1e-3, "{mean}");
        assert!(tail.iter().any(|&s| s < -0.1) && tail.iter().any(|&s| s > 0.1));
    }

    #[test]
    fn reset_keeps_the_audio_output_and_silences_the_chip() {
        let mut psg = Psg::default();
        psg.set_audio_output(Some(AudioConfig::mono(48_000)));
        write_all(&mut psg, &[(7, mixer(0, 0)), (8, 15)]);
        run(&mut psg, 1000);
        let pending = psg.pending_samples();
        assert!(pending > 0);
        psg.reset();
        assert_eq!(psg.pending_samples(), pending);
        assert_eq!(psg.audio_config(), Some(AudioConfig::mono(48_000)));
        assert_eq!(run(&mut psg, 16)[15], [0.0; 3]);
    }

    #[test]
    #[should_panic]
    fn a_zero_sample_rate_is_rejected() {
        Psg::default().set_audio_output(Some(AudioConfig::mono(0)));
    }
}
