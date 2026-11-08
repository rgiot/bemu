use smallvec::*;

use crate::chip::*;
use crate::counter::*;
use crate::crtc::*;
use crate::debug::DisassembledInstruction;
use crate::expansion::{LargeRamExpansion, LargeRamExpansionPin, RamBankExpansion};
use crate::expansion_port::{
    CpcSide, ExpansionDevice, ExpansionPin, ExpansionPort, PrinterDevice, PrinterPort,
};
use crate::fdc::{Fdc, FdcDrive};
use crate::ga::*;
use crate::keyboard::KeyboardMatrix;
use crate::memory::*;
use crate::pal::*;
use crate::ppi::*;
use crate::psg::{AudioConfig, BusControl, Psg, REG_IO_PORT_A};
use crate::z80::*;

use std::fmt;

/// Ticks from the start of an input cycle (the T1 of this model's Input M-cycle, which opens the
/// microsecond of the real cycle's T3) to the Z80 latching the data bus, on the falling edge of
/// that T3 (Zilog Z80 CPU User Manual, Input or Output Cycles: `IN A,(C)`'s T3 is the 3rd T-state
/// of its 4th microsecond once stretched by /WAIT, ACCC 7.2). A signal read through a port, the
/// VSYNC on PPI port B first, is seen as it is 10 ticks into that microsecond.
const INPUT_LATCH_TICKS: u32 = 10;

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
    /// The CPC+'s ASIC (its lock, register page, interrupts, DMA): `None` on a CPC.
    asic: Option<Box<crate::asic::Asic>>,
    /// A card plugged into the expansion port extending RAM banking beyond
    /// the base machine's own 512K limit, if any (see `src/expansion.rs`).
    /// `None` is a stock machine - the common case, and the default.
    large_ram_expansion: Option<LargeRamExpansion>,
    fdc: Fdc,
    /// The cassette deck, with a tape in it.
    tape: Option<Box<crate::tape::TapeDeck>>,
    /// What port B's bit 7 (the cassette input) reads with no tape playing: 0 on the CPCs with a
    /// cassette port, 1 on the CPC+ (CRTC 3), which has none (Shaker A4 prints port B as #DE/#DF on
    /// a real 6128+).
    cassette_idle: bool,
    /// The expansion port, when a card is plugged on it (a PlayCity).
    expansion_port: Option<Box<ExpansionPort>>,
    /// The printer port, when a device is plugged on it (a Digiblaster).
    printer_port: Option<Box<PrinterPort>>,
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
    /// Microseconds since the start and the PC jump trace (AMSRUST_TRACE_PC=<from us>)
    us: u64,
    trace_pc_from: Option<u64>,
    trace_pc_to: u64,
    trace_pc_min: i32,
    last_traced_pc: u16,
    fdc_read_active: bool,
    /// What a read of a port nothing decodes returns.
    floating_bus: u8,
    /// The byte latched onto the data bus for the currently active input
    /// cycle (see `fdc_read_active`) - held steady and reused across the
    /// cycle's repeated ticks instead of re-deriving it (which would
    /// re-invoke `Fdc::read_data` and advance its internal state each time).
    cached_input_byte: u8,
    /// Ticks since the current input cycle began (see `INPUT_LATCH_TICKS`).
    input_ticks: u32,

    psg: Psg,
    keyboard: KeyboardMatrix,

    /// The CPU's bus accesses, recorded for debuggers (watchpoints) while
    /// `Some` - see [`Motherboard::watch_bus`].
    bus_log: Option<Vec<BusAccess>>,
    /// The files the discs of drives A and B are saved to when the FDC wrote to them (see
    /// [`Motherboard::set_disc_file`]), and those saved since [`Motherboard::take_saved_disc_files`].
    disc_files: [Option<String>; 2],
    saved_disc_files: Vec<String>,
    /// The I/O writes `(port, value)` since they were last taken, when watched.
    io_writes: Option<Vec<(u16, u8)>>,

    model: CpcModel,
    /// The CRTC of the CPC before it became a CPC+ ([`Self::change_model`]).
    crtc_before_plus: CrtcType,
    /// The MMR banks RAM: a 6128, or a RAM expansion on a 464/664.
    ram_banking: bool,
    /// A disc interface (AMSDOS and the FDC ports): a 664/6128, or a
    /// DDI-1 on a 464.
    disc_interface: bool,
    /// The ROMs' language, so what the keys type.
    keyboard_layout: crate::keyboard::KeyboardLayout,

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
            asic: None,
            large_ram_expansion: None,
            fdc: {
                // the drive turns and steps like a real one, unless AMSRUST_FAST_DISC is set
                let mut fdc = Fdc::default();
                fdc.set_realistic_timing(std::env::var("AMSRUST_FAST_DISC").is_err());
                if std::env::var("AMSRUST_FDC_LOG").is_ok() {
                    fdc.start_command_log();
                }
                fdc
            },
            tape: None,
            cassette_idle: false,
            expansion_port: None,
            printer_port: None,
            fdc_write_active: false,
            us: 0,
            trace_pc_from: std::env::var("AMSRUST_TRACE_PC")
                .ok()
                .and_then(|v| v.parse().ok()),
            trace_pc_to: std::env::var("AMSRUST_TRACE_PC_TO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(u64::MAX),
            trace_pc_min: std::env::var("AMSRUST_TRACE_PC_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            last_traced_pc: 0,
            fdc_read_active: false,
            floating_bus: 0xff,
            cached_input_byte: 0xff,
            input_ticks: 0,
            psg: Default::default(),
            keyboard: Default::default(),
            bus_log: None,
            disc_files: [None, None],
            saved_disc_files: Vec::new(),
            io_writes: None,
            model: CpcModel::default(),
            crtc_before_plus: CrtcType::Crtc0,
            ram_banking: true,
            disc_interface: true,
            keyboard_layout: Default::default(),

            t_state_counter: Default::default(),
            nops_counter: Default::default(),
        };
        motherboard.stabilize_run();
        motherboard
    }
}

/// The CPC models: their ROMs (OS, BASIC 1.0 or 1.1, AMSDOS on the disc
/// machines) and whether the PAL banking the 6128's second 64K is fitted. The CPC+ (464+, 6128+,
/// and the GX4000 console) has its ASIC (CRTC 3) and a cartridge instead of ROMs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CpcModel {
    Cpc464,
    Cpc664,
    #[default]
    Cpc6128,
    Cpc464Plus,
    Cpc6128Plus,
    Gx4000,
}

impl CpcModel {
    pub const ALL: [CpcModel; 6] = [
        CpcModel::Cpc464,
        CpcModel::Cpc664,
        CpcModel::Cpc6128,
        CpcModel::Cpc464Plus,
        CpcModel::Cpc6128Plus,
        CpcModel::Gx4000,
    ];

    /// "464", "664", "6128", "464plus", "6128plus" or "gx4000".
    pub fn name(&self) -> &'static str {
        match self {
            CpcModel::Cpc464 => "464",
            CpcModel::Cpc664 => "664",
            CpcModel::Cpc6128 => "6128",
            CpcModel::Cpc464Plus => "464plus",
            CpcModel::Cpc6128Plus => "6128plus",
            CpcModel::Gx4000 => "gx4000",
        }
    }

    /// The BASIC in ROM: 10 for 1.0, 11 for 1.1 (the CPC+'s cartridge has its own 1.1).
    pub fn basic_version(&self) -> u8 {
        match self {
            CpcModel::Cpc464 => 10,
            _ => 11,
        }
    }

    /// Whether the MMR (&7Fxx, &C0-&FF) banks RAM as bought: the 6128's (and 6128+'s) second
    /// 64K. A 464/664 gets it from a RAM expansion.
    pub fn has_ram_banking(&self) -> bool {
        matches!(self, CpcModel::Cpc6128 | CpcModel::Cpc6128Plus)
    }

    /// Whether a disc interface is built in (664, 6128, 6128+). A 464 gets one
    /// from a DDI-1.
    pub fn has_disc_interface(&self) -> bool {
        matches!(
            self,
            CpcModel::Cpc664 | CpcModel::Cpc6128 | CpcModel::Cpc6128Plus
        )
    }

    /// A CPC+ (or the GX4000): the ASIC, CRTC 3, and a cartridge.
    pub fn is_plus(&self) -> bool {
        matches!(
            self,
            CpcModel::Cpc464Plus | CpcModel::Cpc6128Plus | CpcModel::Gx4000
        )
    }
}

impl std::str::FromStr for CpcModel {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        // "cpc6128", "6128+", "6128 Plus"...
        let name = text
            .to_ascii_lowercase()
            .replace([' ', '-', '_'], "")
            .replace('+', "plus");
        let name = name.trim_start_matches("cpc");
        CpcModel::ALL
            .into_iter()
            .find(|model| model.name() == name)
            .ok_or_else(|| {
                format!("unknown CPC model \"{text}\": 464, 664, 6128, 464plus, 6128plus or gx4000")
            })
    }
}

/// One access of the CPU to the memory or an IO port, as recorded by
/// [`Motherboard::watch_bus`]. Opcode fetches (M1 cycles) are not memory
/// reads here: a debugger watches data, and breaks on code with execution
/// breakpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusAccess {
    pub kind: BusAccessKind,
    pub address: u16,
    pub value: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusAccessKind {
    MemoryRead,
    MemoryWrite,
    IoRead,
    IoWrite,
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
    keyboard_layout: Option<crate::keyboard::KeyboardLayout>,
    #[cfg(feature = "playcity")]
    playcity: bool,
    #[cfg(feature = "digiblaster")]
    digiblaster: bool,
    /// A DMP2000 printer on the printer port.
    dmp2000: bool,
    /// The files it writes by itself, once idle.
    dmp2000_export: Option<&'static str>,
    dmp2000_charset: u8,
    /// Another EPROM than the one embedded, for the printer's characters.
    dmp2000_font: Option<&'static [u8]>,
    dmp2000_realtime: bool,
    /// Willy with an OPL3LPT, at this base.
    #[cfg(feature = "willy")]
    willy_opl3: Option<u16>,
    /// If interruption are allowed
    enable_interruptions: bool, // false by default
    disable_lower_rom: bool, // false by default
    disable_upper_rom: bool, // false by defualt
    extra_ram_pages: Option<usize>,
    large_ram_expansion_banks: Option<usize>,
    model: Option<CpcModel>,
    disc_interface: Option<bool>,
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

    /// The machine: its ROMs and RAM banking (6128 if unset).
    pub fn set_model(&mut self, model: CpcModel) -> &mut Self {
        self.model = Some(model);
        self
    }

    /// Fit (a DDI-1 on a 464) or remove the disc interface: the model's
    /// own if unset.
    pub fn set_disc_interface(&mut self, fitted: bool) -> &mut Self {
        self.disc_interface = Some(fitted);
        self
    }

    /// The French ROMs (AZERTY keyboard), on a 6128 only, or the English ones.
    pub fn set_keyboard_layout(&mut self, layout: crate::keyboard::KeyboardLayout) -> &mut Self {
        self.keyboard_layout = Some(layout);
        self
    }

    /// Plug a PlayCity (two more AYs and a CTC).
    #[cfg(feature = "playcity")]
    pub fn plug_playcity(&mut self) -> &mut Self {
        self.playcity = true;
        self
    }

    /// Plug Willy with an OPL3LPT (an OPL3) at `base` (&FEBC usually).
    #[cfg(feature = "willy")]
    pub fn plug_willy_opl3(&mut self, base: u16) -> &mut Self {
        self.willy_opl3 = Some(base);
        self
    }

    /// Plug an Amstrad DMP2000 printer on the printer port.
    pub fn plug_dmp2000(&mut self) -> &mut Self {
        self.dmp2000 = true;
        self
    }

    /// The DMP2000's EPROM (its characters).
    pub fn set_dmp2000_font(&mut self, eprom: &'static [u8]) -> &mut Self {
        self.dmp2000_font = Some(eprom);
        self
    }

    /// The DMP2000's character set (ESC R n's numbers), and whether it takes its real time.
    pub fn set_dmp2000(&mut self, charset: u8, realtime: bool) -> &mut Self {
        self.dmp2000_charset = charset;
        self.dmp2000_realtime = realtime;
        self
    }

    /// The DMP2000 writes its files (`prefix.txt`...) by itself, a while after its last byte.
    pub fn export_dmp2000_to(&mut self, prefix: &'static str) -> &mut Self {
        self.dmp2000_export = Some(prefix);
        self
    }

    /// Plug a Digiblaster (an 8-bit DAC on the printer port).
    #[cfg(feature = "digiblaster")]
    pub fn plug_digiblaster(&mut self) -> &mut Self {
        self.digiblaster = true;
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

    /// The CRTC chosen (the default one if unset).
    pub fn crtc(&self) -> CrtcType {
        self.crtc_type.unwrap_or_default()
    }

    /// The Gate Array chosen (the default one if unset).
    pub fn gate_array_type(&self) -> crate::ga::GateArrayType {
        self.ga_type.unwrap_or_default()
    }

    pub fn build(self) -> Motherboard {
        // the CPC+'s CRTC is its ASIC's
        let crtc_type = match self.model {
            Some(model) if model.is_plus() => CrtcType::Crtc3,
            _ => self.crtc_type.unwrap_or_default(),
        };

        let mut motherboard = Motherboard {
            crtc: crtc_type.into(),
            ..Default::default()
        };

        if let Some(model) = self.model {
            motherboard.set_model(model);
        }
        if let Some(fitted) = self.disc_interface {
            motherboard.set_disc_interface(fitted);
        }
        if let Some(layout) = self.keyboard_layout {
            if let Err(e) = motherboard.set_keyboard_layout(layout) {
                eprintln!("{e}");
            }
        }
        #[cfg(feature = "playcity")]
        if self.playcity {
            motherboard.plug_expansion(Box::new(crate::sound_cards::PlayCity::default()));
        }
        #[cfg(feature = "willy")]
        if let Some(base) = self.willy_opl3 {
            motherboard.plug_expansion(Box::new(crate::willy::Opl3Lpt::new(base)));
        }
        #[cfg(feature = "digiblaster")]
        if self.digiblaster {
            motherboard.plug_printer(Box::new(crate::sound_cards::Digiblaster::default()));
        }
        if self.dmp2000 {
            let mut printer = crate::dmp2000::Dmp2000::with_os_rom(crate::roms::OS6128);
            if let Some(prefix) = self.dmp2000_export {
                printer.export_to(prefix);
            }
            // its own characters: the EPROM it has, or the one it is given
            if let Err(e) = printer.load_font_rom(self.dmp2000_font.unwrap_or(crate::roms::DMP2000))
            {
                eprintln!("DMP2000's EPROM: {e}");
            }
            printer.set_international_set(self.dmp2000_charset);
            printer.set_realtime(self.dmp2000_realtime);
            motherboard.plug_printer(Box::new(printer));
        }
        if let Some(brand) = self.brand {
            motherboard.ppi.set_brand(brand);
        }
        if let Some(ga_type) = self.ga_type {
            motherboard.ga.set_gate_array_type(ga_type);
        }
        motherboard.fit_crtc_chips(crtc_type);

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
            motherboard.cpu.iff1 = true;
            motherboard.cpu.iff2 = true;
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

    /// Generate the picture (the default), or not: frames nobody sees (a fast-forward) run faster
    /// without it, with no other difference to the machine. See [`GateArray::set_video`].
    pub fn set_video(&mut self, on: bool) {
        self.ga.set_video(on);
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

    /// The PAL's memory mapping register (RAM banking), as last written.
    /// The upper ROM selected (the last number written to &DFxx).
    pub fn upper_rom_number(&self) -> u8 {
        self.rom_selector.upper_rom_number()
    }

    pub fn mmr(&self) -> u8 {
        self.pal.mmr()
    }

    /// Set the MMR as if written to (a snapshot's), when RAM is banked.
    pub fn force_mmr(&mut self, value: u8) {
        if self.ram_banking {
            self.pal.force_mmr(value);
        }
    }

    pub fn fdc(&self) -> &Fdc {
        &self.fdc
    }

    pub fn fdc_mut(&mut self) -> &mut Fdc {
        &mut self.fdc
    }

    /// Save the disc of `drive` to `file` (a DSK, an HFE, or `archive.zip[#entry]`) whenever the
    /// FDC wrote to it: when the motor stops, when another disc replaces it
    /// ([`Motherboard::insert_disc`]), and on [`Motherboard::save_modified_discs`]. None: never.
    pub fn set_disc_file(&mut self, drive: FdcDrive, file: Option<String>) {
        self.disc_files[drive as usize] = file;
    }

    /// Save the discs the FDC wrote to, to their files (see [`Motherboard::set_disc_file`]).
    pub fn save_modified_discs(&mut self) {
        for drive in [FdcDrive::A, FdcDrive::B] {
            self.save_disc(drive);
        }
    }

    fn save_disc(&mut self, drive: FdcDrive) {
        let Some(file) = self.disc_files[drive as usize].clone() else {
            return;
        };
        if !self.fdc.is_modified(drive) {
            return;
        }
        let Some(image) = self.fdc.image(drive) else {
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        match crate::disc_image::save_disc_file(&file, image) {
            Ok(()) => {
                self.fdc.mark_saved(drive);
                eprintln!("{file}: saved");
                self.saved_disc_files.push(file);
            }
            Err(e) => eprintln!("cannot save the disc: {e}"),
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (file, image);
    }

    /// The files saved since the last call (a watcher should not take them for outside changes).
    pub fn take_saved_disc_files(&mut self) -> Vec<String> {
        std::mem::take(&mut self.saved_disc_files)
    }

    /// Put `image` in `drive` (None ejects), the disc there saved first if it was written to.
    pub fn insert_disc(&mut self, drive: FdcDrive, image: Option<crate::disc_image::DiscImage>) {
        self.save_disc(drive);
        self.fdc.insert_image(drive, image);
    }

    /// Put `tape` in the cassette deck (rewound, PLAY down: it plays when the CPC starts the
    /// motor); `None` ejects it.
    pub fn insert_tape(&mut self, tape: Option<crate::tape::Tape>) {
        self.tape = tape.map(|tape| Box::new(crate::tape::TapeDeck::new(tape)));
        if self.tape.is_none() {
            self.ppi.set_pin(PpiPin::B7, self.cassette_idle);
        }
    }

    /// The cassette deck of `other` comes here, as it is (a reset machine keeps its deck).
    pub fn take_tape_deck_from(&mut self, other: &mut Motherboard) {
        self.tape = other.tape.take();
    }

    /// Plug a card on the expansion port (after the ones already there).
    pub fn plug_expansion(&mut self, device: Box<dyn ExpansionDevice>) {
        self.expansion_port
            .get_or_insert_with(Box::default)
            .plug(device);
        self.update_sound_mix();
    }

    /// Plug a device on the printer port (in place of the one there).
    pub fn plug_printer(&mut self, device: Box<dyn PrinterDevice>) {
        self.printer_port
            .get_or_insert_with(Box::default)
            .plug(device);
        self.update_sound_mix();
    }

    /// A card on the expansion port, or a device making sound on the printer port.
    fn sound_cards_fitted(&self) -> bool {
        self.expansion_port.is_some()
            || self
                .printer_port
                .as_ref()
                .is_some_and(|port| port.has_sound())
    }

    /// The AY's share of the sound: all of it alone, half with cards that make sound.
    fn update_sound_mix(&mut self) {
        let cards = self.sound_cards_fitted();
        self.psg
            .set_extra_sound([0.0; 2], if cards { 0.5 } else { 1.0 });
    }

    pub fn expansion_port(&self) -> Option<&ExpansionPort> {
        self.expansion_port.as_deref()
    }

    pub fn expansion_port_mut(&mut self) -> Option<&mut ExpansionPort> {
        self.expansion_port.as_deref_mut()
    }

    pub fn printer_port_mut(&mut self) -> Option<&mut PrinterPort> {
        self.printer_port.as_deref_mut()
    }

    pub fn printer_port(&self) -> Option<&PrinterPort> {
        self.printer_port.as_deref()
    }

    /// The sound chips of the cards, named (`right`, `left` for a PlayCity), for the sound
    /// export.
    pub fn expansion_sound_chips(&mut self) -> Vec<(&'static str, &mut crate::psg::Psg)> {
        self.expansion_port
            .as_deref_mut()
            .map(|port| {
                port.devices_mut()
                    .flat_map(|device| device.sound_chips())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The CPC's side of the expansion port's pins this tick, and the cards' answer.
    fn tick_expansion_port(&mut self) {
        if self.expansion_port.is_none() {
            return;
        }
        // the gate array's /ROMEN and /RAMRD: a memory read, from a ROM or from the RAM
        let memory_read = !self.cpu.get_pin(Z80Pin::Neg_MREQ) && !self.cpu.get_pin(Z80Pin::Neg_RD);
        let address = self.cpu.get_address_from_pins();
        let rom = memory_read && {
            let config = self.memory_config();
            config.is_in_enabled_lower_rom(address) || config.is_in_enabled_upper_rom(address)
        };
        let Some(port) = &mut self.expansion_port else {
            return;
        };
        let cpu = &self.cpu;
        port.tick_with(CpcSide {
            address,
            data: cpu.get_data_from_pins(),
            mreq: cpu.get_pin(Z80Pin::Neg_MREQ),
            m1: cpu.get_pin(Z80Pin::Neg_M1),
            rfsh: cpu.get_pin(Z80Pin::Neg_RFSH),
            iorq: cpu.get_pin(Z80Pin::Neg_IORQ),
            rd: cpu.get_pin(Z80Pin::Neg_RD),
            wr: cpu.get_pin(Z80Pin::Neg_WR),
            halt: cpu.get_pin(Z80Pin::Neg_HALT),
            busak: cpu.get_pin(Z80Pin::Neg_BUSACK),
            romen: !rom,
            ramrd: !(memory_read && !rom),
            cursor: self.crtc.cursor(),
            clk4: cpu.get_pin(Z80Pin::Neg_CLK),
        });
        // /NMI: the CPU latches its falling edge
        self.cpu.set_nmi_line(port.get_pin(ExpansionPin::Neg_NMI));
    }

    /// The cards' sound for the coming microsecond.
    fn expansion_sound(&self) -> [f32; 2] {
        let mut sound = self
            .printer_port
            .as_ref()
            .map_or([0.0; 2], |port| port.sound());
        if let Some(port) = &self.expansion_port {
            for device in port.devices() {
                let [left, right] = device.sound();
                sound[0] += left;
                sound[1] += right;
            }
        }
        sound
    }

    pub fn tape_deck(&self) -> Option<&crate::tape::TapeDeck> {
        self.tape.as_deref()
    }

    pub fn tape_deck_mut(&mut self) -> Option<&mut crate::tape::TapeDeck> {
        self.tape.as_deref_mut()
    }

    pub fn psg(&self) -> &Psg {
        &self.psg
    }

    pub fn psg_mut(&mut self) -> &mut Psg {
        &mut self.psg
    }

    /// Start (`Some`) or stop (`None`) producing PSG audio samples - see
    /// [`AudioConfig`] for the format.
    pub fn set_audio_output(&mut self, config: Option<AudioConfig>) {
        self.psg.set_audio_output(config);
    }

    /// The PSG audio samples produced since the last call (interleaved
    /// left/right in stereo); empty when no audio output is configured.
    pub fn take_audio_samples(&mut self) -> Vec<f32> {
        self.psg.take_samples()
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
    /// Press the reset button: the chips are reset and the CPU restarts at
    /// 0, as at power on (RAM is kept). [`Self::reset`] alone leaves the
    /// CPU's /WAIT low: it never runs again.
    pub fn restart(&mut self) {
        self.stabilize_run();
    }

    /// Power the machine off and on: [`Self::restart`] with every RAM
    /// bank (extra pages and expansion included) cleared.
    pub fn power_cycle(&mut self) {
        for bank in 0..self.physical_bank_count() {
            for offset in 0..0x4000 {
                self.write_bank_byte(bank, offset, 0);
            }
        }
        self.restart();
    }

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
        if let Some(asic) = &mut self.asic {
            asic.reset();
        }
        self.pal.reset();
        if let Some(expansion) = &mut self.large_ram_expansion {
            expansion.reset();
        }
        if let Some(port) = &mut self.expansion_port {
            port.reset();
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

        if self.expansion_port.is_some() {
            self.tick_expansion_port();
        }
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
            self.crtc.set_phase_ticks(0);
            self.fdc.advance(1); // one character = one microsecond
            if self.sound_cards_fitted() {
                self.psg.set_extra_sound(self.expansion_sound(), 0.5);
            }
            if let Some(printer) = &mut self.printer_port {
                printer.advance_us();
                if let Some(busy) = printer.busy() {
                    self.ppi.set_pin(PpiPin::B6, busy);
                }
            }
            if let Some(tape) = &mut self.tape {
                // the cassette motor relay is port C's bit 4, the head's level port B's bit 7
                tape.advance_us(self.ppi.get_pin(PpiPin::C4));
                self.ppi.set_pin(PpiPin::B7, tape.level());
            }
            self.us += 1;
            if let Some(from) = self.trace_pc_from {
                if self.us >= from && self.us < self.trace_pc_to {
                    let pc = self.cpu.pc();
                    if (i32::from(pc) - i32::from(self.last_traced_pc)).abs() > self.trace_pc_min {
                        eprintln!(
                            "PC {} {:04X} sp={:04X}",
                            self.us,
                            pc,
                            self.cpu.reg16(Reg16::Sp).value()
                        );
                    }
                    self.last_traced_pc = pc;
                }
            }
            // a register access present on the character clock edge is taken before the character
            // is processed (an OUTI's I/O, from the start of its microsecond, counts for it)
            if self.crtc.must_execute_tick() {
                self.crtc.execute_tick();
            }
            if let Some(plus) = self.ga.plus() {
                self.crtc.set_plus_vertical_scroll(plus.sscr >> 4);
            }
            if let Some(asic) = &self.asic {
                self.crtc.set_plus_split(asic.split());
            }
            self.crtc.tick(); // Virtually uses 16 cycles
            if let Some(asic) = &mut self.asic {
                // the CPC+: raster interrupt, DMA, split (the DMA reads the base 64K)
                let Motherboard {
                    crtc,
                    ga,
                    psg,
                    memory,
                    ..
                } = self;
                asic.microsecond(crtc, ga, psg, |address| {
                    memory.read(address, &MemoryConfig::default(), None)
                });
            }
        } else {
            self.crtc
                .set_phase_ticks(self.crtc.phase_ticks().saturating_add(1));
            if self.crtc.must_execute_tick() {
                self.crtc.execute_tick();
            }
        }
        // The gate array blanks the pixels the very tick the CRTC drops
        // DE: it fetches the video bytes from the CRTC's address pins of the
        // current tick, so seeing DE one tick late made the border start (and
        // the paper end) a pixel late - the first pixel of the next fetched
        // byte leaked at the right edge of every line and the first pixel of
        // the line was lost.
        let display = self.crtc.get_pin(CrtcPin::DE)
            && !((self.crtc.second_byte_is_border()
                || self.crtc.r6_conflict_second_byte_is_border())
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
        // (no picture, no fetch: see `set_video`)
        if self.ga.video() && self.ga.need_memory_byte() {
            let address = self
                .ga
                .video_fetch_address(self.crtc.get_address_from_pins());
            let byte = self.read_video_ram_byte(address);
            self.ga.set_memory_byte(byte);
            if self.asic.is_some() {
                let counters = self.crtc.counters();
                if let Some(plus) = self.ga.plus_mut() {
                    plus.byte_fetched(counters.hcc, address & 1 != 0, counters.vcc, counters.vlc);
                }
            }
        }
        if let Some(asic) = &mut self.asic {
            if self.cpu.get_pin(Z80Pin::Neg_M1) || self.cpu.get_pin(Z80Pin::Neg_IORQ) {
                asic.note_ga_interrupt(!self.ga.get_pin(GateArrayPin::INTERRUPT));
            }
        }
        self.ga.set_last_idle_t(self.cpu.last_idle_t());
        self.ga
            .set_monitor_hsync_advance(self.crtc.hsync_late_ticks());
        self.ga
            .set_crtc_hsync_two_wide(self.crtc.hsync_blank_window());
        self.ga.tick();
        if self.ram_banking {
            self.pal.tick();
        }
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
            if self.t_state_counter.value().is_multiple_of(4) {
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
        if self.ga_request_read() {
            unimplemented!();
        } else if self.cpu_request_byte_write() {
            let addr = self.cpu.get_address_from_pins();
            let byte = self.cpu.get_data_from_pins();
            //    eprintln!("0x{:x}=0x{:x}", addr, byte);
            self.write_byte(addr, byte);
            self.log_bus(BusAccessKind::MemoryWrite, addr, byte);
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
                let addr = self.cpu.get_address_from_pins();
                let byte = self.cpu.get_data_from_pins();
                self.log_bus(BusAccessKind::IoWrite, addr, byte);
                if let Some(writes) = &mut self.io_writes {
                    writes.push((addr, byte));
                }
                if addr & 0x4300 == 0 {
                    // the CRTC's select register: the CPC+'s lock listens to it
                    if let Some(asic) = &mut self.asic {
                        if asic.crtc_select_written(byte) {
                            if let Some(video) = self.ga.plus_mut() {
                                video.unlocked = asic.unlocked();
                            }
                        }
                    }
                }
                if addr & 0x1000 == 0 {
                    // the printer port's latch
                    if let Some(printer) = &mut self.printer_port {
                        printer.latch(byte);
                    }
                }
                if !self.disc_interface {
                    // Nothing on the disc ports.
                } else if Self::is_fdc_motor_addr(addr) {
                    let was_on = self.fdc.motor();
                    self.fdc.set_motor(byte & 1 != 0);
                    // a written disc is saved when the motor stops (AMSDOS stops it about 2 s
                    // after its last access)
                    if was_on && !self.fdc.motor() {
                        self.save_modified_discs();
                    }
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
        if self.ga_request_read() {
            unimplemented!();
        } else if self.cpu_request_byte_read() {
            let addr = self.cpu.get_address_from_pins();
            let mut byte = self.read_byte(addr);
            if let Some(port) = &self.expansion_port {
                // ROMDIS and RAMDIS take the CPC's ROM and RAM off the bus: a card's byte instead
                let rom = !port.get_pin(ExpansionPin::Neg_ROMEN);
                if (rom && port.get_pin(ExpansionPin::ROMDIS))
                    || (!rom && port.get_pin(ExpansionPin::RAMDIS))
                {
                    byte = port.driven_data().unwrap_or(self.floating_bus);
                }
            }
            if self.cpu.get_pin(Z80Pin::Neg_M1) {
                self.log_bus(BusAccessKind::MemoryRead, addr, byte);
            }
            self.cpu.set_data_on_pins(byte);
            self.cpu.set_pin(Z80Pin::Neg_RD, true); // TODO do the real stuff - but here I am sure to not play with the data bus ...
            self.fdc_read_active = false;
        } else if self.cpu_request_byte_input() {
            // The request is held for the whole input cycle; `input_byte` (which can advance the
            // FDC's internal state) is called once, on the tick the Z80 latches the data bus: at
            // the end of the cycle's T3 (`INPUT_LATCH_TICKS`), the first tick for an interrupt
            // acknowledge. The byte is then held on the bus.
            self.input_ticks = if self.fdc_read_active {
                self.input_ticks + 1
            } else {
                0
            };
            let acknowledge = !self.cpu.get_pin(Z80Pin::Neg_M1);
            let latch = self.input_ticks == if acknowledge { 0 } else { INPUT_LATCH_TICKS };
            if latch {
                let addr = self.cpu.get_address_from_pins();
                self.cached_input_byte = self.input_byte(addr);
                if !acknowledge && addr & 0x4300 == 0 {
                    // an IN from the CRTC's select register: on a CPC+ the lock hears it too
                    if let (Some(asic), Some(video)) =
                        (self.asic.as_deref_mut(), self.ga.plus_mut())
                    {
                        if asic.crtc_select_written(video.bus_value) {
                            video.unlocked = asic.unlocked();
                        }
                    }
                }
                self.log_bus(BusAccessKind::IoRead, addr, self.cached_input_byte);
                if !self.cpu.get_pin(Z80Pin::Neg_M1) {
                    // an interrupt acknowledge: the IM 2 vector is what a card puts on the bus,
                    // or the CPC+'s ASIC (the interrupt's source)
                    if let Some(asic) = &mut self.asic {
                        self.cpu
                            .set_interrupt_vector(Some(asic.acknowledge(self.cpu.pc())));
                    } else if let Some(port) = &self.expansion_port {
                        self.cpu.set_interrupt_vector(port.driven_data());
                    }
                }
            }
            self.cpu.set_data_on_pins(self.cached_input_byte);
            if latch {
                self.cpu.set_pin(Z80Pin::Neg_RD, true);
            }
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
        !self.cpu.get_pin(Z80Pin::Neg_WR) && !self.cpu.get_pin(Z80Pin::Neg_MREQ)
    }

    #[inline]
    fn cpu_request_byte_input(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_RD) && !self.cpu.get_pin(Z80Pin::Neg_IORQ)
    }

    #[inline]
    fn cpu_request_byte_output(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_WR) && !self.cpu.get_pin(Z80Pin::Neg_IORQ)
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

        // execute the whole stuff as soon as it is necessary
        loop {
            on_tick(self.tick());

            has_executed_first_state_counter |= !self.cpu.next_state.is_new_opcode();

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
            self.psg
                .set_register(REG_IO_PORT_A, self.keyboard.line(row_select));
        }

        // The bus is shared: the PPI drives it when port A is an output (what the CPU wrote), the PSG
        // drives it while it is read. Both together fight (we take the AND); the latch survives it, so
        // the next PSG latch/write cycle still sees what the CPU wrote.
        let a_out = self.ppi.port_a_is_output();
        let latch = self.ppi.port_a_latch();
        let data_bus = if a_out {
            latch
        } else {
            self.ppi.get_data_a_from_pins()
        };
        if let Some(read_value) = self.psg.bus_cycle(control, data_bus) {
            self.ppi.set_port_a_input(if a_out {
                latch & read_value
            } else {
                read_value
            });
        } else if a_out {
            self.ppi.set_port_a_input(latch);
        } else {
            // port A an input, the PSG not read: nothing drives the bus, it floats high (Kevin
            // Thacker's acid test "psg inactive; PPI port A input, write PPI port A and read back")
            self.ppi.set_port_a_input(0xff);
        }

        // The PSG's CLOCK pin is wired to the gate array's 1MHz output, the
        // same signal as the CRTC's CCLK: the chip advances on its rising
        // edges (once every 16 ticks).
        self.psg.set_clock_pin(self.ga.get_pin(GateArrayPin::CCLK));
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
        }
        // /INT is wired-or: a card on the expansion port pulls it too, and the CPC+'s DMA
        // channels
        if self.expansion_port.is_some() || self.asic.is_some() {
            let card = self
                .expansion_port
                .as_ref()
                .is_none_or(|port| port.get_pin(ExpansionPin::Neg_INT));
            let dma = self.asic.as_ref().is_some_and(|asic| asic.dma_interrupt());
            let line = self.ga.get_pin(GateArrayPin::INTERRUPT) && card && !dma;
            self.cpu.set_pin(Z80Pin::Neg_INT, line);
        }
        if ga_changed {
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
            let vsync = self.crtc.get_pin(CrtcPin::VS);
            if vsync && !self.ppi.get_pin(PpiPin::B0) && self.psg.logs_frames() {
                // a frame begins: the sound export's tick
                self.psg.log_frame();
                for (_, chip) in self.expansion_sound_chips() {
                    chip.log_frame();
                }
            }
            self.ppi.set_pin(PpiPin::B0, vsync);
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

            let neg_iowr = self.cpu.get_pin(Z80Pin::Neg_WR) || self.cpu.get_pin(Z80Pin::Neg_IORQ);
            let neg_iord = self.cpu.get_pin(Z80Pin::Neg_RD) || self.cpu.get_pin(Z80Pin::Neg_IORQ);

            if !neg_iord {
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
                    Z80Pin::Neg_MREQ,
                ],
                &mut self.ga,
                &[
                    GateArrayPin::A14,
                    GateArrayPin::A15,
                    GateArrayPin::Neg_IORQ,
                    GateArrayPin::Neg_RD,
                    GateArrayPin::Neg_M1,
                    GateArrayPin::Neg_MREQ,
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
            self.pal.set_pin(PalPin::IOWR, !neg_iowr);

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
                expansion.set_pin(LargeRamExpansionPin::IOWR, !neg_iowr);
            }

            // CPU to CRTC
            // EN is active (low) during a real I/O cycle only: an interrupt
            // acknowledge also drives IORQ (together with M1) and must not
            // be seen as an access to whatever port the PC happens to spell.
            self.crtc.set_pin(
                CrtcPin::EN,
                (neg_iord & neg_iowr) | !self.cpu.get_pin(Z80Pin::Neg_M1),
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
            if let Some(plus) = self.ga.plus() {
                // the CPC+: an IN from the CRTC's select or write port writes the ASIC's value
                if !neg_iord && !self.cpu.get_pin(Z80Pin::A14) && !self.cpu.get_pin(Z80Pin::A9) {
                    self.crtc.set_data_on_pins(plus.bus_value);
                }
            }
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
            if !neg_iowr {
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
            self.ppi.set_pin(PpiPin::Neg_RD, neg_iord);
            self.ppi.set_pin(PpiPin::Neg_WR, neg_iowr);
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
    pub fn disassemble_instructions_from(
        &self,
        mut pc: u16,
        count: usize,
    ) -> Vec<DisassembledInstruction> {
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
        let mut instruction_bytes: SmallVec<[u8; 4]> = Default::default();
        let mut ir;
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
                        _ => OpcodePrefix::None,
                    };

                    if current_prefix == OpcodePrefix::None {
                        break;
                    }
                }

                OpcodePrefix::Cb | OpcodePrefix::Ed => {
                    break;
                }

                OpcodePrefix::Dd => {
                    if ir == 0xcb {
                        current_prefix = OpcodePrefix::DdCb
                    } else {
                        break;
                    }
                }

                OpcodePrefix::Fd => {
                    if ir == 0xcb {
                        current_prefix = OpcodePrefix::FdCb
                    } else {
                        break;
                    }
                }

                OpcodePrefix::DdCb | OpcodePrefix::FdCb => {
                    break;
                }
            }
        }

        // Decoded from the bits (see `z80::decoder`), like the CPU does.
        let token = if matches!(current_prefix, OpcodePrefix::DdCb | OpcodePrefix::FdCb) {
            // DD CB d op: `ir` is the displacement, the opcode follows.
            let opcode = read();
            instruction_bytes.push(opcode);
            crate::z80::decoder::with_operands(current_prefix, opcode, [ir, 0])
        } else {
            let mut operands = [0u8; 2];
            for operand in operands
                .iter_mut()
                .take(crate::z80::decoder::nb_operand_bytes(current_prefix, ir) as usize)
            {
                *operand = read();
                instruction_bytes.push(*operand);
            }
            crate::z80::decoder::with_operands(current_prefix, ir, operands)
        };

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
    /// Fill the extra RAM page `page` (a 128K snapshot's second 64K, ...).
    pub fn load_extra_ram_page(&mut self, page: usize, data: &[u8]) {
        self.memory.load_extra_page(page, data);
    }

    pub fn memory_config(&self) -> MemoryConfig {
        let mut ram_config = self.pal.get_ram_configuration();
        if let Some(expansion) = &self.large_ram_expansion {
            ram_config = ram_config.with_expansion_high_bits(expansion.ram_bank_high_bits());
        }
        // RMR2 (CPC+): the lower ROM's cartridge page and where it is (bits 4-3 at 3: at &0000,
        // the register page at &4000)
        let (lower_rom_page, lower_rom_quarter) = match self.ga.plus() {
            Some(plus) => (
                plus.rmr2 & 7,
                match plus.rmr2 >> 3 & 3 {
                    3 => 0,
                    quarter => quarter,
                },
            ),
            None => (0, 0),
        };
        MemoryConfig(
            RomConfig {
                lower_rom: self.ga.lower_rom_pagging(),
                upper_rom: self.ga.upper_rom_pagging(),
                rom_number: self.rom_selector.upper_rom_number(),
                lower_rom_page,
                lower_rom_quarter,
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

    /// Become a `model`: its ROMs, and RAM banking only on a 6128 (the
    /// MMR is then back to the base 64K). Restart it afterwards.
    pub fn set_model(&mut self, model: CpcModel) {
        self.model = model;
        if !matches!(model, CpcModel::Cpc6128 | CpcModel::Cpc6128Plus) {
            // French ROMs exist for the 6128 (and its system cartridge) only
            self.keyboard_layout = Default::default();
        }
        if model.is_plus() != self.asic.is_some() {
            self.ga.set_plus(model.is_plus());
            self.asic = model.is_plus().then(Default::default);
            if model.is_plus() && self.crtc.crtc_type() != CrtcType::Crtc3 {
                self.crtc = CrtcType::Crtc3.into();
                self.fit_crtc_chips(CrtcType::Crtc3);
            }
        }
        if let Some(plus) = self.ga.plus_mut() {
            plus.bus_value = if model == CpcModel::Cpc6128Plus {
                0x79
            } else {
                0x78
            };
        }
        self.memory.set_firmware(model, self.keyboard_layout);
        self.ram_banking = model.has_ram_banking();
        self.set_disc_interface(model.has_disc_interface());
        self.pal.reset();
    }

    /// What goes with the CRTC fitted: the CPC+ (CRTC 3) has its PPI inside the ASIC, which keeps
    /// its ports on a mode-set write, unlike a real 8255, and no cassette input; the ASICs' Gate
    /// Array syncs (CRTC 3 and 4).
    fn fit_crtc_chips(&mut self, crtc_type: CrtcType) {
        self.ppi
            .set_clears_ports_on_mode_set(crtc_type != CrtcType::Crtc3);
        self.cassette_idle = crtc_type == CrtcType::Crtc3;
        self.ppi.set_pin(PpiPin::B7, self.cassette_idle);
        self.ga
            .set_c_vsync_needs_crtc_vsync(matches!(crtc_type, CrtcType::Crtc3 | CrtcType::Crtc4));
        self.ga.set_sync_black_lag_ticks(match crtc_type {
            CrtcType::Crtc3 | CrtcType::Crtc4 => crate::ga::ASIC_SYNC_BLACK_LAG_TICKS,
            _ => crate::ga::SYNC_BLACK_LAG_TICKS,
        });
    }

    /// Fit a CRTC of another type while the machine runs: the registers (masked as the new
    /// chip stores them), the selected register and the counters go over to the new chip, as a
    /// version 3 snapshot carries them, so the picture goes on where it was. A CPC+ keeps its
    /// ASIC (CRTC 3).
    pub fn change_crtc(&mut self, crtc_type: CrtcType) -> Result<(), String> {
        if self.model.is_plus() && crtc_type != CrtcType::Crtc3 {
            return Err(format!(
                "the {} has its CRTC in its ASIC (CRTC 3)",
                self.model.name()
            ));
        }
        if self.crtc.crtc_type() == crtc_type {
            return Ok(());
        }
        let (registers, selected, counters) = (
            self.crtc.registers(),
            self.crtc.selected_register(),
            self.crtc.counters(),
        );
        let mut crtc: Crtc = crtc_type.into();
        crtc.restore(&registers, selected, Some(counters));
        self.crtc = crtc;
        self.fit_crtc_chips(crtc_type);
        Ok(())
    }

    /// Become another model while running, as [`Self::set_model`] then the reset button: the
    /// RAM, the discs, the tape, the cards on the ports and the extra RAM stay. Back from a
    /// CPC+, the CRTC is the one the CPC had before (CRTC 0 when there was none).
    pub fn change_model(&mut self, model: CpcModel) {
        if model == self.model {
            return;
        }
        let old = self.model;
        let own_kb = |model: CpcModel| if model.has_ram_banking() { 128 } else { 64 };
        let total_kb = self.total_ram_kb();
        if !old.is_plus() {
            self.crtc_before_plus = self.crtc.crtc_type();
        }
        let disc_interface = self.disc_interface;
        let leaves_plus = old.is_plus() && !model.is_plus();
        self.set_model(model);
        if leaves_plus {
            let crtc_type = self.crtc_before_plus;
            let _ = self.change_crtc(crtc_type);
        }
        // what was chosen beyond the old model's own stays: a RAM expansion (the new model's
        // own RAM otherwise), a DDI-1 added or a disc interface removed
        if total_kb != own_kb(old) {
            let _ = self.set_total_ram_kb(Some(total_kb));
        }
        if disc_interface != old.has_disc_interface() {
            self.set_disc_interface(disc_interface);
        }
        self.restart();
    }

    /// The RAM the CPU can reach in all, in KB: 64K without banking, else 64K and the extra
    /// pages.
    pub fn total_ram_kb(&self) -> usize {
        if self.ram_banking {
            64 * (1 + self.extra_ram_pages())
        } else {
            64
        }
    }

    /// The RAM in all (64K to 576K, in 64K steps): the extra pages are added or removed, the
    /// ones kept keep what they hold. The model's own RAM (64K, 128K on a 6128) is
    /// `None`.
    pub fn set_total_ram_kb(&mut self, kb: Option<usize>) -> Result<(), String> {
        let pages = match kb {
            None if self.model.has_ram_banking() => 1,
            None => 0,
            Some(kb) if kb % 64 == 0 && (64..=576).contains(&kb) => kb / 64 - 1,
            Some(kb) => return Err(format!("{kb}K: 64K to 576K, in steps of 64K")),
        };
        self.set_extra_ram_pages(pages);
        // the 6128's own 128K banks RAM, with or without extension
        if self.model.has_ram_banking() {
            self.ram_banking = true;
        }
        Ok(())
    }

    /// Unplug the first card of type `T` from the expansion port, the machine running on.
    pub fn unplug_expansion<T: ExpansionDevice>(&mut self) -> Option<Box<dyn ExpansionDevice>> {
        let port = self.expansion_port.as_deref_mut()?;
        let card = port.unplug::<T>();
        if port.is_empty() {
            self.expansion_port = None;
        }
        self.update_sound_mix();
        card
    }

    /// Whether a card of type `T` is on the expansion port.
    pub fn has_expansion<T: ExpansionDevice>(&self) -> bool {
        self.expansion_port
            .as_deref()
            .is_some_and(|port| port.device::<T>().is_some())
    }

    /// Unplug whatever is on the printer port.
    pub fn unplug_printer(&mut self) -> Option<Box<dyn PrinterDevice>> {
        let device = self.printer_port.take().and_then(|mut port| port.unplug());
        self.update_sound_mix();
        device
    }

    /// Put `cartridge` in the slot of a CPC+ (`None`: its system cartridge). Restart it
    /// afterwards, as the cartridge is changed with the power off.
    pub fn insert_cartridge(&mut self, cartridge: Option<crate::asic::Cartridge>) {
        self.memory.insert_cartridge(cartridge);
    }

    /// The CPC+'s ASIC.
    pub fn asic(&self) -> Option<&crate::asic::Asic> {
        self.asic.as_deref()
    }

    /// The CPC+'s analogue input `channel` (0-7) reads `value` (0-&3F).
    pub fn set_analogue_input(&mut self, channel: usize, value: u8) {
        if let Some(asic) = &mut self.asic {
            asic.set_analogue_input(channel, value);
        }
    }

    /// The CPC+'s ASIC as a snapshot's "CPC+" chunk; `None` on a CPC.
    pub fn asic_snapshot_chunk(&self) -> Option<Vec<u8>> {
        Some(self.asic.as_ref()?.snapshot_chunk(self.ga.plus()?))
    }

    /// Restore the CPC+'s ASIC from a snapshot's "CPC+" chunk.
    pub fn restore_asic_snapshot_chunk(&mut self, chunk: &[u8]) -> Result<(), String> {
        match (self.asic.as_deref_mut(), self.ga.plus_mut()) {
            (Some(asic), Some(video)) => asic.restore_snapshot_chunk(video, chunk),
            _ => Err("a CPC+ chunk in the snapshot of a CPC".to_owned()),
        }
    }

    /// Fit `data` as upper ROM `slot` (`None` empties the slot: selecting it gives BASIC, as
    /// on a CPC).
    /// The ROM fitted in upper ROM `slot`, if any.
    pub fn upper_rom(&self, slot: u8) -> Option<&[u8]> {
        self.memory.upper_rom(slot)
    }

    pub fn set_upper_rom(&mut self, slot: u8, data: Option<&[u8]>) -> Result<(), String> {
        self.memory.set_upper_rom(slot, data)
    }

    /// Fit or remove a disc interface (a DDI-1 on a 464): AMSDOS as upper
    /// ROM 7 and the FDC on its ports.
    pub fn set_disc_interface(&mut self, fitted: bool) {
        self.disc_interface = fitted;
        self.memory.set_amsdos(fitted);
    }

    pub fn has_disc_interface(&self) -> bool {
        self.disc_interface
    }

    /// Whether the MMR banks RAM (6128, or RAM expansion).
    pub fn has_ram_banking(&self) -> bool {
        self.ram_banking
    }

    /// Fit the French ROMs (AZERTY) or the English ones: the French ones exist for the 6128 only.
    pub fn set_keyboard_layout(
        &mut self,
        layout: crate::keyboard::KeyboardLayout,
    ) -> Result<(), String> {
        if layout == crate::keyboard::KeyboardLayout::French
            && !matches!(self.model, CpcModel::Cpc6128 | CpcModel::Cpc6128Plus)
        {
            return Err(format!(
                "there are French ROMs for the 6128 only, not the {}",
                self.model.name()
            ));
        }
        self.keyboard_layout = layout;
        self.memory.set_firmware(self.model, layout);
        Ok(())
    }

    /// What the keys type, as the ROMs fitted say.
    pub fn keyboard_layout(&self) -> crate::keyboard::KeyboardLayout {
        self.keyboard_layout
    }

    pub fn model(&self) -> CpcModel {
        self.model
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
    /// > instead - it calls this with the card's own bank count.
    pub fn set_extra_ram_pages(&mut self, pages: usize) {
        // A RAM expansion decodes the MMR, on any model.
        self.ram_banking = pages > 0;
        self.memory.set_extra_pages(pages);
    }

    /// All the RAM, as a snapshot stores it: the main 64K then, when RAM is banked, each extra
    /// page of the machine (not the banks of a large expansion card).
    pub fn ram_dump(&self) -> Vec<u8> {
        let mut ram = self.memory.page_dump(None);
        if self.ram_banking {
            for page in 0..self.memory.extra_pages() {
                ram.extend(self.memory.page_dump(Some(page)));
            }
        }
        ram
    }

    /// How many extra 64kb RAM pages are currently fitted.
    pub fn extra_ram_pages(&self) -> usize {
        self.memory.extra_pages()
    }

    /// Plugs a [`LargeRamExpansion`] (the standard Yarek/RAM7/CPC4MB-style
    /// >512K protocol, see `src/expansion.rs`) into the expansion port,
    /// > with `bank_count` 64kb banks fitted (up to 64 = 4MB) - the card's own
    /// > [`Page`]s, not `Memory`'s (see [`Expansion`]'s doc comment
    /// > for why). `Memory::additional_memory` (the CPC's own on-board extra
    /// > page) is left exactly as it was: real hardware wires a plugged-in
    /// > card to the same bus lines, so once one *is* plugged in,
    /// > `RamConfig::with_expansion_high_bits` always redirects every access
    /// > that would have reached it to the card instead - it becomes
    /// > unreachable, not explicitly resized away, matching the spec's own
    /// > note that a real card "replaces", not adds to, that internal page.
    pub fn plug_large_ram_expansion(&mut self, bank_count: usize) {
        self.ram_banking = true;
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
        self.large_ram_expansion
            .as_ref()
            .map(Expansion::ram_bank_count)
    }

    /// The currently plugged-in `LargeRamExpansion`'s last-latched high
    /// bits (see `RamBankExpansion::ram_bank_high_bits`), if any - lets
    /// external code (tests, diagnostics) observe whether anything has
    /// actually driven a port other than the standard &7Fxx.
    pub fn large_ram_expansion_high_bits(&self) -> Option<u8> {
        self.large_ram_expansion
            .as_ref()
            .map(|e| e.ram_bank_high_bits())
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
        self.large_ram_expansion
            .as_ref()
            .map(|e| e as &dyn Expansion)
    }

    /// Write the databuffer in memory according to the memory configuration
    pub fn write_buffer(&mut self, mut addr: u16, buffer: &[u8]) {
        let config = self.memory_config();
        let Motherboard {
            memory,
            large_ram_expansion,
            ..
        } = self;
        for byte in buffer.iter() {
            memory.write(
                addr,
                *byte,
                &config,
                large_ram_expansion
                    .as_mut()
                    .map(|e| e as &mut dyn Expansion),
            );
            addr = addr.wrapping_add(1);
        }
    }

    /// The CPC+'s register page is mapped at `addr` (RMR2 bits 4-3 at 3): it hides the RAM and
    /// ROMs there, for reads and writes.
    #[inline]
    fn in_register_page(&self, addr: u16) -> bool {
        (0x4000..0x8000).contains(&addr)
            && self.ga.plus().is_some_and(|plus| plus.rmr2 & 0x18 == 0x18)
    }

    /// Write one memory byte according to the memory configuration
    #[inline]
    pub fn write_byte(&mut self, addr: u16, byte: u8) {
        if self.in_register_page(addr) {
            if let (Some(asic), Some(video)) = (self.asic.as_deref_mut(), self.ga.plus_mut()) {
                asic.write(video, addr, byte);
                if addr == 0x6800 && byte != 0 {
                    self.ga.cancel_r52_interrupt();
                }
            }
            return;
        }
        let memory_cfg = self.memory_config();
        let Motherboard {
            memory,
            large_ram_expansion,
            ..
        } = self;
        memory.write(
            addr,
            byte,
            &memory_cfg,
            large_ram_expansion
                .as_mut()
                .map(|e| e as &mut dyn Expansion),
        );
    }

    pub fn write_word(&mut self, addr: u16, word: u16) {
        let memory_cfg = self.memory_config();
        let low = (word & 0xff) as u8;
        let high = ((word & 0xff00) >> 8) as u8;
        let Motherboard {
            memory,
            large_ram_expansion,
            ..
        } = self;
        memory.write(
            addr,
            low,
            &memory_cfg,
            large_ram_expansion
                .as_mut()
                .map(|e| e as &mut dyn Expansion),
        );
        memory.write(
            addr.wrapping_add(1),
            high,
            &memory_cfg,
            large_ram_expansion
                .as_mut()
                .map(|e| e as &mut dyn Expansion),
        );
    }

    /// Read one memory byte according to the memory configuration
    #[inline]
    pub fn read_byte(&self, addr: u16) -> u8 {
        if self.in_register_page(addr) {
            if let (Some(asic), Some(video)) = (self.asic.as_deref(), self.ga.plus()) {
                if let Some(byte) = asic.read(video, addr) {
                    return byte;
                }
            }
        }
        let memory_cfg = self.memory_config();
        self.memory
            .read(addr, &memory_cfg, self.expansion_ram_source())
    }

    /// Debug/diagnostic: one byte of a physical RAM bank, whatever the
    /// current CPU mapping, ROMs ignored.
    pub fn read_physical_ram(
        &self,
        page: crate::memory::PageNumber,
        bank: crate::memory::BankNumber,
        offset: u16,
    ) -> u8 {
        use crate::memory::{BankConfig, MemoryConfig, RamConfig};
        let config = MemoryConfig(
            MemoryConfig::default().0,
            RamConfig([BankConfig(page, bank); 4]),
        );
        self.memory
            .read(offset % 0x4000, &config, self.expansion_ram_source())
    }

    /// Record (`true`) or not (`false`, the default) the CPU's bus accesses
    /// for [`Self::take_bus_accesses`]. Watching does not change timing.
    pub fn watch_bus(&mut self, watch: bool) {
        self.bus_log = watch.then(Vec::new);
    }

    /// Record (`true`) or not (the default) the I/O writes alone, for [`Self::take_io_writes`]
    /// (a timing overlay): cheaper than [`Self::watch_bus`].
    pub fn watch_io_writes(&mut self, watch: bool) {
        self.io_writes = watch.then(Vec::new);
    }

    /// The I/O writes `(port, value)` since the last call (none when not watching).
    pub fn take_io_writes(&mut self) -> impl Iterator<Item = (u16, u8)> + '_ {
        self.io_writes
            .iter_mut()
            .flat_map(|writes| writes.drain(..))
    }

    /// The accesses recorded since the last call (empty when not watching).
    pub fn take_bus_accesses(&mut self) -> Vec<BusAccess> {
        self.bus_log
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    #[inline]
    fn log_bus(&mut self, kind: BusAccessKind, address: u16, value: u8) {
        if let Some(log) = &mut self.bus_log {
            log.push(BusAccess {
                kind,
                address,
                value,
            });
        }
    }

    /// Where the CPU address `addr` lands under the current mapping.
    pub fn physical_address(&self, addr: u16) -> PhysicalAddress {
        let config = self.memory_config();
        let offset = addr & 0x3fff;
        if config.is_in_enabled_lower_rom(addr) {
            PhysicalAddress::LowerRom { offset }
        } else if config.is_in_enabled_upper_rom(addr) {
            PhysicalAddress::UpperRom {
                rom: config.0.rom_number,
                offset,
            }
        } else {
            PhysicalAddress::Ram {
                bank: config.bank_at(addr).physical_index(),
                offset,
            }
        }
    }

    /// How many 16K RAM banks exist: the base 64K plus the extra pages, on
    /// board or on an expansion card.
    /// Where a write to `addr` goes: always RAM, whatever ROM is read there.
    pub fn physical_address_for_write(&self, addr: u16) -> PhysicalAddress {
        PhysicalAddress::Ram {
            bank: self.memory_config().bank_at(addr).physical_index(),
            offset: addr & 0x3fff,
        }
    }

    pub fn physical_bank_count(&self) -> u16 {
        let extra_pages = match self.large_ram_expansion_bank_count() {
            Some(count) => count,
            None => self.extra_ram_pages(),
        };
        4 + 4 * extra_pages as u16
    }

    fn physical_bank_config(&self, bank: u16) -> MemoryConfig {
        let bank_config = BankConfig::of_physical_index(bank, self.large_ram_expansion.is_some());
        MemoryConfig(MemoryConfig::default().0, RamConfig([bank_config; 4]))
    }

    /// One byte of a 16K RAM bank (see [`Self::physical_bank_count`]),
    /// whatever the CPU mapping. `None` for a bank that does not exist.
    pub fn read_bank_byte(&self, bank: u16, offset: u16) -> Option<u8> {
        (bank < self.physical_bank_count()).then(|| {
            self.memory.read(
                offset & 0x3fff,
                &self.physical_bank_config(bank),
                self.expansion_ram_source(),
            )
        })
    }

    /// Write one byte of a 16K RAM bank; `false` for a bank that does not
    /// exist.
    pub fn write_bank_byte(&mut self, bank: u16, offset: u16, byte: u8) -> bool {
        if bank >= self.physical_bank_count() {
            return false;
        }
        let config = self.physical_bank_config(bank);
        let Motherboard {
            memory,
            large_ram_expansion,
            ..
        } = self;
        memory.write(
            offset & 0x3fff,
            byte,
            &config,
            large_ram_expansion
                .as_mut()
                .map(|e| e as &mut dyn Expansion),
        );
        true
    }

    /// Write one byte of the base 64K RAM, whatever is mapped for the CPU
    /// (ROM overlays, extra RAM pages) - the counterpart of
    /// [`Self::read_video_ram_byte`].
    pub fn write_base_ram(&mut self, addr: u16, byte: u8) {
        let Motherboard {
            memory,
            large_ram_expansion,
            ..
        } = self;
        memory.write(
            addr,
            byte,
            &MemoryConfig::default(),
            large_ram_expansion
                .as_mut()
                .map(|e| e as &mut dyn Expansion),
        );
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
        self.memory
            .read(addr, &MemoryConfig::default(), self.expansion_ram_source())
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
                ..rom
            },
            ram,
        );
        self.memory.read(addr, &config, self.expansion_ram_source())
    }

    #[inline]
    pub fn read_word(&self, addr: u16) -> u16 {
        u16::from(self.read_byte(addr)) | (u16::from(self.read_byte(addr.wrapping_add(1))) << 8)
    }
}

impl Motherboard {
    /// Return the byte read from a chip
    /// TODO handle the case where there is a read conflict over several chips
    fn input_byte(&mut self, addr: u16) -> u8 {
        if let Some(plus) = self.ga.plus() {
            // the CPC+'s ASIC answers an IN from the Gate Array's port and the CRTC's select and
            // write ports (and takes it as an OUT, see `AsicVideo::bus_value`)
            if addr & 0xc000 == 0x4000 || addr & 0x4200 == 0 {
                return plus.bus_value;
            }
        }
        // The Gate Array is write-only: it has no data output driver, so an IN from its port
        // reads the floating bus (&FF), as Madram's CRTC detection (Amstrad Live, 05/2000)
        // expects of every CPC but the CPC+.
        if self.ppi().must_execute_tick() {
            self.ppi().get_data_from_pins()
        } else if self.crtc().must_execute_tick() {
            self.crtc().get_data_from_pins()
        } else if self.disc_interface && Self::is_fdc_status_addr(addr) {
            self.fdc.read_status()
        } else if self.disc_interface && Self::is_fdc_data_addr(addr) {
            self.fdc.read_data()
        } else if let Some(value) = self
            .expansion_port
            .as_ref()
            .and_then(|port| port.driven_data())
        {
            value
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

    /// A DD/FD prefix followed by another prefix only costs its M1: the last one applies (RE-WIRED's
    /// loader starts with `FD DD 21`).
    #[test]
    fn a_prefix_after_a_prefix_gives_way_to_it() {
        let mut motherboard = MotherboardBuilder::default();
        let mut motherboard = motherboard.disable_lower_rom().disable_upper_rom().build();
        // FD DD 21 34 12: LD IX,&1234; FD DD E5: PUSH IX; DD ED 44: NEG; DD FD 21 78 56: LD IY,&5678
        let program = [
            0xfd, 0xdd, 0x21, 0x34, 0x12, 0x3e, 0x05, 0xdd, 0xed, 0x44, 0xdd, 0xfd, 0x21, 0x78,
            0x56,
        ];
        for (offset, byte) in program.iter().enumerate() {
            motherboard.write_byte(0x4000 + offset as u16, *byte);
        }
        motherboard.cpu_mut().set_pc(0x4000);
        for _ in 0..4 {
            motherboard.execute_next_opcode_no_video();
        }
        assert_eq!(motherboard.ix(), 0x1234);
        assert_eq!(motherboard.a(), 0xfb, "NEG of 5");
        assert_eq!(motherboard.iy(), 0x5678);
        assert_eq!(motherboard.pc(), 0x400f);
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
        for _i in 0..32 {
            println!(
                "true,{},{}",
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
        assert_eq!(motherboard.pc(), 0x1001);
        assert_eq!(res.len(), 16);
        assert_eq!(motherboard.nops_counter().value(), 1);
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
        assert_eq!(motherboard.pc(), 0x1000);
        assert_eq!(res.len(), 16);
        assert_eq!(motherboard.nops_counter().value(), 1);
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
        let _res = motherboard.execute_next_opcode();
        dbg!(&motherboard.cpu.next_state);
        assert_eq!(motherboard.pc(), 0x38);
        assert_eq!(motherboard.nops_counter().value(), 1 + 5);
    }
}

#[cfg(test)]
mod model_test {
    use super::*;

    /// Without the picture ([`Motherboard::set_video`]) the machine runs exactly as with it:
    /// the same CPU state and memory after 2 s from the boot, only no pixels made.
    #[test]
    fn a_machine_without_picture_runs_as_with_it() {
        let run = |video: bool| {
            let mut mb = MotherboardBuilder::default().enable_interruptions().build();
            mb.set_video(video);
            while mb.t_state_counter().value() < 8_000_000 {
                mb.execute_next_opcode_no_video();
            }
            let memory: Vec<u8> = (0..=0xffffu16).map(|addr| mb.read_byte(addr)).collect();
            (
                mb.t_state_counter().value(),
                mb.pc(),
                mb.cpu().af(),
                mb.cpu().hl(),
                mb.cpu().sp(),
                memory,
            )
        };
        assert!(run(true) == run(false));
    }

    /// LD BC,&7FC4: OUT (C),C - then write 0x42 at &4000.
    fn bank_and_write(model: CpcModel, extra_pages: Option<usize>) -> Motherboard {
        let mut builder = MotherboardBuilder::default();
        builder
            .set_model(model)
            .disable_lower_rom()
            .disable_upper_rom();
        if let Some(pages) = extra_pages {
            builder.set_extra_ram_pages(pages);
        }
        let mut mb = builder.build();
        // The code runs from &8000.
        mb.write_buffer(
            0x8000,
            &[0x01, 0xc4, 0x7f, 0xed, 0x49, 0x3e, 0x42, 0x32, 0x00, 0x40],
        );
        mb.cpu_mut().reg16_mut(crate::z80::Reg16::Pc).set(0x8000);
        for _ in 0..4 {
            mb.execute_next_opcode_no_video();
        }
        mb
    }

    #[test]
    fn ram_banking_needs_the_6128_or_an_expansion() {
        let mb = bank_and_write(CpcModel::Cpc6128, None);
        assert_eq!(mb.read_bank_byte(4, 0), Some(0x42), "6128: &4000 is bank 4");
        let mb = bank_and_write(CpcModel::Cpc464, None);
        assert_eq!(
            mb.read_bank_byte(1, 0),
            Some(0x42),
            "stock 464: the MMR does nothing"
        );
        let mb = bank_and_write(CpcModel::Cpc464, Some(1));
        assert_eq!(
            mb.read_bank_byte(4, 0),
            Some(0x42),
            "464 with a 64K expansion"
        );
    }

    #[test]
    fn ram_banking_ignores_a14() {
        let mut builder = MotherboardBuilder::default();
        builder
            .set_model(CpcModel::Cpc6128)
            .disable_lower_rom()
            .disable_upper_rom();
        let mut mb = builder.build();
        // LD BC,&01C4: OUT (C),C - A15=0 selects the RAM configuration even with A14=0.
        mb.write_buffer(
            0x8000,
            &[0x01, 0xc4, 0x01, 0xed, 0x49, 0x3e, 0x42, 0x32, 0x00, 0x40],
        );
        mb.cpu_mut().reg16_mut(crate::z80::Reg16::Pc).set(0x8000);
        for _ in 0..4 {
            mb.execute_next_opcode_no_video();
        }
        assert_eq!(mb.read_bank_byte(4, 0), Some(0x42));
    }

    #[test]
    fn a_plus_maps_its_cartridge_and_its_register_page() {
        let mut mb = MotherboardBuilder::default()
            .set_model(CpcModel::Cpc6128Plus)
            .build();
        assert_eq!(mb.crtc().crtc_type(), CrtcType::Crtc3, "the ASIC's CRTC");
        assert!(mb.has_disc_interface() && mb.has_ram_banking());
        let lower_rom: Vec<u8> = (0..0x4000).map(|address| mb.read_byte(address)).collect();
        assert_ne!(
            &lower_rom[..],
            &crate::roms::OS6128[..],
            "the lower ROM is the cartridge's (Plus) firmware"
        );

        // RMR2 does not exist while the lock is closed
        mb.ga.write_register(0xb8);
        mb.write_byte(0x6402, 0x5a);
        assert_eq!(
            mb.read_bank_byte(1, 0x2402),
            Some(0x5a),
            "a plain RAM write, the page is not mapped"
        );
        for byte in [
            0xff, 0x00, 0xff, 0x77, 0xb3, 0x51, 0xa8, 0xd4, 0x62, 0x39, 0x9c, 0x46, 0x2b, 0x15,
            0x8a, 0xcd, 0xee,
        ] {
            if mb.asic.as_mut().unwrap().crtc_select_written(byte) {
                mb.ga.plus_mut().unwrap().unlocked = true;
            }
        }
        mb.ga.write_register(0xb8);
        mb.write_byte(0x6402, 0x21);
        mb.write_byte(0x6403, 0xf7);
        assert_eq!(
            (mb.read_byte(0x6402), mb.read_byte(0x6403)),
            (0x21, 0x07),
            "the palette, in the register page"
        );
        assert_eq!(
            mb.read_bank_byte(1, 0x2402),
            Some(0x5a),
            "the RAM below is not written"
        );
        assert_eq!(
            mb.read_byte(0x7820),
            mb.read_bank_byte(1, 0x3820).unwrap(),
            "where no register is: the RAM"
        );
        mb.ga.write_register(0xa0);
        assert_eq!(mb.read_byte(0x6402), 0x5a, "the page unmapped");

        // the lower ROM at &4000 (RMR2 &A9: page 1, BASIC), the upper ROM from the cartridge
        mb.ga.write_register(0xa9);
        let basic = mb.read_byte(0x4000);
        mb.force_rom_config(RomConfig {
            upper_rom: Flag::Enable,
            rom_number: 0x81,
            ..mb.memory_config().0
        });
        assert_eq!(
            mb.read_byte(0xc000),
            basic,
            "&DF81 selects the cartridge's page 1 too"
        );
        mb.force_rom_config(RomConfig {
            rom_number: 7,
            ..mb.memory_config().0
        });
        assert_ne!(
            mb.read_byte(0xc006),
            mb.read_byte(0x4006),
            "ROM 7 is AMSDOS (page 3), not BASIC"
        );

        let gx = MotherboardBuilder::default()
            .set_model(CpcModel::Gx4000)
            .build();
        assert!(!gx.has_disc_interface() && !gx.has_ram_banking());
        assert_eq!("6128+".parse::<CpcModel>(), Ok(CpcModel::Cpc6128Plus));
        assert_eq!("GX4000".parse::<CpcModel>(), Ok(CpcModel::Gx4000));
    }

    #[test]
    fn the_fdc_answers_with_a_disc_interface_only() {
        let msr = |mb: &mut Motherboard| mb.input_byte(0xfb7e);
        let mut mb = MotherboardBuilder::default()
            .set_model(CpcModel::Cpc464)
            .build();
        assert_eq!(msr(&mut mb), 0xff, "464: nothing on &FB7E");
        assert!(!mb.has_disc_interface());
        let mut mb = MotherboardBuilder::default()
            .set_model(CpcModel::Cpc464)
            .set_disc_interface(true)
            .build();
        assert_eq!(msr(&mut mb), 0x80, "464 with a DDI-1");
        let mut mb = MotherboardBuilder::default()
            .set_model(CpcModel::Cpc664)
            .build();
        assert_eq!(msr(&mut mb), 0x80);
        assert_eq!("cpc464".parse::<CpcModel>(), Ok(CpcModel::Cpc464));
        assert!("6129".parse::<CpcModel>().is_err());
    }
}
