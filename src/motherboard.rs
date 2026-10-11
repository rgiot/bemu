use cpclib_asm::preamble::Mnemonic;
use cpclib_asm::preamble::Token;
use smallvec::*;

use crate::chip::*;
use crate::counter::*;
use crate::crtc::*;
use crate::debug::DisassembledInstruction;
use crate::expansion::{LargeRamExpansion, LargeRamExpansionPin, RamBankExpansion};
use crate::fdc::Fdc;
use crate::ga::*;
use crate::keyboard::KeyboardMatrix;
use crate::memory::*;
use crate::pal::*;
use crate::ppi::*;
use crate::psg::{BusControl, Psg, REG_IO_PORT_A};
use crate::z80::*;

use std::convert::TryFrom;
use std::fmt;

/// Motherboard of the AMstrad CPC
/// Some component are emulated at the lowest level as possible (Z80/CRTC) while
/// some others are partially emulated (GA/PAL) or totally ignored (RAM)
/// http://www.cpcwiki.eu/index.php/Default_I/O_Port_Summary
pub struct Motherboard {
    cpu: Z80,
    crtc: Crtc,
    ga: GateArray,
    pal: Pal,
    ppi: Ppi,
    rom_selector: UpperRomSelector,
    memory: Memory,
    /// A card plugged into the expansion port extending RAM banking beyond
    /// the base machine's own 512K limit, if any (see `src/expansion.rs`).
    /// `None` is a stock machine - the common case, and the default.
    large_ram_expansion: Option<LargeRamExpansion>,
    fdc: Fdc,
    // The FDC isn't a pin-level `Chip` (see fdc.rs), so unlike the other
    // peripherals its register writes/reads aren't naturally idempotent
    // across repeated calls: a single Z80 `OUT`/`IN` instruction's bus
    // request condition (`cpu_request_byte_output`/`_input`) stays true
    // for several consecutive `tick()`s (matching real M-cycle timing),
    // and other chips tolerate being asked to "write"/"read" several times
    // for that one instruction because doing so is a no-op the 2nd+ time.
    // The FDC's command/result byte queues are not a no-op the 2nd time -
    // these track whether the current bus request was already serviced so
    // it's only acted on once, on its first tick.
    fdc_write_active: bool,
    fdc_read_active: bool,
    /// What a read of a port nothing decodes returns.
    floating_bus: u8,
    /// The byte latched onto the data bus for the currently active input
    /// cycle (see `fdc_read_active`) - held steady and reused across the
    /// cycle's repeated ticks instead of re-deriving it (which would
    /// re-invoke `Fdc::read_data` and advance its internal state each time).
    cached_input_byte: u8,

    psg: Psg,
    keyboard: KeyboardMatrix,

    // Counter of t-states (to be removed later)
    t_state_counter: Counter,
    // Counter of nops
    nops_counter: Counter,
}

impl fmt::Debug for Motherboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Motherboard")
            .field("cpu", &self.cpu)
            .field("crtc", &self.crtc)
            .field("ga", &self.ga)
            .field("pal", &self.pal)
            .finish()
    }
}

impl Default for Motherboard {
    fn default() -> Self {
        let mut motherboard = Motherboard {
            cpu: Default::default(),
            crtc: Default::default(),
            ga: Default::default(),
            pal: Default::default(),
            ppi: Default::default(),
            rom_selector: Default::default(),
            memory: Default::default(),
            large_ram_expansion: None,
            fdc: Default::default(),
            fdc_write_active: false,
            fdc_read_active: false,
            floating_bus: 0xff,
            cached_input_byte: 0xff,
            psg: Default::default(),
            keyboard: Default::default(),

            t_state_counter: Default::default(),
            nops_counter: Default::default(),
        };
        motherboard.stabilize_run();
        motherboard
    }
}

/// TODO add the other kinds of varitions
#[derive(Default, Clone, Copy)]
pub struct MotherboardBuilder {
    /// The type of CRTC in the motherboard
    crtc_type: Option<CrtcType>,
    /// Which of the two classic Gate Array chip families is fitted - see
    /// [`crate::ga::GateArrayType`]'s own doc comment.
    ga_type: Option<crate::ga::GateArrayType>,
    /// The manufacturer identity the firmware reads back via PPI port B
    /// (bits 1-3) to pick which name ("Amstrad", "Schneider", ...) to print
    /// in the boot banner.
    brand: Option<Brand>,
    /// If interruption are allowed
    enable_interruptions: bool, // false by default
    disable_lower_rom: bool, // false by default
    disable_upper_rom: bool, // false by defualt
    extra_ram_pages: Option<usize>,
    large_ram_expansion_banks: Option<usize>,
}

impl MotherboardBuilder {
    pub fn set_crtc(&mut self, crtc_type: CrtcType) -> &mut Self {
        self.crtc_type = Some(crtc_type);
        self
    }

    /// Which of the two classic Gate Array chip families is fitted - see
    /// [`crate::ga::GateArrayType`]'s own doc comment. Defaults to the
    /// 40010 (unset), matching what this emulator always modelled before
    /// this option existed.
    pub fn set_gate_array_type(&mut self, ga_type: crate::ga::GateArrayType) -> &mut Self {
        self.ga_type = Some(ga_type);
        self
    }

    pub fn set_brand(&mut self, brand: Brand) -> &mut Self {
        self.brand = Some(brand);
        self
    }

    pub fn enable_interruptions(&mut self) -> &mut Self {
        self.enable_interruptions = true;
        self
    }

    pub fn disable_lower_rom(&mut self) -> &mut Self {
        self.disable_lower_rom = true;
        self
    }

    pub fn disable_upper_rom(&mut self) -> &mut Self {
        self.disable_upper_rom = true;
        self
    }

    /// Fit `pages` extra 64kb RAM pages beyond the base 64kb every CPC has
    /// (1 = a stock 128K CPC 6128, the default if unset; up to 7 = a fully
    /// populated 512K expansion). See [`Motherboard::set_extra_ram_pages`].
    pub fn set_extra_ram_pages(&mut self, pages: usize) -> &mut Self {
        self.extra_ram_pages = Some(pages);
        self
    }

    /// Plugs a [`crate::expansion::LargeRamExpansion`] (the standard
    /// Yarek/RAM7/CPC4MB-style >512K protocol) into the expansion port,
    /// with `bank_count` 64kb banks fitted (up to 64 = 4MB). See
    /// [`Motherboard::plug_large_ram_expansion`].
    pub fn plug_large_ram_expansion(&mut self, bank_count: usize) -> &mut Self {
        self.large_ram_expansion_banks = Some(bank_count);
        self
    }

    pub fn build(self) -> Motherboard {
        let crtc_type = self.crtc_type.unwrap_or_default();

        let mut motherboard = Motherboard {
            crtc: crtc_type.into(),
            ..Default::default()
        };

        if let Some(brand) = self.brand {
            motherboard.ppi.set_brand(brand);
        }
        if let Some(ga_type) = self.ga_type {
            motherboard.ga.set_gate_array_type(ga_type);
        }
        // The CPC+ (CRTC 3) has its PPI inside the ASIC, which keeps its
        // ports on a mode-set write, unlike a real 8255.
        motherboard
            .ppi
            .set_clears_ports_on_mode_set(crtc_type != CrtcType::Crtc3);

        if self.disable_lower_rom {
            motherboard.disable_lower_rom();
        }
        if self.disable_upper_rom {
            motherboard.disable_upper_rom();
        }
        if let Some(pages) = self.extra_ram_pages {
            motherboard.set_extra_ram_pages(pages);
        }
        if let Some(bank_count) = self.large_ram_expansion_banks {
            motherboard.plug_large_ram_expansion(bank_count);
        }
        motherboard.stabilize_run();

        if self.enable_interruptions {
            motherboard.cpu.iff1 = true ;
            motherboard.cpu.iff2 = true ;
            motherboard.cpu.im = InterruptMode::Mode1;
        }
        
        motherboard
    }
}

// Fields access
impl Motherboard {
    #[inline]
    pub fn cpu(&self) -> &Z80 {
        &self.cpu
    }

    #[inline]
    pub fn cpu_mut(&mut self) -> &mut Z80 {
        &mut self.cpu
    }

    #[inline]
    pub fn crtc(&self) -> &Crtc {
        &self.crtc
    }

    pub fn crtc_mut(&mut self) -> &mut Crtc {
        &mut self.crtc
    }

    pub fn ga(&self) -> &GateArray {
        &self.ga
    }

    pub fn ga_mut(&mut self) -> &mut GateArray {
        &mut self.ga
    }

    pub fn ppi(&self) -> &Ppi {
        &self.ppi
    }

    pub fn ppi_mut(&mut self) -> &mut Ppi {
        &mut self.ppi
    }

    pub fn fdc(&self) -> &Fdc {
        &self.fdc
    }

    pub fn fdc_mut(&mut self) -> &mut Fdc {
        &mut self.fdc
    }

    pub fn psg(&self) -> &Psg {
        &self.psg
    }

    pub fn psg_mut(&mut self) -> &mut Psg {
        &mut self.psg
    }

    pub fn keyboard(&self) -> &KeyboardMatrix {
        &self.keyboard
    }

    pub fn keyboard_mut(&mut self) -> &mut KeyboardMatrix {
        &mut self.keyboard
    }

    pub fn t_state_counter(&self) -> &Counter {
        &self.t_state_counter
    }

    pub fn nops_counter(&self) -> &Counter {
        &self.nops_counter
    }

    pub fn nops_counter_mut(&mut self) -> &mut Counter {
        &mut self.nops_counter
    }
}

impl Motherboard {
    #[inline]
    fn stabilize_run(&mut self) {
        self.reset();
        while !self.cpu.is_activated() {
            self.ga.manage_clocks();
            self.sync_chips_pins(); // just to get the front stuff
        }
        self.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA !!
    }

    /// Reset all the components of the motherboard
    /// Set what reading a port that no chip decodes returns (the CPC leaves
    /// the bus floating: &FF unless something drives it).
    pub fn set_floating_bus(&mut self, value: u8) {
        self.floating_bus = value;
    }

    pub fn reset(&mut self) {
        self.crtc.reset();
        self.ga.reset();
        self.pal.reset();
        if let Some(expansion) = &mut self.large_ram_expansion {
            expansion.reset();
        }
        self.cpu.reset();

        for _ in 0..2 {
            self.sync_chips_pins();
        }
    }

    /// Execute one tick on all the components of the motherboard and returns the
    /// corresponding gate array information
    /// TODO check if it must be generated before or after
    pub fn tick(&mut self) -> (RgbLevels, Sync) {
        // if self.crtc.is_activated() {dbg!(&self.cpu().next_state);}

        self.handle_write_access();

        // Keep this order to avoid side effect mixing previous state and current state
        //
        // The CRTC's video counters advance on its character clock (CCLK),
        // but its CPU-facing register interface is asynchronous to that
        // clock: the CPU holds CS/RS/RW for a whole I/O cycle whatever CCLK
        // is doing. Running the register access only on CCLK edges (as it
        // used to) lets a short I/O cycle fall between two edges and never be
        // serviced, so e.g. a register read returned whatever the CPU had
        // left on the data bus. Access is idempotent for the duration of a
        // cycle (the data bus is stable), so servicing it on every tick is
        // safe.
        if self.crtc.is_activated() {
            self.crtc.tick(); // Virtually uses 16 cycles
        }
        else if self.crtc.must_execute_tick() {
            self.crtc.execute_tick();
        }
        // The gate array blanks the pixels the very tick the CRTC drops
        // DE: it fetches the video bytes from the CRTC's address pins of the
        // current tick, so seeing DE one tick late made the border start (and
        // the paper end) a pixel late - the first pixel of the next fetched
        // byte leaked at the right edge of every line and the first pixel of
        // the line was lost.
        let display = self.crtc.get_pin(CrtcPin::DE)
            && !((self.crtc.second_byte_is_border() || self.crtc.r6_conflict_second_byte_is_border())
                && self.ga.get_pin(GateArrayPin::CCLK));
        self.ga.set_pin(GateArrayPin::DISPEN, display);
        // Unlike the CRTC (whose is_activated() gates on CCLK - a genuinely
        // free-running clock unrelated to chip-select, since its internal
        // counters must advance regardless of CPU access), the PPI's
        // is_activated() gates on the falling edge of Neg_CS itself. Since
        // Neg_CS stays selected for the PPI's *whole* multi-tick read/write
        // window (not a single-tick pulse), that edge only fires on the
        // exact tick selection begins - which, depending on what Neg_RD/
        // Neg_WR happen to be doing at that precise tick, can easily miss
        // every genuine access, permanently freezing whatever the PPI last
        // read/wrote (e.g. the CPU polling PPI port B for VSYNC would never
        // observe it change). `Chip::tick()` already gates on the correct,
        // level-based `must_execute_tick()` internally, so call it
        // unconditionally here and let that internal check decide.
        self.ppi.tick();
        self.sync_psg();
        // The gate array owns the video bus: when its pixel pipeline is
        // empty it fetches the byte at the address the CRTC currently
        // exposes (MA/RA), whatever ROM is overlaid for the CPU.
        if self.ga.need_memory_byte() {
            let address = self
                .ga
                .video_fetch_address(self.crtc.get_address_from_pins());
            let byte = self.read_video_ram_byte(address);
            self.ga.set_memory_byte(byte);
        }
        self.ga.set_last_idle_t(self.cpu.last_idle_t());
        self.ga.tick();
        self.pal.tick();
        if let Some(expansion) = &mut self.large_ram_expansion {
            expansion.tick();
        }
        // Never ticked before at all - only its pins were kept in sync via
        // sync_pins, so `execute_tick()` (which latches the selected upper
        // ROM number from the data bus) never ran and `rom_number` stayed
        // at whatever `reset()`/`force_upper_rom_number()` last set it to,
        // regardless of what the CPU tried to select via `OUT`. No internal
        // counters here (no `pre_tick()` override), so - like the PPI -
        // ticking it unconditionally and letting `must_execute_tick()`
        // gate `execute_tick()` is correct.
        self.rom_selector.tick();

        self.handle_read_access();

        if self.cpu.is_activated() {
            self.cpu.tick(); // Virtually uses 4 cycles
            self.t_state_counter.inc();

            // By definition it is a new opcode because the previous one has been executed
            if self.t_state_counter.value() % 4 == 0 {
                self.nops_counter.inc();
            }
        }

        let ga_output = (self.ga.rgb(), self.ga.sync());

        // Divides and distribute the 16Mhz clock for next round
        self.ga.manage_clocks();

        // make the synchro between the pins of the various chips
        // TODO replace that by a real bus abstraction
        self.sync_chips_pins();

        ga_output
    }

    // Manage the operations where CPU requests a write
    pub fn handle_write_access(&mut self) {
        let addr = self.cpu.get_address_from_pins();
        let byte = self.cpu.get_data_from_pins();

        if self.ga_request_read() {
            unimplemented!();
        }  else if self.cpu_request_byte_write() {
        //    eprintln!("0x{:x}=0x{:x}", addr, byte);
            self.write_byte(addr, byte);
            self.cpu.set_pin(Z80Pin::Neg_WR, true); // TODO do the real stuff - but here I am sure to not play with the data bus ...
            self.fdc_write_active = false;
        } else if self.cpu_request_byte_output() {
            // there is nothing to do there as the pin syncing method already did everything
            // ... except for the FDC, which isn't a pin-level `Chip` (see
            // fdc.rs) and so isn't reached by `sync_chips_pins()`. This
            // condition stays true for several consecutive ticks per single
            // `OUT` instruction (matching real M-cycle timing), so the
            // actual write only happens once, on the first such tick.
            if !self.fdc_write_active {
                if Self::is_fdc_motor_addr(addr) {
                    self.fdc.set_motor(byte & 1 != 0);
                } else if Self::is_fdc_data_addr(addr) {
                    self.fdc.write_data(byte);
                }
            }
            self.fdc_write_active = true;
        } else {
            self.fdc_write_active = false;
        }
    }

    /// Address decode for the disk interface (motor control latch, FDC
    /// status/data registers). Cross-checked against the NEC uPD765A/B
    /// datasheet's register addressing (`doc/fdc.upd765ab.pdf`) and the
    /// well-known CPC port conventions &FA7E/&FB7E/&FB7F: only A15-A8, A7
    /// and (for the FDC's own two registers) A0 are actually decoded by the
    /// hardware - firmware just always uses those conventional addresses.
    #[inline]
    fn is_fdc_motor_addr(addr: u16) -> bool {
        (addr & 0xff80) == 0xfa00
    }

    #[inline]
    fn is_fdc_status_addr(addr: u16) -> bool {
        (addr & 0xff81) == 0xfb00
    }

    #[inline]
    fn is_fdc_data_addr(addr: u16) -> bool {
        (addr & 0xff81) == 0xfb01
    }

    /// Manage the operations where CPU requests a read
    pub fn handle_read_access(&mut self) {
        let addr = self.cpu.get_address_from_pins();

        if self.ga_request_read() {
            unimplemented!();
        
        } else  if self.cpu_request_byte_read() {
            let byte = self.read_byte(addr);
            self.cpu.set_data_on_pins(byte);
            self.cpu.set_pin(Z80Pin::Neg_RD, true); // TODO do the real stuff - but here I am sure to not play with the data bus ...
            self.fdc_read_active = false;
        }else if self.cpu_request_byte_input() {
            // As in handle_write_access: this condition stays true for
            // several consecutive ticks per single `IN` instruction, so
            // `input_byte` (which can advance the FDC's internal state) is
            // only actually called once, on the first such tick - the
            // latched byte is then held steady on the bus for the rest of
            // the cycle, same as real hardware would.
            if !self.fdc_read_active {
                self.cached_input_byte = self.input_byte(addr);
            }
            self.cpu.set_data_on_pins(self.cached_input_byte);
            self.cpu.set_pin(Z80Pin::Neg_RD, true);
            self.fdc_read_active = true;
        } else {
            self.fdc_read_active = false;
        }
    }

    #[inline]
    fn cpu_request_byte_read(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_RD) && !self.cpu.get_pin(Z80Pin::Neg_MREQ)
    }

    #[inline]
    fn cpu_request_byte_write(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_WR)&& !self.cpu.get_pin(Z80Pin::Neg_MREQ)
    }

    #[inline]
    fn cpu_request_byte_input(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_RD) && !self.cpu.get_pin(Z80Pin::Neg_IORQ)
    }

    #[inline]
    fn cpu_request_byte_output(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_WR)&& !self.cpu.get_pin(Z80Pin::Neg_IORQ)
    }



    #[inline]
    /// TODO implement that aspect
    fn ga_request_read(&self) -> bool {
        false
    }

    /// Completely execute an opcode, handing every tick's video output (the
    /// gate array's RGB and sync) to `on_tick`, without allocating.
    #[inline]
    pub fn execute_next_opcode_with_video<F>(&mut self, on_tick: F)
    where
        F: FnMut((RgbLevels, Sync)),
    {
        self.execute_next_opcode_internal(on_tick);
    }

    #[inline]
    fn execute_next_opcode_internal<F>(&mut self, mut on_tick: F)
    where
        F: FnMut((RgbLevels, Sync)),
    {
        assert!(
            self.cpu.next_state.is_new_opcode(),
            "We have to start by a new opcode"
        );

        let mut has_executed_first_state_counter = false;
        let mut has_finished_z80_execution = false;

        /// execute the whole stuff as soon as it is necessary
        loop {
            on_tick(self.tick());

            has_executed_first_state_counter =
                has_executed_first_state_counter | !self.cpu.next_state.is_new_opcode();

            if has_executed_first_state_counter && self.cpu.is_downfront(Z80::clock_pin()) {
                has_finished_z80_execution = true;
            }

            if self.cpu.is_activated()
                && self.cpu.next_state.is_new_opcode()
                && has_executed_first_state_counter
                && has_finished_z80_execution
            {
                break;
            }
        }
    }

    /// Completly execute an opcode and returns the set of video information associated
    pub fn execute_next_opcode(&mut self) -> Vec<(RgbLevels, Sync)> {
        let mut video = Vec::with_capacity(32);
        self.execute_next_opcode_internal(|ga_output| video.push(ga_output));
        video
    }

    /// Completely execute an opcode without collecting video samples.
    /// Useful for CPU-only test harnesses where frame data is not consumed.
    #[inline]
    pub fn execute_next_opcode_no_video(&mut self) {
        self.execute_next_opcode_internal(|_| {});
    }

    pub fn execute_ticks(&mut self, ticks: usize) -> Vec<(RgbLevels, Sync)> {
        let mut video = Vec::new();

        loop {
            video.append(&mut self.execute_next_opcode());
            if video.len() >= ticks {
                break;
            }
        }
        video
    }

    /// Drive the PSG's bus protocol from PPI port A/C, exactly the way real
    /// hardware wires it: port A *is* the PSG's data bus (DA0-7), and port
    /// C bits 7/6 are BDIR/BC1 (bits 0-3 select the keyboard row read back
    /// through register 14 - see `psg.rs` and `keyboard.rs`).
    fn sync_psg(&mut self) {
        // Deliberately unconditional (no dedup on port C's resulting byte
        // vs. last time): two genuinely separate writes/latches can use
        // the identical control byte while Port A's data differs between
        // them (e.g. the firmware's own keyboard-scan routine latching
        // PSG register 14 via the same BDIR/BC1 pulse pattern another
        // routine, like a music player, just used for a different
        // register) - a dedup keyed only on port C's value silently
        // dropped the second, *intended* latch, since nothing about port
        // C's own byte looked different. `Psg::bus_cycle` is idempotent
        // for repeated identical (control, data_bus) pairs - the same
        // write/latch/read landing again every tick for the ~8 ticks of
        // one CPU instruction's bus-active window is harmless - so there
        // is no correctness or performance reason to skip this.
        let port_c = self.ppi.get_data_c_from_pins();
        let row_select = port_c & 0x0f;
        let bc1 = port_c & (1 << 6) != 0;
        let bdir = port_c & (1 << 7) != 0;
        let control = BusControl::from_bdir_bc1(bdir, bc1);

        // Keep register 14 (I/O port A) in sync with whatever row is
        // currently selected before it's potentially read below - on real
        // hardware this register *is* the keyboard matrix line, live.
        //
        // Per the AY-3-8910/8912 datasheet (`doc/psg.ay-3-8910.datamanual.pdf`,
        // §3.6, octal register R16 = our decimal 14): "when in the input
        // mode, the contents of registers R16 and/or R17 will follow the
        // signals applied to the I/O port(s)" - this tracking happens live,
        // regardless of which register currently sits in the address latch;
        // only *transferring* that value to the CPU bus needs an explicit
        // read cycle with 14 addressed. Gating the update itself behind
        // `selected_register() == Some(REG_IO_PORT_A)` (removed below) was
        // wrong: any other code (e.g. a music player selecting a different
        // register to silence channels) would freeze register 14's mirror
        // stale, which the firmware's own keyboard-checksum self-test then
        // read back as a hardware fault and cold-reset on - reproduced via
        // HBL.DSK rebooting during its "amsdos leaving" sequence.
        if self.psg.port_a_is_input() {
            self.psg.set_register(REG_IO_PORT_A, self.keyboard.line(row_select));
        }

        let data_bus = self.ppi.get_data_a_from_pins();
        if let Some(read_value) = self.psg.bus_cycle(control, data_bus) {
            self.ppi.set_port_a_input(read_value);
        }
    }

    /// Make the appropriate copy between some pins of some chips to other (we simulate the various buses here)
    /// TODO improve behavior by handling BUSACK BUSREQ
    /// XXX as z80 timings are WRONG at them moment, it may change the signals too fast. As a workaround copy are made conditional for some chips
    fn sync_chips_pins(&mut self) {
        // This function only ever *reads* from `cpu`, `ga` and `crtc` (as
        // sources) - everything else it touches (`pal`, `rom_selector`,
        // `ppi`, `large_ram_expansion`) is a pure destination, driven
        // entirely by those three. Profiling (`project_cpu_tick_performance`)
        // found this function was the second-largest cost in the emulator,
        // and an all-or-nothing "skip if nothing changed" guard turned out
        // to help essentially never in practice (empirically, ~0% of
        // ticks: measured via temporary instrumentation) - GA's own
        // clock/interrupt pins change on the large majority of ticks by
        // themselves, which alone defeats an all-three-at-once check almost
        // every time. But CPU's pins specifically are unchanged on ~44% of
        // ticks (same measurement), and the CPU-sourced section below is
        // the overwhelming majority of this function's own cost (~120 of
        // ~125 pin operations) - so each source gets its *own* independent
        // gate instead of one shared one. Each section's own
        // `save_pins_state()` call is skipped along with it, which is
        // exactly correct, not just "probably fine": if that source didn't
        // change, its own `.0`/`.1` already agree, so re-saving would have
        // been a no-op anyway (see `Chip::any_pin_changed`'s doc comment).
        //
        // None of `pal`/`rom_selector`/`ppi`/`large_ram_expansion`'s own
        // `must_execute_tick()` gates use edge detection (grepped for
        // `is_upfront`/`is_downfront`/`Activable` across the whole crate to
        // confirm) - only `ga` does, on the VSYNC/HSYNC pins this function
        // itself drives from `crtc`, which is why that write is gated on
        // `crtc_changed` below, not folded into the GA section. If that
        // ever changes, these gates would need revisiting.
        let cpu_changed = self.cpu.any_pin_changed();
        let ga_changed = self.ga.any_pin_changed();
        let crtc_changed = self.crtc.any_pin_changed();

        // Unlike `pal`/`rom_selector`/`ppi`/`large_ram_expansion` (pure
        // destinations, written only from `cpu`, so their saves stay
        // correctly gated below), `cpu`/`ga`/`crtc` are each BOTH a source
        // *and* a destination for one of the other two within this same
        // function (the GA-sourced block below writes into CPU's CLK/INT
        // pins; the CRTC-sourced block writes into GA's VSYNC/HSYNC/DISPEN
        // pins) - so their own `save_pins_state()` must run unconditionally,
        // every call, exactly like before this change. Gating any of these
        // three on its *own* dirty flag alone would miss a write landing on
        // it from one of the other two sections later in this same call,
        // silently delaying edge detection by a tick - confirmed the hard
        // way: an earlier version of this gated each save on its own flag
        // and broke `test_z80_vsync_activated` exactly this way (CPU's CLK
        // edge, driven by GA, got detected a tick late whenever CPU's own
        // pins happened not to change that same tick). Only the *writes*
        // below (the actual expensive ~125 pin operations) are safe to
        // skip per-source - the saves themselves are cheap (one `u64`
        // compare each) regardless.
        self.ga.save_pins_state();
        self.crtc.save_pins_state();
        self.cpu.save_pins_state();

        // GA-sourced: its own clock/interrupt lines, to CPU and CRTC.
        if ga_changed {
            sync_pins(
                &self.ga,
                &[GateArrayPin::NCPU, GateArrayPin::INTERRUPT],
                &mut self.cpu,
                &[Z80Pin::Neg_CLK, Z80Pin::Neg_INT],
            );
            sync_pins(
                &self.ga,
                &[GateArrayPin::CCLK],
                &mut self.crtc,
                &[CrtcPin::CCLK],
            );
        }

        // CRTC-sourced: video timing signals, to PPI (VSYNC -> port B bit
        // 0) and the gate array (VSYNC/HSYNC/DISPEN).
        if crtc_changed {
            self.ppi.set_pin(PpiPin::B0, self.crtc.get_pin(CrtcPin::VS));
            sync_pins(
                &self.crtc,
                &[CrtcPin::VS, CrtcPin::HS, CrtcPin::DE],
                &mut self.ga,
                &[
                    GateArrayPin::VSYNC,
                    GateArrayPin::HSYNC,
                    GateArrayPin::DISPEN,
                ],
            );
        }

        // CPU-sourced: the bulk of real bus traffic - address/data/control
        // lines, to every peripheral actually wired to the CPU bus.
        if cpu_changed {
            self.pal.save_pins_state();
            if let Some(expansion) = &mut self.large_ram_expansion {
                expansion.save_pins_state();
            }

            let Neg_IOWR = self.cpu.get_pin(Z80Pin::Neg_WR) || self.cpu.get_pin(Z80Pin::Neg_IORQ);
            let Neg_IORD = self.cpu.get_pin(Z80Pin::Neg_RD) || self.cpu.get_pin(Z80Pin::Neg_IORQ);

            if !Neg_IORD {
                assert!(!self.cpu.get_pin(Z80Pin::Neg_IORQ));
            }

            // CPU to GA
            sync_pins(
                &self.cpu,
                Z80::pins_data(),
                &mut self.ga,
                GateArray::pins_data(),
            );
            sync_pins(
                &self.cpu,
                &[
                    Z80Pin::A14,
                    Z80Pin::A15,
                    Z80Pin::Neg_IORQ,
                    Z80Pin::Neg_RD,
                    Z80Pin::Neg_M1,
                    Z80Pin::Neg_MREQ, ],
                &mut self.ga,
                &[
                    GateArrayPin::A14,
                    GateArrayPin::A15,
                    GateArrayPin::Neg_IORQ,
                    GateArrayPin::Neg_RD,
                    GateArrayPin::Neg_M1,
                    GateArrayPin::Neg_MREQ
                ],
            );

            // CPU to PAL
            sync_pins(
                &self.cpu,
                &[
                    Z80Pin::A14,
                    Z80Pin::A15,
                    Z80Pin::D0,
                    Z80Pin::D1,
                    Z80Pin::D2,
                    Z80Pin::D3,
                    Z80Pin::D4,
                    Z80Pin::D5,
                ],
                &mut self.pal,
                &[
                    PalPin::A14,
                    PalPin::A15,
                    PalPin::D0,
                    PalPin::D1,
                    PalPin::D2,
                    PalPin::D3,
                    PalPin::D4,
                    PalPin::D5,
                ],
            );
            self.pal.set_pin(
                PalPin::D6_and_D7,
                self.cpu.get_pin(Z80Pin::D6) && self.cpu.get_pin(Z80Pin::D7),
            );
            self.pal.set_pin(PalPin::IOWR, !Neg_IOWR);

            // CPU to the expansion port's large RAM card, if any is plugged
            // in - it needs the full data byte (not `Pal`'s
            // D6-D7-collapsed-to-one shortcut) plus address lines A8-A10,
            // which the base machine's own decode never looks at (see
            // `LargeRamExpansion`'s doc comment).
            if let Some(expansion) = &mut self.large_ram_expansion {
                sync_pins(
                    &self.cpu,
                    &[
                        Z80Pin::A8,
                        Z80Pin::A9,
                        Z80Pin::A10,
                        Z80Pin::A14,
                        Z80Pin::A15,
                        Z80Pin::D0,
                        Z80Pin::D1,
                        Z80Pin::D2,
                        Z80Pin::D3,
                        Z80Pin::D4,
                        Z80Pin::D5,
                        Z80Pin::D6,
                        Z80Pin::D7,
                    ],
                    expansion,
                    &[
                        LargeRamExpansionPin::A8,
                        LargeRamExpansionPin::A9,
                        LargeRamExpansionPin::A10,
                        LargeRamExpansionPin::A14,
                        LargeRamExpansionPin::A15,
                        LargeRamExpansionPin::D0,
                        LargeRamExpansionPin::D1,
                        LargeRamExpansionPin::D2,
                        LargeRamExpansionPin::D3,
                        LargeRamExpansionPin::D4,
                        LargeRamExpansionPin::D5,
                        LargeRamExpansionPin::D6,
                        LargeRamExpansionPin::D7,
                    ],
                );
                expansion.set_pin(LargeRamExpansionPin::IOWR, !Neg_IOWR);
            }

            // CPU to CRTC
            // EN is active (low) during a real I/O cycle only: an interrupt
            // acknowledge also drives IORQ (together with M1) and must not
            // be seen as an access to whatever port the PC happens to spell.
            self.crtc.set_pin(
                CrtcPin::EN,
                (Neg_IORD & Neg_IOWR) | !self.cpu.get_pin(Z80Pin::Neg_M1),
            );

            sync_pins(
                &self.cpu,
                &[Z80Pin::A8, Z80Pin::A9, Z80Pin::A14],
                &mut self.crtc,
                &[CrtcPin::RS, CrtcPin::RW, CrtcPin::CS],
            );
            sync_pins(
                &self.cpu,
                Z80::pins_data(),
                &mut self.crtc,
                Crtc::pins_data(),
            );
            // CPU to rom selection
            sync_pins(
                &self.cpu,
                Z80::pins_data(),
                &mut self.rom_selector,
                UpperRomSelector::pins_data(),
            );
            sync_pins(
                &self.cpu,
                &[Z80Pin::A13, Z80Pin::Neg_IORQ, Z80Pin::Neg_WR],
                &mut self.rom_selector,
                &[
                    UpperRomSelectorPin::A13,
                    UpperRomSelectorPin::IORQ,
                    UpperRomSelectorPin::WR,
                ],
            );

            // CPU to ppi
            if !Neg_IOWR {
                sync_pins(&self.cpu, Z80::pins_data(), &mut self.ppi, Ppi::pins_data());
            }
            sync_pins(
                &self.cpu,
                &[Z80Pin::A11, Z80Pin::A9, Z80Pin::A8],
                &mut self.ppi,
                &[PpiPin::Neg_CS, PpiPin::InA1, PpiPin::InA0],
            );
            // Neg_CS must only be asserted during a real I/O bus cycle - A11
            // alone is also whatever it happens to be during ordinary memory
            // fetches, so without gating on Neg_IORQ here the PPI's
            // is_activated() (an edge on Neg_CS) essentially never fires
            // again once code executes from a page where A11 stays constant,
            // freezing every PPI port read/write (e.g. VSYNC polling on
            // port B) at whatever stale value it last captured.
            self.ppi.set_pin(
                PpiPin::Neg_CS,
                self.cpu.get_pin(Z80Pin::A11) || self.cpu.get_pin(Z80Pin::Neg_IORQ),
            );
            self.ppi.set_pin(PpiPin::Neg_RD, Neg_IORD);
            self.ppi.set_pin(PpiPin::Neg_WR, Neg_IOWR);
        }
    }
}

// Block for disassembling related functions
impl Motherboard {
    /// Return the opcode that will be executed
    pub fn disassemble_next_instruction(&self) -> DisassembledInstruction {
        self.disassemble_instruction_at(self.cpu.pc.value())
    }

    /// Disassemble a bunch of instruction given an address
    pub fn disassemble_instructions_from(&self, mut pc: u16, count: usize) -> Vec<DisassembledInstruction> {
        let mut res = Vec::new();
        for _ in 0..count {
            let inst = self.disassemble_instruction_at(pc);
            pc = pc.wrapping_add(inst.len() as u16);
            res.push(inst);
        }
        res
    }

    /// Disassemble the instruction at the given address
    pub fn disassemble_instruction_at(&self, mut pc: u16) -> DisassembledInstruction {
        let address = pc;

        let mut read = || {
            let byte = self.read_byte(pc);
            pc = pc.wrapping_add(1);
            byte
        };

        let mut current_prefix = OpcodePrefix::None;
        let mut instruction_bytes : SmallVec <[u8;4]> = Default::default();
        let mut ir = 0;
        loop {
            ir = read();
            instruction_bytes.push(ir);
            match current_prefix {
                OpcodePrefix::None => {
                    current_prefix = match ir {
                        0xdd => OpcodePrefix::Dd,
                        0xfd => OpcodePrefix::Fd,
                        0xcb => OpcodePrefix::Cb,
                        0xed => OpcodePrefix::Ed,
                        _ => OpcodePrefix::None
                    };
                    
                    if current_prefix == OpcodePrefix::None {
                        break;
                    }
                },

                OpcodePrefix::Cb | OpcodePrefix::Ed => {
                    break;
                }

                OpcodePrefix::Dd => {
                    if ir == 0xcb {
                        current_prefix = OpcodePrefix::DdCb
                    } else {
                        break;
                    }
                },
                
                OpcodePrefix::Fd => {
                    if ir == 0xcb {
                        current_prefix = OpcodePrefix::FdCb
                    } else {
                        break;
                    }
                },

                OpcodePrefix::DdCb |  OpcodePrefix::FdCb => {
                   break;
                }
            }
        }

        /// Read the extra byte
        let nb_bytes = if matches!(current_prefix, OpcodePrefix::DdCb |  OpcodePrefix::FdCb) {1} else {current_prefix.opcode_information(ir).nb_additional_opcode_bytes};
        for _i in 0..nb_bytes {
            instruction_bytes.push(read());
        }

        let token = cpclib_asm::disass::disassemble(&instruction_bytes)
        .get(0)
        .cloned()
        .unwrap_or(Token::OpCode(Mnemonic::Nop, None, None, None));
        

        DisassembledInstruction {
            token,
            memory: instruction_bytes,
            address,
        }
    }

    /// Execute the right amount of ticks to be sure next CPU tick will be in the start of an instruction
    pub fn tick_until_next_instruction(&mut self) {
        while !self.is_aligned_with_new_instruction() {
            self.tick();
        }
    }
}

// Block for the memory related functions
impl Motherboard {
    /// Provides the memory configuration for a proper access
    pub fn memory_config(&self) -> MemoryConfig {
        let mut ram_config = self.pal.get_ram_configuration();
        if let Some(expansion) = &self.large_ram_expansion {
            ram_config = ram_config.with_expansion_high_bits(expansion.ram_bank_high_bits());
        }
        MemoryConfig(
            RomConfig {
                lower_rom: self.ga.lower_rom_pagging(),
                upper_rom: self.ga.upper_rom_pagging(),
                rom_number: self.rom_selector.upper_rom_number(),
            },
            ram_config,
        )
    }

    /// Changes the rom configuration of the machine
    pub fn force_rom_config(&mut self, config: RomConfig) {
        self.ga.force_lower_rom_pagging(config.lower_rom);
        self.ga.force_upper_rom_pagging(config.upper_rom);
        self.rom_selector.force_upper_rom_number(config.rom_number);
    }

    pub fn disable_lower_rom(&mut self) {
        self.ga.force_lower_rom_pagging(Flag::Disable);
    }

    pub fn disable_upper_rom(&mut self) {
        self.ga.force_upper_rom_pagging(Flag::Disable);
    }

    /// Change how many extra 64kb RAM pages are fitted, beyond the base
    /// 64kb every CPC has (1 = a stock 128K CPC 6128, the default; up to 7
    /// = a fully populated 512K expansion, the standard CPC6128-style RAM
    /// select's own limit). See [`Memory::set_extra_pages`]. For a real
    /// >512K expansion card (up to 4MB), use [`Self::plug_large_ram_expansion`]
    /// instead - it calls this with the card's own bank count.
    pub fn set_extra_ram_pages(&mut self, pages: usize) {
        self.memory.set_extra_pages(pages);
    }

    /// How many extra 64kb RAM pages are currently fitted.
    pub fn extra_ram_pages(&self) -> usize {
        self.memory.extra_pages()
    }

    /// Plugs a [`LargeRamExpansion`] (the standard Yarek/RAM7/CPC4MB-style
    /// >512K protocol, see `src/expansion.rs`) into the expansion port,
    /// with `bank_count` 64kb banks fitted (up to 64 = 4MB) - the card's own
    /// [`Page`]s, not `Memory`'s (see [`Expansion`]'s doc comment
    /// for why). `Memory::additional_memory` (the CPC's own on-board extra
    /// page) is left exactly as it was: real hardware wires a plugged-in
    /// card to the same bus lines, so once one *is* plugged in,
    /// `RamConfig::with_expansion_high_bits` always redirects every access
    /// that would have reached it to the card instead - it becomes
    /// unreachable, not explicitly resized away, matching the spec's own
    /// note that a real card "replaces", not adds to, that internal page.
    pub fn plug_large_ram_expansion(&mut self, bank_count: usize) {
        self.large_ram_expansion = Some(LargeRamExpansion::new(bank_count));
    }

    /// Removes whatever [`LargeRamExpansion`] was plugged in (a no-op if
    /// none was), reverting to the stock machine's own single extra page -
    /// automatically reachable again, since `memory_config()` stops calling
    /// `with_expansion_high_bits` at all once `large_ram_expansion` is
    /// `None`.
    pub fn unplug_large_ram_expansion(&mut self) {
        self.large_ram_expansion = None;
    }

    /// How many 64kb banks the currently plugged-in `LargeRamExpansion`
    /// provides, if any - `None` on a stock machine.
    pub fn large_ram_expansion_bank_count(&self) -> Option<usize> {
        self.large_ram_expansion.as_ref().map(|e| Expansion::ram_bank_count(e))
    }

    /// The currently plugged-in `LargeRamExpansion`'s last-latched high
    /// bits (see `RamBankExpansion::ram_bank_high_bits`), if any - lets
    /// external code (tests, diagnostics) observe whether anything has
    /// actually driven a port other than the standard &7Fxx.
    pub fn large_ram_expansion_high_bits(&self) -> Option<u8> {
        self.large_ram_expansion.as_ref().map(|e| e.ram_bank_high_bits())
    }

    /// Whatever's plugged into the expansion port, as the trait `Memory`
    /// itself needs to reach its (real, on-card) RAM/ROM - see
    /// `src/memory.rs`'s `Expansion` doc comment for why `Memory` never
    /// owns this storage. Read-only: the write paths below need a `&mut`
    /// version too, but a `&mut self` helper for that would conflict with
    /// also borrowing `self.memory` mutably in the same call - they
    /// destructure `self` into its `memory`/`large_ram_expansion` fields
    /// directly instead, which the borrow checker can see are disjoint.
    fn expansion_ram_source(&self) -> Option<&dyn Expansion> {
        self.large_ram_expansion.as_ref().map(|e| e as &dyn Expansion)
    }

    /// Write the databuffer in memory according to the memory configuration
    pub fn write_buffer(&mut self, mut addr: u16, buffer: &[u8]) {
        let config = self.memory_config();
        let Motherboard { memory, large_ram_expansion, .. } = self;
        for byte in buffer.iter() {
            memory.write(addr, *byte, &config, large_ram_expansion.as_mut().map(|e| e as &mut dyn Expansion));
            addr = addr.wrapping_add(1);
        }
    }

    /// Write one memory byte according to the memory configuration
    #[inline]
    pub fn write_byte(&mut self, addr: u16, byte: u8) {
        let memory_cfg = self.memory_config();
        let Motherboard { memory, large_ram_expansion, .. } = self;
        memory.write(addr, byte, &memory_cfg, large_ram_expansion.as_mut().map(|e| e as &mut dyn Expansion));
    }

    pub fn write_word(&mut self, addr: u16, word: u16) {
        let memory_cfg = self.memory_config();
        let low = (word & 0xff) as u8;
        let high = ((word & 0xff00) >> 8) as u8;
        let Motherboard { memory, large_ram_expansion, .. } = self;
        memory.write(addr, low, &memory_cfg, large_ram_expansion.as_mut().map(|e| e as &mut dyn Expansion));
        memory.write(addr.wrapping_add(1), high, &memory_cfg, large_ram_expansion.as_mut().map(|e| e as &mut dyn Expansion));
    }

    /// Read one memory byte according to the memory configuration
    #[inline]
    pub fn read_byte(&self, addr: u16) -> u8 {
        let memory_cfg = self.memory_config();
        self.memory.read(addr, &memory_cfg, self.expansion_ram_source())
    }

    /// Debug/diagnostic: one byte of a physical RAM bank, whatever the
    /// current CPU mapping, ROMs ignored.
    pub fn read_physical_ram(&self, page: crate::memory::PageNumber, bank: crate::memory::BankNumber, offset: u16) -> u8 {
        use crate::memory::{BankConfig, MemoryConfig, RamConfig};
        let mut config = MemoryConfig::default();
        config.1 = RamConfig([BankConfig(page, bank); 4]);
        self.memory.read(offset % 0x4000, &config, self.expansion_ram_source())
    }

    /// Read a byte the way the video hardware sees it: always from RAM, even
    /// where a ROM is currently overlaid for the CPU (the gate array fetches
    /// screen bytes from RAM regardless of the CPU's ROM mapping), and
    /// always from the CPC's fixed base 64K (banks 0-3), regardless of
    /// whatever extra RAM page the CPU's own MMR paging currently has
    /// banked in. On real hardware the gate array's video-fetch address
    /// lines are wired only to the base RAM ICs, entirely independently of
    /// the CPU's own memory mapping - a demo using the MMR to page in an
    /// extra 64K page (e.g. to hold an off-screen layer) never makes that
    /// page visible to the CRTC/GA, only to the CPU. So this deliberately
    /// never consults `self.memory_config()` at all: the standard identity
    /// config (`MemoryConfig::default()`, ROMs off, base Bank0-3) is the
    /// video hardware's *only* view, full stop.
    #[inline]
    pub fn read_video_ram_byte(&self, addr: u16) -> u8 {
        self.memory.read(addr, &MemoryConfig::default(), self.expansion_ram_source())
    }

    /// Read a byte as if the lower ROM was mapped in (and no upper ROM),
    /// whatever the CPU's current mapping is - e.g. to reach the firmware
    /// character matrix at &3800 while a ROM-disabled routine is running.
    #[inline]
    pub fn read_lower_rom_byte(&self, addr: u16) -> u8 {
        self.read_byte_with_roms(addr, Flag::Enable, Flag::Disable)
    }

    fn read_byte_with_roms(&self, addr: u16, lower_rom: Flag, upper_rom: Flag) -> u8 {
        let MemoryConfig(rom, ram) = self.memory_config();
        let config = MemoryConfig(
            RomConfig {
                lower_rom,
                upper_rom,
                rom_number: rom.rom_number
            },
            ram
        );
        self.memory.read(addr, &config, self.expansion_ram_source())
    }

    #[inline]
    pub fn read_word(&self, addr: u16) -> u16 {
        let memory_cfg = self.memory_config();

        let low = self.memory.read(addr, &memory_cfg, self.expansion_ram_source()) as u16;
        let high = self.memory.read(addr.wrapping_add(1), &memory_cfg, self.expansion_ram_source()) as u16;

        256 * high + low
    }
}

impl Motherboard {
    /// Return the byte read from a chip
    /// TODO handle the case where there is a read conflict over several chips
    fn input_byte(&mut self, addr: u16) -> u8 {
        if self.ppi().must_execute_tick() {
            self.ppi().get_data_from_pins()
        } else if self.ga().must_execute_tick() {
            self.ga().get_data_from_pins()
        } else if self.crtc().must_execute_tick() {
            self.crtc().get_data_from_pins()
        } else if Self::is_fdc_status_addr(addr) {
            self.fdc.read_status()
        } else if Self::is_fdc_data_addr(addr) {
            self.fdc.read_data()
        } else {
            self.floating_bus
        }
    }
}

impl Motherboard {
    delegate::delegate! {
        to self.cpu {
            pub fn pc(&self) -> u16;
            pub fn sp(&self) -> u16;
            pub fn af(&self) -> u16;
            pub fn bc(&self) -> u16;
            pub fn de(&self) -> u16;
            pub fn hl(&self) -> u16;
            pub fn ix(&self) -> u16;
            pub fn iy(&self) -> u16;


    pub fn a(&self) -> u8;
    pub fn b(&self) -> u8;
    pub fn d(&self) -> u8;
    pub fn h(&self) -> u8;
    pub fn ixh(&self) -> u8;
    pub fn iyh(&self) -> u8;
    pub fn c(&self) -> u8;
    pub fn e(&self) -> u8;
    pub fn l(&self) -> u8;
    pub fn x(&self) -> u8;
    pub fn y(&self) -> u8;

            pub fn set_pc(&mut self, value: u16);
            pub fn set_sp(&mut self, value: u16);
            pub fn set_af(&mut self, value: u16);
            pub fn set_bc(&mut self, value: u16);
            pub fn set_de(&mut self, value: u16);
            pub fn set_hl(&mut self, value: u16);
            pub fn set_ix(&mut self, value: u16);
            pub fn set_iy(&mut self, value: u16);

            pub fn is_aligned_with_new_instruction(&self) -> bool;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_pins_consistency() {
        let mut motherboard = Motherboard::default();
        motherboard.cpu.set_data_on_pins(0xfe);
        motherboard.sync_chips_pins();
        assert_eq!(motherboard.ga.get_data_from_pins(), 0xfe);
    }

    #[test]
    fn test_nop_execution() {
        let mut motherboard = Motherboard::default();
        let memory_config = motherboard.memory_config();

        // setup pins
        motherboard.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA !!
                                                         // install the program
        motherboard.cpu.pc.set(0x4000);
        motherboard.memory.write(0x4000, 0x00, &memory_config, None);

        assert!(
            motherboard.cpu.next_state.is_new_opcode(),
            "We have to start by a new opcode"
        );
        assert!(motherboard.cpu.is_activated());

        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(
            video.len(),
            16 // 4 t-states for 4 mcycles
        );

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4001,
            "{:x} != {:x} ",
            motherboard.cpu.pc.value(),
            0x4001
        );
    }

    #[test]
    fn test_xor_a_execution() {
        let mut motherboard = Motherboard::default();
        let memory_config = motherboard.memory_config();

        // setup pins
        motherboard.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA !!
                                                         // install the program
        motherboard.cpu.pc.set(0x4000);
        motherboard.cpu.af.set(0xffff);
        motherboard.memory.write(0x4000, 0xaf, &memory_config, None);

        assert!(
            motherboard.cpu.next_state.is_new_opcode(),
            "We have to start by a new opcode"
        );
        assert!(motherboard.cpu.is_activated());

        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(
            video.len(),
            16 // 4 t-states for 4 mcycles
        );

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4001,
            "{:x} != {:x} ",
            motherboard.cpu.pc.value(),
            0x4001
        );

        assert_eq!(motherboard.cpu.af.high().value(), 0);
    }

    #[test]
    fn test_rom_config() {
        let mut motherboard = Motherboard::default();

        let memory_config = motherboard.memory_config();
        assert_eq!(memory_config.0.lower_rom, Flag::Enable);
        assert_eq!(memory_config.0.upper_rom, Flag::Enable);

        motherboard.disable_lower_rom();
        let memory_config = motherboard.memory_config();
        assert_eq!(memory_config.0.lower_rom, Flag::Disable);
        assert_eq!(memory_config.0.upper_rom, Flag::Enable);

        motherboard.disable_upper_rom();
        let memory_config = motherboard.memory_config();
        assert_eq!(memory_config.0.lower_rom, Flag::Disable);
        assert_eq!(memory_config.0.upper_rom, Flag::Disable);
    }

    #[test]
    fn test_motherboard_builder() {
        let motherboard = MotherboardBuilder::default()
            .disable_lower_rom()
            .disable_upper_rom()
            .build();
        let memory_config = motherboard.memory_config();
        assert_eq!(memory_config.0.lower_rom, Flag::Disable);
        assert_eq!(memory_config.0.upper_rom, Flag::Disable);
    }

    #[test]
    fn test_hl_dead_execution() {
        let mut motherboard = Motherboard::default();
        let memory_config = motherboard.memory_config();

        // setup pins
        // install the program
        motherboard.cpu.pc.set(0x4000);
        motherboard.cpu.hl.set(0xff);
        motherboard.memory.write(0x4000, 0x21, &memory_config, None);
        motherboard.memory.write(0x4001, 0xad, &memory_config, None);
        motherboard.memory.write(0x4002, 0xde, &memory_config, None);

        assert!(
            motherboard.cpu.next_state.is_new_opcode(),
            "We have to start by a new opcode"
        );
        assert!(motherboard.cpu.is_activated());

        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(motherboard.cpu.hl.value(), 0xdead);

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4003,
            "{:x} != {:x} ",
            motherboard.cpu.pc.value(),
            0x4003
        );

        assert_eq!(video.len(), 4 * 3 * 4);
    }

    #[test]
    fn test_chips_activation() {
        let mut motherboard = Motherboard::default();

        // setup pins
        motherboard.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA

        let mut count_cpu = 0;
        let mut count_crtc = 0;
        for i in 0..32 {
            println!(
                "{},{},{}",
                true,
                motherboard.cpu.is_activated(),
                motherboard.crtc.is_activated()
            );

            if motherboard.cpu.is_activated() {
                count_cpu += 1;
            }

            if motherboard.crtc.is_activated() {
                count_crtc += 1;
            }

            motherboard.ga.manage_clocks();
            motherboard.sync_chips_pins();
        }

        assert_eq!(count_cpu, 32 / 4);

        assert_eq!(count_crtc, 32 / 16);
    }



    #[test]
    fn motherboard_execute_step_nop() {
        let mut motherboard = MotherboardBuilder::default()
                                .disable_lower_rom()
                                .disable_upper_rom()
                                .build();

        // full of nop, no need to inject
        motherboard.set_pc(0x1000);
        motherboard.execute_next_opcode(); // ensure state is correct (it seems emu ini is wrong)

        motherboard.nops_counter_mut().reset();
        motherboard.set_pc(0x1000);
        let res = motherboard.execute_next_opcode();
        assert_eq!(
            motherboard.pc(),
            0x1001
        );
        assert_eq!(
            res.len(),
            16
        );
        assert_eq!(
            motherboard.nops_counter().value(),
            1
        );
    }

    #[test]
    fn motherboard_execute_step_halt_no_inter() {
        let mut motherboard = MotherboardBuilder::default()
                                .disable_lower_rom()
                                .disable_upper_rom()
                                .build();

         motherboard.nops_counter_mut().reset();
        motherboard.set_pc(0x1000);
        motherboard.cpu_mut().force_interruption_received(true);


        motherboard.write_byte(0x1000, 0x76);
        let res = motherboard.execute_next_opcode();
        assert_eq!(
            motherboard.pc(),
            0x1000
        );
        assert_eq!(
            res.len(),
            16
        );
        assert_eq!(
            motherboard.nops_counter().value(),
            1
        );
    }

    #[test]
    fn motherboard_execute_step_halt() {
        let mut motherboard = MotherboardBuilder::default()
                                .disable_lower_rom()
                                .disable_upper_rom()
                                .enable_interruptions()
                                .build();


        motherboard.nops_counter_mut().reset();
        motherboard.set_pc(0x1000);
        motherboard.cpu_mut().force_interruption_received(true);
        motherboard.cpu_mut().iff1 = true;
        motherboard.cpu_mut().iff2 = true;

        motherboard.write_byte(0x1000, 0x76);
        let res = motherboard.execute_next_opcode();
        dbg!(&motherboard.cpu.next_state);
        assert_eq!(
            motherboard.pc(),
            0x38
        );
        assert_eq!(
            motherboard.nops_counter().value(),
            1 + 5
        );
    }

}
