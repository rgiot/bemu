//! What each command does ([`execute`]) and how it shows now ([`state`]).

use std::path::PathBuf;

use super::{
    Arg, ArgKind, ArgPrompt, Command, CommandError, CommandResult, CommandState, CommandTarget,
    HostRequest, Outcome, Recording, Speed, View,
};
use crate::crtc::CrtcType;
use crate::display::{crt::CrtProfile, DisplayMode};
use crate::fdc::{DriveKind, FdcDrive};
use crate::ga::GateArrayType;
use crate::input::{HostLayout, KeyboardMapping};
use crate::keyboard::KeyboardLayout;
use crate::motherboard::{CpcModel, Motherboard};
use crate::ppi::Brand;

/// The RAM sizes of Machine > Memory expansion.
pub const MACHINE_RAM_SIZES: [usize; 9] = [64, 128, 192, 256, 320, 384, 448, 512, 576];

const DISC_FILES: &[&str] = &["dsk", "hfe", "zip", "7z", "gz"];
const SNAPSHOT_FILES: &[&str] = &["sna"];
const TAPE_FILES: &[&str] = &["cdt", "tzx"];
const TEXT_FILES: &[&str] = &["txt", "bas", "asc"];

const BRANDS: [(&str, Brand); 8] = [
    ("amstrad", Brand::Amstrad),
    ("schneider", Brand::Schneider),
    ("awa", Brand::Awa),
    ("orion", Brand::Orion),
    ("saisho", Brand::Saisho),
    ("solavox", Brand::Solavox),
    ("triumph", Brand::Triumph),
    ("lsp", Brand::Lsp),
];

const PLATFORMS: [(&str, CpcModel); 6] = [
    ("cpc464", CpcModel::Cpc464),
    ("cpc664", CpcModel::Cpc664),
    ("cpc6128", CpcModel::Cpc6128),
    ("cpc464plus", CpcModel::Cpc464Plus),
    ("cpc6128plus", CpcModel::Cpc6128Plus),
    ("gx4000", CpcModel::Gx4000),
];

const RENDERINGS: [(&str, DisplayMode); 5] = [
    ("raw", DisplayMode::Raw),
    ("ctm", DisplayMode::Crt(CrtProfile::Ctm644)),
    ("gt65", DisplayMode::Crt(CrtProfile::Gt65)),
    ("trinitron", DisplayMode::Crt(CrtProfile::Trinitron)),
    ("atari", DisplayMode::Crt(CrtProfile::AtariColor)),
];

const SPEEDS: [(&str, Speed); 6] = [
    ("normal", Speed::Factor(1.0)),
    ("max", Speed::Max),
    ("25", Speed::Factor(0.25)),
    ("50", Speed::Factor(0.5)),
    ("200", Speed::Factor(2.0)),
    ("400", Speed::Factor(4.0)),
];

const PRINTER_CHARSETS: [&str; 9] = [
    "usa", "france", "germany", "uk", "denmark", "sweden", "italy", "spain", "japan",
];

const VIEWS: [(&str, View); 18] = [
    ("view-z80", View::Z80),
    ("view-stack", View::Stack),
    ("view-crtc", View::Crtc),
    ("view-ga", View::GateArray),
    ("view-psg", View::Psg),
    ("view-fdc", View::Fdc),
    ("view-memmap", View::MemoryMap),
    ("view-memory", View::Memory),
    ("view-disassembly", View::Disassembly),
    ("view-history", View::History),
    ("view-symbols", View::Symbols),
    ("view-beam", View::Beam),
    ("view-breakpoints", View::Breakpoints),
    ("view-counters", View::Counters),
    ("view-ppi", View::Ppi),
    ("view-tape", View::Tape),
    ("view-asic", View::Asic),
    ("palette-open", View::Palette),
];

#[cfg_attr(not(feature = "willy"), allow(dead_code))]
const WILLY_BASES: &[(&str, &str)] = &[
    ("&FEBC", "&FEBC"),
    ("&FEAC", "&FEAC"),
    ("&FEA4", "&FEA4"),
    ("&FE9C", "&FE9C"),
    ("&FE94", "&FE94"),
    ("&FE8C", "&FE8C"),
    ("&FE84", "&FE84"),
];

type Mb = Motherboard;

/// A disc image file of the host (none in a web page: the page gives bytes).
#[cfg(not(target_arch = "wasm32"))]
use crate::disc_image::{load_disc_file, save_disc_file};

#[cfg(target_arch = "wasm32")]
fn load_disc_file(spec: &str) -> Result<crate::disc_image::DiscImage, String> {
    Err(format!("{spec}: no files here"))
}

#[cfg(target_arch = "wasm32")]
fn save_disc_file(spec: &str, _: &crate::disc_image::DiscImage) -> Result<(), String> {
    Err(format!("{spec}: no files here"))
}

fn drive_of(id: &str) -> Option<(FdcDrive, usize, &str)> {
    if let Some(rest) = id.strip_prefix("drive-a-") {
        Some((FdcDrive::A, 0, rest))
    } else {
        id.strip_prefix("drive-b-")
            .map(|rest| (FdcDrive::B, 1, rest))
    }
}

fn done() -> CommandResult {
    Ok(Outcome::Done(None))
}

fn said(message: impl Into<String>) -> CommandResult {
    Ok(Outcome::Done(Some(message.into())))
}

fn ask(id: &'static str, title: &'static str, kind: ArgKind) -> CommandResult {
    Ok(Outcome::NeedsArg(ArgPrompt { id, title, kind }))
}

fn failed(error: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(error.to_string())
}

/// The argument, or the question to ask for it.
macro_rules! arg {
    ($command:expr, $title:expr, $kind:expr) => {
        match &$command.arg {
            Some(arg) => arg.clone(),
            None => return ask($command.id, $title, $kind),
        }
    };
}

fn settings(target: &mut dyn CommandTarget) -> Result<&mut super::HostSettings, CommandError> {
    target.settings_mut().ok_or(CommandError::Unsupported)
}

fn host(target: &mut dyn CommandTarget, request: HostRequest) -> CommandResult {
    target.request(request).map(|()| Outcome::Done(None))
}

fn paused(target: &dyn CommandTarget) -> bool {
    match target.debugger() {
        Some(debugger) => debugger.paused(),
        None => target.settings().is_some_and(|s| s.paused),
    }
}

fn set_paused(target: &mut dyn CommandTarget, paused: bool) -> CommandResult {
    if let Some(debugger) = target.debugger_mut() {
        debugger.set_paused(paused);
    } else {
        settings(target)?.paused = paused;
    }
    done()
}

/// The debugger and the machine, or why not.
fn debugger(
    target: &mut dyn CommandTarget,
) -> Result<(&mut crate::debug_server::DebugServer, &mut Mb), CommandError> {
    target
        .debugger_and_motherboard()
        .ok_or(CommandError::Unsupported)
}

/// An address, or a symbol of the debugger.
fn address(arg: &Arg, target: &dyn CommandTarget) -> Result<u16, CommandError> {
    if let Ok(n) = arg.number() {
        return u16::try_from(n).map_err(|_| CommandError::Invalid(format!("{n}: not an address")));
    }
    let text = arg.text();
    target
        .debugger()
        .and_then(|debugger| debugger.symbols().resolve(text.trim()))
        .ok_or_else(|| CommandError::Invalid(format!("{text}: no such address or symbol")))
}

/// Write `data` to the host's file `path`, or have the host save it (a page downloads it).
fn save(
    target: &mut dyn CommandTarget,
    arg: &Arg,
    default_name: &str,
    data: Vec<u8>,
) -> CommandResult {
    match arg {
        Arg::Path(_) | Arg::Text(_) if target.has_file_system() => {
            let path = arg.path()?;
            std::fs::write(&path, data).map_err(|e| failed(format!("{}: {e}", path.display())))?;
            said(format!("{} written", path.display()))
        }
        _ => host(
            target,
            HostRequest::SaveFile {
                name: match arg {
                    Arg::Text(name) if !name.is_empty() => name.clone(),
                    _ => default_name.to_owned(),
                },
                data,
            },
        ),
    }
}

fn load_disc(arg: &Arg) -> Result<(crate::disc_image::DiscImage, String), CommandError> {
    match arg {
        Arg::Bytes { name, data } => {
            let (file, entry) = crate::disc_image::split_disc_spec(name);
            crate::disc_image::load_disc_bytes(data, file, entry)
                .map(|image| (image, name.clone()))
                .map_err(failed)
        }
        other => {
            let spec = other.text();
            load_disc_file(&spec)
                .map(|image| (image, spec))
                .map_err(failed)
        }
    }
}

fn insert_disc(
    target: &mut dyn CommandTarget,
    drive: FdcDrive,
    index: usize,
    arg: &Arg,
) -> CommandResult {
    let (image, name) = load_disc(arg)?;
    let has_files = target.has_file_system() && !matches!(arg, Arg::Bytes { .. });
    let write_back = target.settings().is_some_and(|s| s.save_discs);
    let mb = target.motherboard_mut();
    mb.insert_disc(drive, Some(image));
    if write_back && has_files {
        mb.set_disc_file(drive, Some(name.clone()));
    }
    if let Some(settings) = target.settings_mut() {
        settings.media.disc_inserted(index, Some(name.clone()));
    }
    said(format!(
        "drive {}: {name}",
        if index == 0 { 'A' } else { 'B' }
    ))
}

fn disc_command(command: &Command, target: &mut dyn CommandTarget) -> Option<CommandResult> {
    let (drive, index, action) = drive_of(command.id)?;
    let letter = if index == 0 { 'A' } else { 'B' };
    Some((|| match action {
        "select-disc" | "insert-recent" => {
            let arg = arg!(
                command,
                "Insert a disc image",
                ArgKind::OpenFile(DISC_FILES)
            );
            insert_disc(target, drive, index, &arg)
        }
        "new-disc" => {
            let arg = arg!(
                command,
                "New blank disc",
                ArgKind::SaveFile(&["dsk"], "blank.dsk")
            );
            let mut bytes = Vec::new();
            cpclib_disc::builder::single_head_data_dsk().to_buffer(&mut bytes);
            let image = crate::disc_image::load_dsk(&bytes).map_err(failed)?;
            let file =
                (target.has_file_system() && !matches!(arg, Arg::Bytes { .. })).then(|| arg.text());
            if let Some(file) = &file {
                save_disc_file(file, &image).map_err(failed)?;
            }
            let mb = target.motherboard_mut();
            mb.insert_disc(drive, Some(image));
            mb.set_disc_file(drive, file.clone());
            if let Some(settings) = target.settings_mut() {
                settings.media.disc_inserted(index, file);
            }
            said(format!("drive {letter}: a blank data disc"))
        }
        "flip-disc" => match target.motherboard_mut().fdc_mut().flip_disc(drive) {
            Some(side) => said(format!(
                "drive {letter}: disc turned over, side {} up",
                if side == 0 { 'A' } else { 'B' }
            )),
            None if target.motherboard().fdc().has_disk(drive) => Err(CommandError::Unavailable(
                format!("drive {letter}: a 3.5\" drive reads both sides, nothing to turn over"),
            )),
            None => Err(CommandError::Unavailable(format!(
                "drive {letter}: no disc"
            ))),
        },
        "remove-disc" => {
            target.motherboard_mut().insert_disc(drive, None);
            target.motherboard_mut().set_disc_file(drive, None);
            if let Some(settings) = target.settings_mut() {
                settings.media.discs[index] = None;
            }
            said(format!("drive {letter}: no disc"))
        }
        "save-disc" => {
            let file = target
                .settings()
                .and_then(|s| s.media.discs[index].clone())
                .ok_or_else(|| CommandError::Unavailable("the disc has no file: Save as".into()))?;
            let image = target
                .motherboard()
                .fdc()
                .image(drive)
                .cloned()
                .ok_or_else(|| CommandError::Unavailable(format!("drive {letter}: no disc")))?;
            if target.has_file_system() {
                save_disc_file(&file, &image).map_err(failed)?;
            } else {
                target.request(HostRequest::SaveFile {
                    name: file.clone(),
                    data: image.to_bytes(),
                })?;
            }
            target.motherboard_mut().fdc_mut().mark_saved(drive);
            said(format!("{file} written"))
        }
        "save-disc-as" => {
            let arg = arg!(
                command,
                "Save the disc as",
                ArgKind::SaveFile(&["dsk", "hfe"], "disc.dsk")
            );
            let image = target
                .motherboard()
                .fdc()
                .image(drive)
                .cloned()
                .ok_or_else(|| CommandError::Unavailable(format!("drive {letter}: no disc")))?;
            if target.has_file_system() && !matches!(arg, Arg::Bytes { .. }) {
                let file = arg.text();
                save_disc_file(&file, &image).map_err(failed)?;
                if let Some(settings) = target.settings_mut() {
                    settings.media.disc_inserted(index, Some(file.clone()));
                }
                target.motherboard_mut().fdc_mut().mark_saved(drive);
                said(format!("{file} written"))
            } else {
                host(
                    target,
                    HostRequest::SaveFile {
                        name: "disc.dsk".into(),
                        data: image.to_bytes(),
                    },
                )
            }
        }
        "original" => {
            target
                .motherboard_mut()
                .fdc_mut()
                .set_drive_kind(drive, DriveKind::ThreeInch);
            done()
        }
        "pc" => {
            target
                .motherboard_mut()
                .fdc_mut()
                .set_drive_kind(drive, DriveKind::ThreeAndHalfInch);
            done()
        }
        "gotek" => {
            target
                .motherboard_mut()
                .fdc_mut()
                .set_drive_kind(drive, DriveKind::Gotek);
            done()
        }
        _ => Err(CommandError::Unknown(command.id.to_owned())),
    })())
}

fn disc_state(id: &str, target: &dyn CommandTarget) -> Option<CommandState> {
    let (drive, index, action) = drive_of(id)?;
    let fdc = target.motherboard().fdc();
    let has_disc = fdc.has_disk(drive);
    let files = target.has_file_system();
    Some(match action {
        "select-disc" => CommandState::enabled(true),
        "new-disc" => CommandState::enabled(true),
        // the files of a host (a page keeps no file it was given)
        "insert-recent" => CommandState::enabled(
            files
                && target
                    .settings()
                    .is_some_and(|s| !s.media.recent_discs[index].is_empty()),
        ),
        "flip-disc" => CommandState::enabled(has_disc && !fdc.drive_kind(drive).is_double_sided()),
        "remove-disc" | "save-disc-as" => CommandState::enabled(has_disc),
        "save-disc" => CommandState::enabled(
            has_disc
                && fdc.is_modified(drive)
                && target
                    .settings()
                    .is_some_and(|s| s.media.discs[index].is_some())
                && (files
                    || target.can(&HostRequest::SaveFile {
                        name: String::new(),
                        data: vec![],
                    })),
        ),
        "original" => CommandState::checked(fdc.drive_kind(drive) == DriveKind::ThreeInch),
        "pc" => CommandState::checked(fdc.drive_kind(drive) == DriveKind::ThreeAndHalfInch),
        "gotek" => CommandState::checked(fdc.drive_kind(drive) == DriveKind::Gotek),
        _ => CommandState::DISABLED, // gotek
    })
}

/// Plug again the card of type `T` with what `plug` makes (the card is unplugged first).
#[allow(dead_code)] // used by the cards built in
fn replug<T: crate::expansion_port::ExpansionDevice>(
    mb: &mut Mb,
    card: Result<T, String>,
) -> Result<(), CommandError> {
    let card = card.map_err(failed)?;
    mb.unplug_expansion::<T>();
    mb.plug_expansion(Box::new(card));
    Ok(())
}

fn devices(target: &mut dyn CommandTarget) -> Result<super::DeviceFiles, CommandError> {
    Ok(settings(target)?.devices.clone())
}

#[cfg(feature = "m4")]
fn m4_board(files: &super::DeviceFiles) -> Result<crate::m4::M4Board, String> {
    let sd = files
        .m4_sd
        .clone()
        .ok_or_else(|| "no SD card folder chosen".to_owned())?;
    let board = match &files.m4_rom {
        Some(rom) => crate::m4::M4Board::new(&crate::roms::load_rom(rom)?, sd),
        None => crate::m4::M4Board::with_bundled_rom(sd),
    }?
    .in_slot(files.m4_slot);
    #[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
    let board = board.with_wifi(files.m4_wifi);
    Ok(board)
}

#[cfg(feature = "booster")]
fn booster(
    files: &super::DeviceFiles,
    model: crate::booster::Model,
) -> Result<crate::booster::Booster, String> {
    let mut booster = crate::booster::Booster::new(model);
    if let Some(uart) = &files.booster_uart {
        booster = booster.with_link(match uart.split_once(':') {
            None if uart == "loopback" => Box::<crate::booster::Loopback>::default(),
            #[cfg(not(target_arch = "wasm32"))]
            Some(("listen", address)) => {
                Box::new(crate::booster::TcpLink::listen(address).map_err(|e| e.to_string())?)
            }
            #[cfg(not(target_arch = "wasm32"))]
            Some(("connect", address)) => {
                Box::new(crate::booster::TcpLink::connect(address).map_err(|e| e.to_string())?)
            }
            _ => {
                return Err(format!(
                    "{uart}: loopback, listen:ADDRESS or connect:ADDRESS"
                ))
            }
        });
    }
    if let Some(file) = &files.booster_eeprom {
        booster = booster.with_eeprom_file(file.clone())?;
    }
    if let Some(file) = &files.booster_flash {
        booster = booster.with_flash_file(file.clone())?;
    }
    if let Some(file) = &files.booster_adc {
        booster = booster.with_adc(crate::booster::AdcSource::open(file)?);
    }
    Ok(booster)
}

fn dmp2000(files: &super::DeviceFiles) -> crate::dmp2000::Dmp2000 {
    let mut printer = crate::dmp2000::Dmp2000::with_os_rom(crate::roms::OS6128);
    if let Some(prefix) = &files.dmp2000_prefix {
        printer.export_to(prefix);
    }
    let rom = files.dmp2000_rom.as_deref().unwrap_or(crate::roms::DMP2000);
    if let Err(e) = printer.load_font_rom(rom) {
        eprintln!("DMP2000's EPROM: {e}");
    }
    printer
}

fn dmp2000_mut(mb: &mut Mb) -> Result<&mut crate::dmp2000::Dmp2000, CommandError> {
    mb.printer_port_mut()
        .and_then(|port| port.device_as_mut::<crate::dmp2000::Dmp2000>())
        .ok_or_else(|| CommandError::Unavailable("no DMP2000 is plugged".into()))
}

fn has_dmp2000(mb: &Mb) -> bool {
    mb.printer_port()
        .is_some_and(|port| port.device_as::<crate::dmp2000::Dmp2000>().is_some())
}

/// The ROMs of a set, in their slots.
fn rom_set(id: &str, mb: &Mb) -> Vec<(u8, &'static str)> {
    match id {
        "rom-set-orgams" => vec![
            (10, "inner://orgams"),
            (11, "inner://orgext"),
            (12, "inner://monogams"),
            (13, "inner://bricbrac"),
            (14, "inner://orgass"),
        ],
        "rom-set-unidos" => {
            #[allow(unused_mut)]
            let mut set = vec![
                (7, "inner://unidos"),
                (14, "inner://unitools"),
                (15, "inner://amsdos"),
            ];
            #[cfg(feature = "nova")]
            if mb.has_expansion::<crate::nova::Nova>() {
                set.push((1, "inner://unidos-nova"));
            }
            #[cfg(feature = "albireo")]
            if mb.has_expansion::<crate::ch376::Albireo>() {
                set.push((27, "inner://unidos-albireo"));
            }
            #[cfg(feature = "m4")]
            if mb.has_expansion::<crate::m4::M4Board>() {
                set.push((1, "inner://unidos-m4"));
            }
            let _ = mb;
            set
        }
        "rom-set-rulezcharge" => vec![(5, "inner://rulezcharge")],
        _ => Vec::new(),
    }
}

fn rom_set_fitted(id: &str, mb: &Mb) -> bool {
    let set = rom_set(id, mb);
    !set.is_empty()
        && set.iter().all(
            |(slot, name)| match (mb.upper_rom(*slot), crate::roms::load_rom(name)) {
                (Some(fitted), Ok(rom)) => fitted == &rom[..rom.len().min(0x4000)],
                _ => false,
            },
        )
}

fn total_ram_kb(mb: &Mb) -> usize {
    mb.total_ram_kb()
}

fn model_ram_kb(model: CpcModel) -> usize {
    if model.has_ram_banking() {
        128
    } else {
        64
    }
}

/// The commands kept by the registry but with nothing behind them yet.
const NOT_IMPLEMENTED: &[&str] = &[];

/// Whether `id` has a behaviour (every command of the registry must).
#[cfg(test)]
pub(super) fn is_known(id: &str) -> bool {
    let mut probe = Probe::default();
    let Ok(command) = Command::new(id) else {
        return false;
    };
    !matches!(execute(command, &mut probe), Err(CommandError::Unknown(_)))
}

/// A target for [`is_known`]: a machine, no host.
#[cfg(test)]
#[derive(Default)]
struct Probe(Mb);

#[cfg(test)]
impl CommandTarget for Probe {
    fn motherboard(&self) -> &Mb {
        &self.0
    }
    fn motherboard_mut(&mut self) -> &mut Mb {
        &mut self.0
    }
    fn has_file_system(&self) -> bool {
        false
    }
}

/// Run `command` on `target`.
pub(super) fn execute(command: Command, target: &mut dyn CommandTarget) -> CommandResult {
    let id = command.id;
    if NOT_IMPLEMENTED.contains(&id) {
        return Err(CommandError::NotImplemented);
    }
    if let Some(result) = disc_command(&command, target) {
        return result;
    }
    if let Some(view) = VIEWS.iter().find(|(name, _)| *name == id).map(|(_, v)| *v) {
        return host(target, HostRequest::Open(view));
    }
    if let Some(rest) = id.strip_prefix("machine-crtc-") {
        let n: u8 = rest.parse().map_err(|_| CommandError::Unknown(id.into()))?;
        let crtc = CrtcType::try_from(n).map_err(|_| CommandError::Unknown(id.into()))?;
        target
            .motherboard_mut()
            .change_crtc(crtc)
            .map_err(CommandError::Unavailable)?;
        return said(format!("CRTC {n}"));
    }
    if let Some(rest) = id.strip_prefix("machine-platform-") {
        let (_, model) = PLATFORMS
            .iter()
            .find(|(name, _)| *name == rest)
            .ok_or_else(|| CommandError::Unknown(id.into()))?;
        target.motherboard_mut().change_model(*model);
        return said(model.name());
    }
    if let Some(rest) = id.strip_prefix("machine-brand-") {
        let rest = if rest == "lsp" { "lsp" } else { rest };
        let (_, brand) = BRANDS
            .iter()
            .find(|(name, _)| *name == rest)
            .ok_or_else(|| CommandError::Unknown(id.into()))?;
        target.motherboard_mut().ppi_mut().set_brand(*brand);
        return said("brand changed: the firmware shows it at its next reset");
    }
    if let Some(rest) = id.strip_prefix("machine-ram-") {
        let kb = match rest {
            "default" => None,
            kb => Some(
                kb.parse::<usize>()
                    .map_err(|_| CommandError::Unknown(id.into()))?,
            ),
        };
        target
            .motherboard_mut()
            .set_total_ram_kb(kb)
            .map_err(CommandError::Invalid)?;
        return said(format!("{}K of RAM", total_ram_kb(target.motherboard())));
    }
    if let Some(rest) = id.strip_prefix("speed-") {
        let (_, speed) = SPEEDS
            .iter()
            .find(|(name, _)| *name == rest)
            .ok_or_else(|| CommandError::Unknown(id.into()))?;
        settings(target)?.speed = *speed;
        return done();
    }
    if let Some(rest) = id.strip_prefix("video-rendering-") {
        let settings = settings(target)?;
        settings.display = match rest {
            "next" => settings.display.next(),
            name => RENDERINGS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, mode)| *mode)
                .ok_or_else(|| CommandError::Unknown(id.into()))?,
        };
        return done();
    }
    if let Some(rest) = id.strip_prefix("printer-charset-") {
        let set = PRINTER_CHARSETS
            .iter()
            .position(|name| *name == rest)
            .ok_or_else(|| CommandError::Unknown(id.into()))?;
        dmp2000_mut(target.motherboard_mut())?.set_international_set(set as u8);
        return done();
    }
    if let Some(rest) = id.strip_prefix("keyboard-host-layout-") {
        settings(target)?.host_layout = match rest {
            "auto" => None,
            "qwerty" => Some(HostLayout::Qwerty),
            "azerty" => Some(HostLayout::Azerty),
            "qwertz" => Some(HostLayout::Qwertz),
            _ => return Err(CommandError::Unknown(id.into())),
        };
        return done();
    }
    if id.starts_with("rom-set-") {
        let mb = target.motherboard_mut();
        let fitted = rom_set_fitted(id, mb);
        for (slot, name) in rom_set(id, mb) {
            let rom = if fitted {
                None
            } else {
                Some(crate::roms::load_rom(name).map_err(failed)?)
            };
            mb.set_upper_rom(slot, rom.as_deref()).map_err(failed)?;
        }
        // slot 7 was UniDOS: AMSDOS again (when the machine has a disc interface)
        if id == "rom-set-unidos" && fitted {
            let fitted = mb.has_disc_interface();
            mb.set_disc_interface(fitted);
        }
        if id == "rom-set-orgams" && !fitted && total_ram_kb(mb) < 576 {
            mb.set_total_ram_kb(Some(576)).map_err(failed)?;
        }
        return said(if fitted {
            "ROMs removed"
        } else {
            "ROMs fitted: reset to use them"
        });
    }

    match id {
        // ---------------------------------------------------------------- File
        "drives-swap" => {
            let mb = target.motherboard_mut();
            let a = mb.fdc().image(FdcDrive::A).cloned();
            let b = mb.fdc().image(FdcDrive::B).cloned();
            mb.fdc_mut().insert_image(FdcDrive::A, b);
            mb.fdc_mut().insert_image(FdcDrive::B, a);
            if let Some(settings) = target.settings_mut() {
                settings.media.discs.swap(0, 1);
                let discs = settings.media.discs.clone();
                let write_back = settings.save_discs;
                let mb = target.motherboard_mut();
                if write_back {
                    mb.set_disc_file(FdcDrive::A, discs[0].clone());
                    mb.set_disc_file(FdcDrive::B, discs[1].clone());
                }
            }
            said("discs A and B swapped")
        }
        "discs-save-automatically-toggle" => {
            let settings = settings(target)?;
            settings.save_discs = !settings.save_discs;
            let (on, discs) = (settings.save_discs, settings.media.discs.clone());
            let mb = target.motherboard_mut();
            mb.set_disc_file(FdcDrive::A, discs[0].clone().filter(|_| on));
            mb.set_disc_file(FdcDrive::B, discs[1].clone().filter(|_| on));
            done()
        }
        "tape-insert" => {
            let arg = arg!(command, "Insert a tape", ArgKind::OpenFile(TAPE_FILES));
            let (name, bytes) = arg.bytes()?;
            let tape = crate::tape::Tape::from_cdt(&bytes).map_err(failed)?;
            target.motherboard_mut().insert_tape(Some(tape));
            if let Some(settings) = target.settings_mut() {
                settings.media.tape = Some(name.clone());
            }
            said(format!("tape: {name}"))
        }
        "tape-eject" => {
            target.motherboard_mut().insert_tape(None);
            if let Some(settings) = target.settings_mut() {
                settings.media.tape = None;
            }
            done()
        }
        "tape-record" => {
            target.motherboard_mut().start_tape_recording();
            said("recording the cassette port: SAVE on the CPC, then stop the recording")
        }
        "tape-record-stop" => {
            if target.motherboard().tape_recorder().is_none() {
                return Err(CommandError::Unavailable("not recording".into()));
            }
            let arg = arg!(
                command,
                "Save the recorded tape",
                ArgKind::SaveFile(TAPE_FILES, "bemu.cdt")
            );
            let seconds = target
                .motherboard()
                .tape_recorder()
                .map_or(0.0, |recorder| recorder.seconds());
            let cdt = target
                .motherboard_mut()
                .stop_tape_recording()
                .unwrap_or_default();
            save(target, &arg, "bemu.cdt", cdt)?;
            said(format!("{seconds:.1} s of tape recorded"))
        }
        "tape-play" | "tape-stop" | "tape-rewind" => {
            let deck = target
                .motherboard_mut()
                .tape_deck_mut()
                .ok_or_else(|| CommandError::Unavailable("no tape inserted".into()))?;
            match id {
                "tape-play" => deck.set_playing(true),
                "tape-stop" => deck.set_playing(false),
                _ => deck.rewind(),
            }
            done()
        }
        "albireo-select-path" | "expansion-albireo-toggle" => {
            #[cfg(feature = "albireo")]
            {
                use crate::ch376::Albireo;
                if id == "expansion-albireo-toggle"
                    && target.motherboard().has_expansion::<Albireo>()
                {
                    target.motherboard_mut().unplug_expansion::<Albireo>();
                    return said("the Albireo is unplugged");
                }
                #[cfg(feature = "m4")]
                if target.motherboard().has_expansion::<crate::m4::M4Board>() {
                    return Err(CommandError::Unavailable(
                        "the M4 decodes &FExx too: unplug it first".into(),
                    ));
                }
                if !target.has_file_system() {
                    // a page: a FAT image uploaded, kept in memory
                    let arg = arg!(
                        command,
                        "The Albireo's USB key: a FAT image",
                        ArgKind::OpenFile(&["img", "ima", "bin", "fat"])
                    );
                    let (name, data) = arg.bytes()?;
                    replug(target.motherboard_mut(), Ok(Albireo::with_image(data)))?;
                    return said(format!("{name}: the Albireo's USB key"));
                }
                let arg = arg!(
                    command,
                    "The Albireo's USB key: a folder or a FAT image",
                    ArgKind::Folder
                );
                let path = arg.path()?;
                replug(target.motherboard_mut(), Albireo::open(&path))?;
                settings(target)?.devices.albireo = Some(path.clone());
                said(format!("{}: the Albireo's USB key", path.display()))
            }
            #[cfg(not(feature = "albireo"))]
            Err(CommandError::Unsupported)
        }
        "albireo-eject" => {
            #[cfg(feature = "albireo")]
            {
                let albireo = target
                    .motherboard_mut()
                    .expansion_port_mut()
                    .and_then(|port| port.device_mut::<crate::ch376::Albireo>())
                    .ok_or_else(|| CommandError::Unavailable("no Albireo".into()))?;
                albireo.ch376.plug(None);
                done()
            }
            #[cfg(not(feature = "albireo"))]
            Err(CommandError::Unsupported)
        }
        "m4-select-folder"
        | "expansion-m4-toggle"
        | "m4-select-rom"
        | "m4-set-slot"
        | "m4-wifi-toggle"
        | "m4-eject" => {
            #[cfg(feature = "m4")]
            {
                use crate::m4::M4Board;
                let plugged = target.motherboard().has_expansion::<M4Board>();
                if (id == "expansion-m4-toggle" && plugged) || id == "m4-eject" {
                    target.motherboard_mut().unplug_expansion::<M4Board>();
                    return said("the M4 is unplugged");
                }
                let mut files = devices(target)?;
                match id {
                    "m4-select-folder" | "expansion-m4-toggle" => {
                        let arg = arg!(command, "The M4's SD card folder", ArgKind::Folder);
                        files.m4_sd = Some(arg.path()?);
                    }
                    "m4-select-rom" => {
                        let arg = arg!(command, "The M4's ROM", ArgKind::OpenFile(&["rom", "bin"]));
                        files.m4_rom = Some(arg.text());
                    }
                    "m4-set-slot" => {
                        let arg = arg!(
                            command,
                            "The M4 ROM's slot",
                            ArgKind::Number {
                                min: 0,
                                max: 255,
                                default: 6
                            }
                        );
                        files.m4_slot = u8::try_from(arg.number()?)
                            .map_err(|_| CommandError::Invalid("a slot from 0 to 255".into()))?;
                    }
                    _ => files.m4_wifi = !files.m4_wifi,
                }
                #[cfg(feature = "albireo")]
                if target
                    .motherboard()
                    .has_expansion::<crate::ch376::Albireo>()
                {
                    return Err(CommandError::Unavailable(
                        "the Albireo decodes &FExx too: unplug it first".into(),
                    ));
                }
                if files.m4_sd.is_some()
                    && (plugged || id == "m4-select-folder" || id == "expansion-m4-toggle")
                {
                    replug(target.motherboard_mut(), m4_board(&files))?;
                }
                settings(target)?.devices = files;
                done()
            }
            #[cfg(not(feature = "m4"))]
            Err(CommandError::Unsupported)
        }
        "cartridge-insert" => {
            if !target.motherboard().model().is_plus() {
                return Err(CommandError::Unavailable(
                    "a cartridge goes in a CPC+".into(),
                ));
            }
            let arg = arg!(
                command,
                "Insert a cartridge",
                ArgKind::OpenFile(&["cpr", "zip"])
            );
            let cartridge = match &arg {
                Arg::Bytes { name, data } => crate::asic::Cartridge::from_bytes(data, name, None),
                other => {
                    let (name, data) = other.bytes()?;
                    crate::asic::Cartridge::from_bytes(&data, &name, None)
                }
            }
            .map_err(failed)?;
            let mb = target.motherboard_mut();
            mb.insert_cartridge(Some(cartridge));
            mb.restart();
            done()
        }
        "cartridge-remove" => {
            let mb = target.motherboard_mut();
            mb.insert_cartridge(None);
            mb.restart();
            done()
        }
        "dandanator-select-romset"
        | "dandanator-eject"
        | "expansion-dandanator-toggle"
        | "dandanator-writable-toggle" => {
            #[cfg(feature = "dandanator")]
            {
                use crate::dandanator::Dandanator;
                let plugged = target.motherboard().has_expansion::<Dandanator>();
                if id == "dandanator-eject" || (id == "expansion-dandanator-toggle" && plugged) {
                    let mb = target.motherboard_mut();
                    mb.unplug_expansion::<Dandanator>();
                    mb.restart();
                    return said("the Dandanator is unplugged");
                }
                let mut files = devices(target)?;
                if id == "dandanator-writable-toggle" {
                    files.dandanator_writable = !files.dandanator_writable;
                } else {
                    let arg = arg!(
                        command,
                        "The Dandanator's romset",
                        ArgKind::OpenFile(&["rom", "bin"])
                    );
                    let (name, data) = arg.bytes()?;
                    files.dandanator = Some((name, data.into()));
                }
                let Some((name, data)) = files.dandanator.clone() else {
                    settings(target)?.devices = files;
                    return done();
                };
                let card = if files.dandanator_writable && target.has_file_system() {
                    Dandanator::open(PathBuf::from(&name), true)
                } else {
                    Ok(Dandanator::new(&data))
                };
                replug(target.motherboard_mut(), card)?;
                target.motherboard_mut().restart();
                settings(target)?.devices = files;
                said(format!("the Dandanator: {name}"))
            }
            #[cfg(not(feature = "dandanator"))]
            Err(CommandError::Unsupported)
        }
        "snapshot-load" => {
            let arg = arg!(
                command,
                "Load a snapshot",
                ArgKind::OpenFile(SNAPSHOT_FILES)
            );
            let (name, bytes) = arg.bytes()?;
            let breakpoints = crate::snapshot::load_sna_bytes(target.motherboard_mut(), &bytes)
                .map_err(failed)?;
            if let Some((debugger, mb)) = target.debugger_and_motherboard() {
                debugger.add_symbols(&breakpoints.symbols);
                for breakpoint in &breakpoints.exec {
                    debugger.add_exec_breakpoint(breakpoint.address, mb);
                }
                for breakpoint in breakpoints.advanced {
                    debugger.add_advanced_breakpoint(breakpoint, mb);
                }
            }
            if let (Some(settings), Arg::Path(path)) = (target.settings_mut(), &arg) {
                settings.media.snapshot = Some(path.clone());
            }
            said(format!("snapshot {name} loaded"))
        }
        "snapshot-save" | "snapshot-update" | "snapshot-quick-save" => {
            let version = target.settings().map_or(3, |s| s.snapshot_version);
            let bytes = crate::snapshot::save_sna_bytes_version(target.motherboard(), version)
                .map_err(failed)?;
            let arg = match id {
                "snapshot-save" => arg!(
                    command,
                    "Save a snapshot",
                    ArgKind::SaveFile(SNAPSHOT_FILES, "bemu.sna")
                ),
                "snapshot-update" => Arg::Path(
                    target
                        .settings()
                        .and_then(|s| s.media.snapshot.clone())
                        .ok_or_else(|| {
                            CommandError::Unavailable("no snapshot loaded or saved yet".into())
                        })?,
                ),
                _ => {
                    let settings = settings(target)?;
                    let mut n = settings.media.quick_snapshots;
                    let name = loop {
                        let name = format!("bemu_{n:04}.sna");
                        n += 1;
                        if !std::path::Path::new(&name).exists() {
                            break name;
                        }
                    };
                    settings.media.quick_snapshots = n;
                    if target.has_file_system() {
                        Arg::Path(name.into())
                    } else {
                        Arg::Text(name)
                    }
                }
            };
            if let (Some(settings), Arg::Path(path)) = (target.settings_mut(), &arg) {
                if id != "snapshot-quick-save" {
                    settings.media.snapshot = Some(path.clone());
                }
            }
            save(target, &arg, "bemu.sna", bytes)
        }
        "snapshot-save-on-exit-toggle" => {
            if settings(target)?.save_snapshot_on_exit.take().is_some() {
                return done();
            }
            let arg = arg!(
                command,
                "Snapshot to save at the end",
                ArgKind::SaveFile(SNAPSHOT_FILES, "bemu.sna")
            );
            settings(target)?.save_snapshot_on_exit = Some(arg.path()?);
            done()
        }
        "config-save" => {
            let arg = arg!(
                command,
                "Save the configuration",
                ArgKind::SaveFile(&["json"], "bemu.json")
            );
            let config = super::config::save(target);
            save(target, &arg, "bemu.json", config.into_bytes())
        }
        "config-load" => {
            let arg = arg!(
                command,
                "Load a configuration",
                ArgKind::OpenFile(&["json"])
            );
            let (_, bytes) = arg.bytes()?;
            let notes = super::config::load(target, &String::from_utf8_lossy(&bytes))?;
            if notes.is_empty() {
                said("configuration loaded")
            } else {
                said(format!("configuration loaded, but: {}", notes.join("; ")))
            }
        }
        "config-reset" => {
            super::config::load(target, "{}")?;
            said("default configuration")
        }
        "csl-load" => {
            let arg = arg!(command, "Play a CSL script", ArgKind::OpenFile(&["csl"]));
            let (_, bytes) = arg.bytes()?;
            let folder = match &arg {
                Arg::Path(path) => path.parent().map(PathBuf::from),
                _ => None,
            };
            host(
                target,
                HostRequest::RunCsl {
                    text: String::from_utf8_lossy(&bytes).into_owned(),
                    folder,
                },
            )
        }
        "csl-stop" => host(target, HostRequest::StopCsl),
        "files-watch-toggle" => {
            let settings = settings(target)?;
            settings.watch_files = !settings.watch_files;
            done()
        }
        "exit" => match target.request(HostRequest::Quit) {
            Err(CommandError::Unsupported) => {
                debugger(target)?.0.request_quit();
                done()
            }
            other => other.map(|()| Outcome::Done(None)),
        },
        // --------------------------------------------------------------- Debug
        "debug-run" => set_paused(target, false),
        "debug-pause" => set_paused(target, true),
        "debug-reset" | "machine-reset-soft" | "machine-reset-hard" => {
            let hard = id == "machine-reset-hard";
            if hard {
                target.power_cycle();
            } else {
                target.motherboard_mut().restart();
            }
            if let Some(debugger) = target.debugger_mut() {
                debugger.reset_done(hard);
            }
            done()
        }
        "debug-step-in" => {
            debugger(target)?.0.step();
            done()
        }
        "debug-step-over" => {
            let (debugger, mb) = debugger(target)?;
            debugger.step_over(mb);
            done()
        }
        "debug-step-out" => {
            let (debugger, mb) = debugger(target)?;
            debugger.step_out(mb);
            done()
        }
        "debug-step-frame" => {
            debugger(target)?.0.step_frame();
            done()
        }
        "debug-run-to-address" => {
            let arg = arg!(command, "Run to the address or symbol", ArgKind::Address);
            let address = address(&arg, target)?;
            debugger(target)?.0.run_to(address);
            done()
        }
        "debug-run-to-beam" | "breakpoint-add-raster" => {
            let arg = arg!(command, "Beam position: LINE,NOP (NOP 0-63)", ArgKind::Text);
            let text = arg.text();
            let (line, nop) = text.split_once(',').unwrap_or((text.as_str(), "0"));
            let (debugger, mb) = debugger(target)?;
            if id == "debug-run-to-beam" {
                let number = |text: &str| {
                    Arg::Text(text.trim().to_owned())
                        .number()
                        .ok()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| CommandError::Invalid(format!("{text}: not a number")))
                };
                debugger.run_to_beam(number(line)?, number(nop)?);
            } else {
                debugger
                    .api(
                        "POST",
                        "/api/raster_bp",
                        &[("add", "1"), ("line", line.trim()), ("nop", nop.trim())],
                        "",
                        mb,
                    )
                    .map_err(CommandError::Invalid)?;
            }
            done()
        }
        "debug-set-pc" => {
            let arg = arg!(command, "Set the PC to", ArgKind::Address);
            let address = address(&arg, target)?;
            target.motherboard_mut().set_pc(address);
            done()
        }
        "breakpoint-add" => {
            let arg = arg!(
                command,
                "Execution breakpoint: address or symbol",
                ArgKind::Address
            );
            let address = address(&arg, target)?;
            let (debugger, mb) = debugger(target)?;
            debugger.add_exec_breakpoint(address, mb);
            said(format!("breakpoint at &{address:04X}"))
        }
        "breakpoint-toggle" => {
            let arg = arg!(
                command,
                "Toggle the execution breakpoint: address or symbol",
                ArgKind::Address
            );
            let address = address(&arg, target)?;
            let (debugger, mb) = debugger(target)?;
            if debugger.remove_exec_breakpoint(address, mb) {
                said(format!("breakpoint at &{address:04X} removed"))
            } else {
                debugger.add_exec_breakpoint(address, mb);
                said(format!("breakpoint at &{address:04X}"))
            }
        }
        "breakpoint-remove" => {
            let arg = arg!(
                command,
                "Remove a breakpoint: address or symbol, or #N for memory / I/O breakpoint N",
                ArgKind::Text
            );
            let text = arg.text();
            if let Some(index) = text.trim().strip_prefix('#') {
                let (debugger, mb) = debugger(target)?;
                debugger
                    .api(
                        "DELETE",
                        "/api/breakpoints",
                        &[("index", index.trim())],
                        "",
                        mb,
                    )
                    .map_err(CommandError::Invalid)?;
                return said(format!("breakpoint #{} removed", index.trim()));
            }
            let address = address(&arg, target)?;
            let (debugger, mb) = debugger(target)?;
            if debugger.remove_exec_breakpoint(address, mb) {
                said(format!("breakpoint at &{address:04X} removed"))
            } else {
                Err(CommandError::Unavailable(format!(
                    "no breakpoint at &{address:04X}"
                )))
            }
        }
        "breakpoint-add-advanced" => {
            let arg = arg!(
                command,
                "Memory / I/O breakpoint: JSON {kind, access, addr, mask, value, run} or a REMU acebreak",
                ArgKind::Text
            );
            let text = arg.text();
            let body = if text.trim_start().starts_with('{') {
                text
            } else {
                serde_json::json!({ "remu": text }).to_string()
            };
            let (debugger, mb) = debugger(target)?;
            debugger
                .api("POST", "/api/breakpoints", &[], &body, mb)
                .map_err(CommandError::Invalid)?;
            done()
        }
        "breakpoint-add-basic" => {
            let arg = arg!(
                command,
                "Stop at the BASIC line",
                ArgKind::Number {
                    min: 0,
                    max: 65535,
                    default: 10
                }
            );
            let line = arg.number()?;
            let (debugger, mb) = debugger(target)?;
            let lines = debugger
                .api("GET", "/api/basic_bp", &[], "", mb)
                .map_err(failed)?;
            let mut list: Vec<String> = lines["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| v.to_string())
                .collect();
            list.push(line.to_string());
            debugger
                .api("POST", "/api/basic_bp", &[], &list.join(","), mb)
                .map_err(CommandError::Invalid)?;
            done()
        }
        "breakpoints-clear" => {
            let (debugger, mb) = debugger(target)?;
            debugger.clear_breakpoints(mb);
            done()
        }
        "counter-new" | "counter-toggle" | "counter-reset" | "counter-delete" => {
            let arg = arg!(
                command,
                "Counter name (empty: all, to delete)",
                ArgKind::Text
            );
            let name = arg.text();
            let (debugger, mb) = debugger(target)?;
            let running = debugger
                .counters()
                .iter()
                .find(|(n, _, _)| *n == name)
                .map(|(_, _, running)| *running);
            let result = match id {
                "counter-delete" if name.trim().is_empty() || name == "all" => {
                    debugger.api("DELETE", "/api/counters", &[], "", mb)
                }
                "counter-delete" => {
                    debugger.api("DELETE", "/api/counters", &[("name", &name)], "", mb)
                }
                _ => {
                    let action = match id {
                        "counter-new" => "create",
                        "counter-reset" => "reset",
                        _ if running == Some(true) => "stop",
                        _ => "start",
                    };
                    debugger.api(
                        "POST",
                        "/api/counters",
                        &[],
                        &serde_json::json!({ "name": name, "action": action }).to_string(),
                        mb,
                    )
                }
            };
            result.map_err(CommandError::Invalid)?;
            done()
        }
        "symbols-load" => {
            let arg = arg!(
                command,
                "Load symbols",
                ArgKind::OpenFile(&["sym", "lst", "txt", "asm"])
            );
            let (name, bytes) = arg.bytes()?;
            let (symbols, errors) =
                crate::symbols::SymbolTable::parse_file(&String::from_utf8_lossy(&bytes));
            let count = symbols.len();
            debugger(target)?.0.add_symbols(&symbols);
            said(format!(
                "{name}: {count} symbols{}",
                if errors.is_empty() {
                    String::new()
                } else {
                    format!(", {} lines not understood", errors.len())
                }
            ))
        }
        "memory-poke" => {
            let arg = arg!(
                command,
                "Poke: ADDRESS then hexadecimal bytes (&4000 3E 01 C9)",
                ArgKind::Text
            );
            let text = arg.text();
            let mut words = text.split_whitespace();
            let start = address(
                &Arg::Text(words.next().unwrap_or_default().to_owned()),
                target,
            )?;
            let bytes = words
                .map(|word| {
                    // bytes in hexadecimal, as a monitor takes them
                    let digits = word
                        .trim_start_matches(['&', '#', '$'])
                        .trim_start_matches("0x");
                    u8::from_str_radix(digits, 16)
                        .ok()
                        .ok_or_else(|| CommandError::Invalid(format!("{word}: not a byte")))
                })
                .collect::<Result<Vec<u8>, _>>()?;
            if bytes.is_empty() {
                return Err(CommandError::Invalid("no byte to write".into()));
            }
            let mb = target.motherboard_mut();
            for (i, byte) in bytes.iter().enumerate() {
                mb.write_byte(start.wrapping_add(i as u16), *byte);
            }
            said(format!("{} bytes at &{start:04X}", bytes.len()))
        }
        "z80-set-register" => {
            let arg = arg!(
                command,
                "Register = value (HL=&C000, A=3, PC=label)",
                ArgKind::Text
            );
            let text = arg.text();
            let (name, value) = text
                .split_once('=')
                .ok_or_else(|| CommandError::Invalid("REGISTER=VALUE".into()))?;
            let name = name.trim().to_uppercase();
            let value = address(&Arg::Text(value.trim().to_owned()), target)?;
            let mb = target.motherboard_mut();
            match name.as_str() {
                "PC" => mb.set_pc(value),
                "SP" => mb.set_sp(value),
                "AF" => mb.set_af(value),
                "BC" => mb.set_bc(value),
                "DE" => mb.set_de(value),
                "HL" => mb.set_hl(value),
                "IX" => mb.set_ix(value),
                "IY" => mb.set_iy(value),
                _ => {
                    let (debugger, mb) = debugger(target)?;
                    debugger
                        .api(
                            "POST",
                            "/api/z80",
                            &[],
                            &format!("{{\"{name}\":{value}}}"),
                            mb,
                        )
                        .map_err(CommandError::Invalid)?;
                }
            }
            done()
        }
        "history-set-size" => {
            let arg = arg!(
                command,
                "Instructions kept",
                ArgKind::Number {
                    min: 1,
                    max: 100_000,
                    default: 20
                }
            );
            let size = arg.number()?;
            if !(1..=100_000).contains(&size) {
                return Err(CommandError::Invalid("1 to 100000".into()));
            }
            debugger(target)?.0.set_history_size(size as usize);
            done()
        }
        "basic-export" => {
            let arg = arg!(
                command,
                "Export the BASIC listing",
                ArgKind::SaveFile(TEXT_FILES, "listing.bas")
            );
            let listing = crate::basic_program::program(target.motherboard())
                .map(|program| program.to_string())
                .map_err(CommandError::Unavailable)?;
            save(target, &arg, "listing.bas", listing.into_bytes())
        }
        "basic-step" => {
            let (debugger, mb) = debugger(target)?;
            debugger
                .api("POST", "/api/basic_step", &[], "", mb)
                .map_err(failed)?;
            done()
        }
        "basic-run-to" => {
            let arg = arg!(
                command,
                "Run to the BASIC line",
                ArgKind::Number {
                    min: 0,
                    max: 65535,
                    default: 10
                }
            );
            let line = arg.number()?.to_string();
            let (debugger, mb) = debugger(target)?;
            debugger
                .api("POST", "/api/basic_runto", &[("line", &line)], "", mb)
                .map_err(CommandError::Invalid)?;
            done()
        }
        "basic-load" => {
            let arg = arg!(
                command,
                "Load a BASIC listing",
                ArgKind::OpenFile(TEXT_FILES)
            );
            let (_, bytes) = arg.bytes()?;
            let (debugger, mb) = debugger(target)?;
            debugger
                .api(
                    "POST",
                    "/api/basic",
                    &[("run", "1")],
                    &String::from_utf8_lossy(&bytes),
                    mb,
                )
                .map_err(CommandError::Invalid)?;
            done()
        }
        "debug-server-toggle" => {
            let listening = target.debugger().and_then(|d| d.listening_port()).is_some();
            host(target, HostRequest::DebugServer(!listening))
        }
        "debug-server-set-port" => {
            let arg = arg!(
                command,
                "The debug server's port",
                ArgKind::Number {
                    min: 1,
                    max: 65535,
                    default: super::DEFAULT_DEBUG_PORT as i64
                }
            );
            let port = u16::try_from(arg.number()?)
                .ok()
                .filter(|p| *p > 0)
                .ok_or_else(|| CommandError::Invalid("a port from 1 to 65535".into()))?;
            settings(target)?.debug_port = port;
            if target.debugger().and_then(|d| d.listening_port()).is_some() {
                target.request(HostRequest::DebugServer(true))?;
            }
            done()
        }
        "debug-server-open-doc" | "help-api-doc" => {
            let port = target
                .debugger()
                .and_then(|d| d.listening_port())
                .ok_or_else(|| {
                    CommandError::Unavailable("the debug server is not running".into())
                })?;
            host(
                target,
                HostRequest::OpenUrl(format!("http://127.0.0.1:{port}/api/doc")),
            )
        }
        "overlay-toggle" => {
            let settings = settings(target)?;
            settings.overlay = !settings.overlay;
            done()
        }
        // ------------------------------------------------------------- Machine
        "machine-ga-40007" | "machine-ga-40010" => {
            target
                .motherboard_mut()
                .ga_mut()
                .set_gate_array_type(if id == "machine-ga-40007" {
                    GateArrayType::Ga40007_8
                } else {
                    GateArrayType::Ga40010
                });
            done()
        }
        "machine-language-en" | "machine-language-fr" => {
            let layout = if id == "machine-language-fr" {
                KeyboardLayout::French
            } else {
                KeyboardLayout::Uk
            };
            let mb = target.motherboard_mut();
            mb.set_keyboard_layout(layout)
                .map_err(CommandError::Unavailable)?;
            mb.restart();
            done()
        }
        "machine-disc-interface-default"
        | "machine-disc-interface-ddi1"
        | "machine-disc-interface-none" => {
            let mb = target.motherboard_mut();
            let fitted = match id {
                "machine-disc-interface-default" => mb.model().has_disc_interface(),
                "machine-disc-interface-ddi1" => true,
                _ => false,
            };
            mb.set_disc_interface(fitted);
            said("disc interface changed: reset for AMSDOS to start")
        }
        "rom-load" => {
            let arg = arg!(
                command,
                "Upper ROM: SLOT=FILE (7=amsdos.rom, 5=inner://rulezcharge)",
                ArgKind::Text
            );
            let text = arg.text();
            let (slot, file) = text
                .split_once('=')
                .ok_or_else(|| CommandError::Invalid("SLOT=FILE".into()))?;
            let slot: u8 = slot
                .trim()
                .parse()
                .map_err(|_| CommandError::Invalid(format!("{slot}: a slot from 0 to 255")))?;
            let rom = crate::roms::load_rom(file.trim()).map_err(failed)?;
            target
                .motherboard_mut()
                .set_upper_rom(slot, Some(&rom))
                .map_err(failed)?;
            said(format!("{}: upper ROM {slot}", file.trim()))
        }
        "rom-remove" => {
            let arg = arg!(
                command,
                "Empty the upper ROM slot",
                ArgKind::Number {
                    min: 0,
                    max: 255,
                    default: 0
                }
            );
            let slot = u8::try_from(arg.number()?)
                .map_err(|_| CommandError::Invalid("a slot from 0 to 255".into()))?;
            target
                .motherboard_mut()
                .set_upper_rom(slot, None)
                .map_err(failed)?;
            done()
        }
        "rom-list" => host(
            target,
            HostRequest::ShowText {
                title: "Embedded ROMs".into(),
                text: crate::roms::listing(),
            },
        ),
        "keyboard-mapping-symbolic" | "keyboard-mapping-positional" | "keyboard-mapping-toggle" => {
            let settings = settings(target)?;
            settings.keyboard = match id {
                "keyboard-mapping-symbolic" => KeyboardMapping::Symbolic,
                "keyboard-mapping-positional" => KeyboardMapping::Positional,
                _ => settings.keyboard.other(),
            };
            said(format!("keyboard: {}", settings.keyboard.name()))
        }
        "expansion-nova-toggle" | "nova-select-file" => {
            #[cfg(feature = "nova")]
            {
                use crate::nova::Nova;
                if id == "expansion-nova-toggle" && target.motherboard().has_expansion::<Nova>() {
                    target.motherboard_mut().unplug_expansion::<Nova>();
                    return said("the Nova is unplugged");
                }
                let file = if target.has_file_system() {
                    let arg = arg!(
                        command,
                        "The Nova's NVRAM file",
                        ArgKind::SaveFile(&["nvram", "bin"], "nova.nvram")
                    );
                    Some(arg.path()?)
                } else {
                    None
                };
                replug(target.motherboard_mut(), Nova::new(file.clone()))?;
                if let Some(settings) = target.settings_mut() {
                    settings.devices.nova = file;
                }
                said("the Nova is plugged")
            }
            #[cfg(not(feature = "nova"))]
            Err(CommandError::Unsupported)
        }
        "expansion-dmp2000-toggle" => {
            let mb = target.motherboard();
            if has_dmp2000(mb) {
                target.motherboard_mut().unplug_printer();
                return said("the DMP2000 is unplugged");
            }
            let files = if target.has_file_system() {
                let arg = arg!(
                    command,
                    "Printout files: PREFIX (PREFIX.txt, .prn, _001.png...)",
                    ArgKind::SaveFile(&[], "printout")
                );
                let mut files = devices(target)?;
                files.dmp2000_prefix = Some(arg.text());
                settings(target)?.devices = files.clone();
                files
            } else {
                target
                    .settings()
                    .map(|s| s.devices.clone())
                    .unwrap_or_default()
            };
            target
                .motherboard_mut()
                .plug_printer(Box::new(dmp2000(&files)));
            said("the DMP2000 is plugged")
        }
        "printer-realtime-toggle" => {
            let printer = dmp2000_mut(target.motherboard_mut())?;
            let realtime = !printer.realtime();
            printer.set_realtime(realtime);
            done()
        }
        "printer-select-rom" => {
            let arg = arg!(
                command,
                "The printer's 16K EPROM",
                ArgKind::OpenFile(&["rom", "bin"])
            );
            let (_, bytes) = arg.bytes()?;
            dmp2000_mut(target.motherboard_mut())?
                .load_font_rom(&bytes)
                .map_err(failed)?;
            if let Some(settings) = target.settings_mut() {
                settings.devices.dmp2000_rom = Some(bytes.into());
            }
            done()
        }
        "printer-save-printout" => host(target, HostRequest::SavePrintout),
        "expansion-digiblaster-toggle" => {
            #[cfg(feature = "digiblaster")]
            {
                use crate::sound_cards::Digiblaster;
                let plugged = target
                    .motherboard()
                    .printer_port()
                    .is_some_and(|port| port.device_as::<Digiblaster>().is_some());
                let mb = target.motherboard_mut();
                if plugged {
                    mb.unplug_printer();
                } else {
                    mb.plug_printer(Box::new(Digiblaster::default()));
                }
                done()
            }
            #[cfg(not(feature = "digiblaster"))]
            Err(CommandError::Unsupported)
        }
        "expansion-playcity-toggle" => {
            #[cfg(feature = "playcity")]
            {
                use crate::sound_cards::PlayCity;
                let mb = target.motherboard_mut();
                if mb.has_expansion::<PlayCity>() {
                    mb.unplug_expansion::<PlayCity>();
                } else {
                    mb.plug_expansion(Box::new(PlayCity::default()));
                }
                done()
            }
            #[cfg(not(feature = "playcity"))]
            Err(CommandError::Unsupported)
        }
        "willy-opl3-toggle"
        | "willy-opl3-set-address"
        | "willy-s2p-toggle"
        | "willy-s2p-select-soundfont"
        | "willy-s2p-set-address" => {
            #[cfg(feature = "willy")]
            {
                use crate::willy::{parse_base, Opl3Lpt, S2p};
                let mut files = target
                    .settings()
                    .map(|s| s.devices.clone())
                    .unwrap_or_default();
                let opl3 = target.motherboard().has_expansion::<Opl3Lpt>();
                let s2p = target.motherboard().has_expansion::<S2p>();
                match id {
                    "willy-opl3-toggle" if opl3 => {
                        target.motherboard_mut().unplug_expansion::<Opl3Lpt>();
                        return done();
                    }
                    "willy-s2p-toggle" if s2p => {
                        target.motherboard_mut().unplug_expansion::<S2p>();
                        return done();
                    }
                    "willy-opl3-set-address" | "willy-s2p-set-address" => {
                        let arg = arg!(
                            command,
                            "Willy's base address",
                            ArgKind::Choice(WILLY_BASES)
                        );
                        let base = parse_base(&arg.text()).map_err(CommandError::Invalid)?;
                        if id == "willy-opl3-set-address" {
                            files.opl3_base = base;
                        } else {
                            files.s2p_base = base;
                        }
                    }
                    "willy-s2p-toggle" | "willy-s2p-select-soundfont"
                        if id == "willy-s2p-select-soundfont" || files.s2p_soundfont.is_none() =>
                    {
                        let arg = arg!(command, "The S2P's SoundFont", ArgKind::OpenFile(&["sf2"]));
                        let (name, data) = arg.bytes()?;
                        files.s2p_soundfont = Some((name, data.into()));
                    }
                    _ => {}
                }
                let mb = target.motherboard_mut();
                if id == "willy-opl3-toggle" || (id == "willy-opl3-set-address" && opl3) {
                    replug(mb, Ok(Opl3Lpt::new(files.opl3_base)))?;
                }
                if id.starts_with("willy-s2p") && (id != "willy-s2p-set-address" || s2p) {
                    if let Some((_, soundfont)) = &files.s2p_soundfont {
                        replug(mb, S2p::new(files.s2p_base, soundfont))?;
                    }
                }
                if let Some(settings) = target.settings_mut() {
                    settings.devices = files;
                }
                done()
            }
            #[cfg(not(feature = "willy"))]
            Err(CommandError::Unsupported)
        }
        "booster-none"
        | "booster-plus"
        | "booster-mini"
        | "booster-set-uart"
        | "booster-select-eeprom"
        | "booster-select-flash"
        | "booster-select-adc" => {
            #[cfg(feature = "booster")]
            {
                use crate::booster::{Booster, Model};
                if id == "booster-none" {
                    target.motherboard_mut().unplug_expansion::<Booster>();
                    return done();
                }
                let mut files = devices(target)?;
                let current = target
                    .motherboard()
                    .expansion_port()
                    .and_then(|port| port.device::<Booster>())
                    .map(Booster::model);
                let model = match id {
                    "booster-plus" => Model::BoosterPlus,
                    "booster-mini" => Model::MiniBooster,
                    _ => {
                        let (title, kind) = match id {
                            "booster-set-uart" => (
                                "UART: loopback, listen:ADDRESS or connect:ADDRESS",
                                ArgKind::Text,
                            ),
                            "booster-select-eeprom" => (
                                "The EEPROM's file",
                                ArgKind::SaveFile(&["bin"], "eeprom.bin"),
                            ),
                            "booster-select-flash" => (
                                "The program memory's file",
                                ArgKind::SaveFile(&["bin"], "flash.bin"),
                            ),
                            _ => (
                                "What the ADC samples (WAV or raw bytes)",
                                ArgKind::OpenFile(&["wav", "raw", "bin"]),
                            ),
                        };
                        let arg = arg!(command, title, kind);
                        match id {
                            "booster-set-uart" => files.booster_uart = Some(arg.text()),
                            "booster-select-eeprom" => files.booster_eeprom = Some(arg.path()?),
                            "booster-select-flash" => files.booster_flash = Some(arg.path()?),
                            _ => files.booster_adc = Some(arg.path()?),
                        }
                        current.ok_or_else(|| {
                            CommandError::Unavailable("no Booster is plugged".into())
                        })?
                    }
                };
                replug(target.motherboard_mut(), booster(&files, model))?;
                settings(target)?.devices = files;
                done()
            }
            #[cfg(not(feature = "booster"))]
            Err(CommandError::Unsupported)
        }
        // --------------------------------------------------------------- Audio
        "audio-stereo" | "audio-mono" => {
            settings(target)?.stereo = id == "audio-stereo";
            done()
        }
        "audio-mute-toggle" => {
            let settings = settings(target)?;
            settings.muted = !settings.muted;
            done()
        }
        "audio-volume-up" | "audio-volume-down" => {
            let settings = settings(target)?;
            settings.volume = if id == "audio-volume-up" {
                settings.volume.saturating_add(10).min(100)
            } else {
                settings.volume.saturating_sub(10)
            };
            said(format!("volume {}%", settings.volume))
        }
        "audio-volume-set" => {
            let default = target.settings().map_or(100, |s| s.volume) as i64;
            let arg = arg!(
                command,
                "Volume (%)",
                ArgKind::Number {
                    min: 0,
                    max: 100,
                    default
                }
            );
            settings(target)?.volume = arg.number()?.clamp(0, 100) as u8;
            done()
        }
        "wav-start" | "ym-start" | "video-start" | "screenshot-series-start" => {
            let (recording, title, kind) = match id {
                "wav-start" => (
                    Recording::Wav,
                    "Record the sound",
                    ArgKind::SaveFile(&["wav"], "bemu.wav"),
                ),
                "ym-start" => (
                    Recording::Ym,
                    "Record the AY registers",
                    ArgKind::SaveFile(&["ym"], "bemu.ym"),
                ),
                "video-start" => (
                    Recording::Video,
                    "Record a video",
                    ArgKind::SaveFile(&["mp4", "mkv", "webm", "gif"], "bemu.mp4"),
                ),
                _ => (
                    Recording::ScreenshotSeries,
                    "Screenshots: FOLDER [every 2|50f] [from 0]",
                    ArgKind::Text,
                ),
            };
            let path = if target.has_file_system() {
                Some(arg!(command, title, kind).path()?)
            } else {
                None
            };
            host(target, HostRequest::StartRecording(recording, path))
        }
        "wav-stop" => host(target, HostRequest::StopRecording(Recording::Wav)),
        "ym-stop" => host(target, HostRequest::StopRecording(Recording::Ym)),
        "video-stop" => host(target, HostRequest::StopRecording(Recording::Video)),
        "screenshot-series-stop" => host(
            target,
            HostRequest::StopRecording(Recording::ScreenshotSeries),
        ),
        // --------------------------------------------------------------- Video
        "screenshot-save" => {
            let path = if target.has_file_system() {
                Some(
                    arg!(
                        command,
                        "Save the screenshot",
                        ArgKind::SaveFile(&["png"], "bemu.png")
                    )
                    .path()?,
                )
            } else {
                None
            };
            host(target, HostRequest::Screenshot(path))
        }
        "screenshot-copy" => host(target, HostRequest::CopyScreenshot),
        "video-curvature-set" => {
            let default = target.settings().map_or(1.0, |s| f64::from(s.curvature));
            let arg = arg!(
                command,
                "Curvature (0: flat, to 4)",
                ArgKind::Decimal {
                    min: 0.0,
                    max: 4.0,
                    default
                }
            );
            let curvature: f32 = arg
                .text()
                .trim()
                .parse()
                .map_err(|_| CommandError::Invalid("a number from 0 to 4".into()))?;
            settings(target)?.curvature = curvature.clamp(0.0, 4.0);
            done()
        }
        "video-black-level-set" => {
            let default = target.settings().map_or(8, |s| s.black_level) as i64;
            let arg = arg!(
                command,
                "Black level (0-255)",
                ArgKind::Number {
                    min: 0,
                    max: 255,
                    default
                }
            );
            settings(target)?.black_level = arg.number()?.clamp(0, 255) as u8;
            done()
        }
        "video-crt-gpu-toggle" => {
            let settings = settings(target)?;
            settings.crt_gpu = !settings.crt_gpu;
            done()
        }
        "video-fullscreen-toggle" => {
            let settings = settings(target)?;
            settings.fullscreen = !settings.fullscreen;
            done()
        }
        // --------------------------------------------------------------- Tools
        "autotype-text" | "autotype-file" => {
            let text = if id == "autotype-text" {
                arg!(command, "Type (\\n: RETURN, \\(WAIT n)...)", ArgKind::Text).text()
            } else {
                let arg = arg!(
                    command,
                    "Type the text of a file",
                    ArgKind::OpenFile(TEXT_FILES)
                );
                String::from_utf8_lossy(&arg.bytes()?.1).into_owned()
            };
            target.type_text(&text)?;
            done()
        }
        "autotype-delays" => {
            let arg = arg!(
                command,
                "Typing: PRESS [AFTER_KEY [AFTER_TEXT]] in µs (70000 70000 400000)",
                ArgKind::Text
            );
            let delays: super::TypingDelays = arg.text().parse().map_err(CommandError::Invalid)?;
            settings(target)?.typing = Some(delays);
            if let Some(debugger) = target.debugger_mut() {
                debugger.set_key_delays(delays.press_us, delays.after_key_us);
            }
            done()
        }
        "keyboard-write" => {
            let arg = arg!(
                command,
                "Keyboard matrix: 10 rows, hexadecimal (FF: no key down)",
                ArgKind::Text
            );
            let text = arg.text();
            let rows = text
                .split([' ', ','])
                .filter(|word| !word.is_empty())
                .map(|word| {
                    u8::from_str_radix(word.trim_start_matches(['&', '#', '$']), 16)
                        .map_err(|_| CommandError::Invalid(format!("{word}: a hexadecimal byte")))
                })
                .collect::<Result<Vec<u8>, _>>()?;
            let lines: [u8; 10] = rows
                .try_into()
                .map_err(|_| CommandError::Invalid("10 rows".into()))?;
            target.motherboard_mut().keyboard_mut().set_lines(lines);
            done()
        }
        "snapshot-version-2" | "snapshot-version-3" => {
            settings(target)?.snapshot_version = if id.ends_with('2') { 2 } else { 3 };
            done()
        }
        "autotype-paste" => host(target, HostRequest::PasteClipboard),
        "tools-disc-catalog" => {
            let arg = arg!(
                command,
                "Catalogue of the disc",
                ArgKind::OpenFile(DISC_FILES)
            );
            let (image, name) = load_disc(&arg)?;
            let text = image
                .amsdos_catalog()
                .iter()
                .map(|entry| entry.filename())
                .collect::<Vec<_>>()
                .join("\n");
            host(target, HostRequest::ShowText { title: name, text })
        }
        "tools-disc-convert" => {
            let arg = arg!(
                command,
                "Convert the disc image (DSK <-> HFE)",
                ArgKind::OpenFile(&["dsk", "hfe"])
            );
            let (image, name) = load_disc(&arg)?;
            let output = match name.rsplit_once('.') {
                Some((stem, ext)) if ext.eq_ignore_ascii_case("hfe") => format!("{stem}.dsk"),
                Some((stem, _)) => format!("{stem}.hfe"),
                None => format!("{name}.hfe"),
            };
            if target.has_file_system() {
                save_disc_file(&output, &image).map_err(failed)?;
                said(format!("{output} written"))
            } else {
                host(
                    target,
                    HostRequest::SaveFile {
                        name: output,
                        data: image.to_bytes(),
                    },
                )
            }
        }
        "preferences-open" => host(target, HostRequest::Open(View::Preferences)),
        "help-shortcuts" => host(target, HostRequest::Open(View::Shortcuts)),
        "help-about" => host(target, HostRequest::Open(View::About)),
        "ui-menu-toggle" => {
            let settings = settings(target)?;
            settings.menu_visible = !settings.menu_visible;
            done()
        }
        _ => Err(CommandError::Unknown(id.to_owned())),
    }
}

fn host_can(target: &dyn CommandTarget, request: HostRequest) -> CommandState {
    CommandState::enabled(target.can(&request))
}

/// How the command `id` shows now in the menus.
pub fn state(id: &str, target: &dyn CommandTarget) -> CommandState {
    if NOT_IMPLEMENTED.contains(&id) {
        return CommandState::DISABLED;
    }
    if let Some(state) = disc_state(id, target) {
        return state;
    }
    let mb = target.motherboard();
    let settings = target.settings();
    let has_settings = settings.is_some();
    let debugger = target.debugger();
    let paused = paused(target);
    if let Some((_, view)) = VIEWS.iter().find(|(name, _)| *name == id) {
        return host_can(target, HostRequest::Open(*view));
    }
    if let Some(rest) = id.strip_prefix("machine-crtc-") {
        let n: u8 = rest.parse().unwrap_or(0);
        return CommandState {
            enabled: !mb.model().is_plus() || n == 3,
            checked: mb.crtc().crtc_type() as u8 == n,
        };
    }
    if let Some(rest) = id.strip_prefix("machine-platform-") {
        return CommandState::checked(
            PLATFORMS
                .iter()
                .any(|(n, m)| *n == rest && *m == mb.model()),
        );
    }
    if let Some(rest) = id.strip_prefix("machine-brand-") {
        return CommandState::checked(
            BRANDS
                .iter()
                .any(|(n, b)| *n == rest && *b == mb.ppi().brand()),
        );
    }
    if let Some(rest) = id.strip_prefix("machine-ram-") {
        let total = total_ram_kb(mb);
        let own = model_ram_kb(mb.model());
        // a 6128's own 128K is banked by its Gate Array: it cannot have less
        let below_own = rest.parse::<usize>().is_ok_and(|kb| kb < own);
        return CommandState {
            enabled: !below_own,
            checked: match rest {
                "default" => total == own,
                kb => kb.parse() == Ok(total) && total != own,
            },
        };
    }
    if let Some(rest) = id.strip_prefix("speed-") {
        return match settings {
            Some(s) => CommandState::checked(
                SPEEDS
                    .iter()
                    .any(|(n, speed)| *n == rest && *speed == s.speed),
            ),
            None => CommandState::DISABLED,
        };
    }
    if let Some(rest) = id.strip_prefix("video-rendering-") {
        return match settings {
            Some(_) if rest == "next" => {
                CommandState::enabled(!target.is_recording(Recording::Video))
            }
            Some(s) => CommandState::checked(
                RENDERINGS
                    .iter()
                    .any(|(n, m)| *n == rest && *m == s.display),
            ),
            None => CommandState::DISABLED,
        };
    }
    if let Some(rest) = id.strip_prefix("printer-charset-") {
        let printer = mb
            .printer_port()
            .and_then(|p| p.device_as::<crate::dmp2000::Dmp2000>());
        return match printer {
            Some(printer) => CommandState::checked(
                PRINTER_CHARSETS.get(usize::from(printer.international_set())) == Some(&rest),
            ),
            None => CommandState::DISABLED,
        };
    }
    if let Some(rest) = id.strip_prefix("keyboard-host-layout-") {
        return match settings {
            Some(s) => CommandState::checked(matches!(
                (rest, s.host_layout),
                ("auto", None)
                    | ("qwerty", Some(HostLayout::Qwerty))
                    | ("azerty", Some(HostLayout::Azerty))
                    | ("qwertz", Some(HostLayout::Qwertz))
            )),
            None => CommandState::DISABLED,
        };
    }
    if id.starts_with("rom-set-") {
        #[allow(unused_mut)]
        let mut available = id != "rom-set-rulezcharge";
        #[cfg(feature = "m4")]
        {
            available |= mb.has_expansion::<crate::m4::M4Board>();
        }
        let embedded = rom_set(id, mb)
            .iter()
            .all(|(_, name)| crate::roms::load_rom(name).is_ok());
        return CommandState {
            enabled: available && embedded,
            checked: rom_set_fitted(id, mb),
        };
    }
    let files = target.has_file_system();
    let toggle = |on: bool| {
        if has_settings {
            CommandState::checked(on)
        } else {
            CommandState::DISABLED
        }
    };
    let needs_settings = CommandState::enabled(has_settings);
    let needs_debugger = CommandState::enabled(debugger.is_some());
    #[allow(unused_variables)]
    let expansion = |plugged: bool| CommandState::checked(plugged);
    match id {
        "drives-swap" => {
            CommandState::enabled(mb.fdc().has_disk(FdcDrive::A) || mb.fdc().has_disk(FdcDrive::B))
        }
        "discs-save-automatically-toggle" => CommandState {
            enabled: has_settings && files,
            checked: settings.is_some_and(|s| s.save_discs),
        },
        "tape-insert" => CommandState::enabled(true),
        "tape-record" => CommandState {
            enabled: true,
            checked: mb.tape_recorder().is_some(),
        },
        "tape-record-stop" => CommandState::enabled(mb.tape_recorder().is_some()),
        "tape-eject" | "tape-rewind" => CommandState::enabled(mb.tape_deck().is_some()),
        "tape-play" => CommandState {
            enabled: mb.tape_deck().is_some(),
            checked: mb.tape_deck().is_some_and(|deck| deck.playing()),
        },
        "tape-stop" => CommandState::enabled(mb.tape_deck().is_some_and(|deck| deck.playing())),
        #[cfg(feature = "albireo")]
        "albireo-select-path" => CommandState::enabled(has_settings),
        #[cfg(feature = "albireo")]
        "albireo-eject" => CommandState::enabled(mb.has_expansion::<crate::ch376::Albireo>()),
        #[cfg(feature = "albireo")]
        "expansion-albireo-toggle" => CommandState {
            enabled: has_settings,
            checked: mb.has_expansion::<crate::ch376::Albireo>(),
        },
        #[cfg(feature = "m4")]
        "m4-select-folder" => CommandState::enabled(files && has_settings),
        #[cfg(feature = "m4")]
        "m4-eject" => CommandState::enabled(mb.has_expansion::<crate::m4::M4Board>()),
        #[cfg(feature = "m4")]
        "expansion-m4-toggle" => CommandState {
            enabled: files && has_settings,
            checked: mb.has_expansion::<crate::m4::M4Board>(),
        },
        #[cfg(feature = "m4")]
        "m4-select-rom" | "m4-set-slot" => CommandState::enabled(files && has_settings),
        #[cfg(feature = "m4")]
        "m4-wifi-toggle" => CommandState {
            enabled: files && has_settings && cfg!(feature = "m4-wifi"),
            checked: settings.is_some_and(|s| s.devices.m4_wifi),
        },
        "cartridge-insert" | "cartridge-remove" => CommandState::enabled(mb.model().is_plus()),
        #[cfg(feature = "dandanator")]
        "dandanator-select-romset" => needs_settings,
        #[cfg(feature = "dandanator")]
        "dandanator-eject" => {
            CommandState::enabled(mb.has_expansion::<crate::dandanator::Dandanator>())
        }
        #[cfg(feature = "dandanator")]
        "expansion-dandanator-toggle" => CommandState {
            enabled: has_settings,
            checked: mb.has_expansion::<crate::dandanator::Dandanator>(),
        },
        #[cfg(feature = "dandanator")]
        "dandanator-writable-toggle" => CommandState {
            enabled: has_settings
                && files
                && settings.is_some_and(|s| s.devices.dandanator.is_some()),
            checked: settings.is_some_and(|s| s.devices.dandanator_writable),
        },
        "snapshot-load" => CommandState::enabled(true),
        "snapshot-save" => CommandState::enabled(true),
        "snapshot-update" => {
            CommandState::enabled(files && settings.is_some_and(|s| s.media.snapshot.is_some()))
        }
        "snapshot-quick-save" => needs_settings,
        "snapshot-save-on-exit-toggle" => CommandState {
            enabled: has_settings && files,
            checked: settings.is_some_and(|s| s.save_snapshot_on_exit.is_some()),
        },
        "config-load" | "config-save" | "config-reset" => CommandState::enabled(true),
        "csl-load" => host_can(
            target,
            HostRequest::RunCsl {
                text: String::new(),
                folder: None,
            },
        ),
        "csl-stop" => CommandState::enabled(target.csl_running()),
        "files-watch-toggle" => CommandState {
            enabled: has_settings && files,
            checked: settings.is_some_and(|s| s.watch_files),
        },
        // a page decides itself whether it closes (its embedder); the debug server quits
        "exit" => CommandState::enabled(target.can(&HostRequest::Quit) || !has_settings),
        "debug-run" => CommandState::enabled(paused),
        "debug-pause" => CommandState::enabled(!paused),
        "debug-reset" | "machine-reset-soft" | "machine-reset-hard" => CommandState::enabled(true),
        "debug-step-in"
        | "debug-step-over"
        | "debug-step-out"
        | "debug-run-to-address"
        | "debug-run-to-beam"
        | "breakpoint-add"
        | "breakpoint-toggle"
        | "breakpoint-remove"
        | "breakpoint-add-advanced"
        | "breakpoint-add-raster"
        | "breakpoint-add-basic"
        | "counter-new"
        | "symbols-load"
        | "history-set-size"
        | "basic-step"
        | "basic-run-to"
        | "basic-load"
        | "autotype-text"
        | "autotype-file" => needs_debugger,
        "debug-step-frame" => CommandState::enabled(debugger.is_some() && paused),
        "debug-set-pc" | "memory-poke" | "z80-set-register" => CommandState::enabled(true),
        "breakpoints-clear" => CommandState::enabled(debugger.is_some_and(|d| d.has_breakpoints())),
        "counter-toggle" | "counter-reset" | "counter-delete" => {
            CommandState::enabled(debugger.is_some_and(|d| !d.counters().is_empty()))
        }
        "basic-export" => CommandState::enabled(crate::basic_program::program(mb).is_ok()),
        "debug-server-toggle" => CommandState {
            enabled: target.can(&HostRequest::DebugServer(true)),
            checked: debugger.and_then(|d| d.listening_port()).is_some(),
        },
        "debug-server-set-port" => {
            CommandState::enabled(has_settings && target.can(&HostRequest::DebugServer(true)))
        }
        "debug-server-open-doc" | "help-api-doc" => CommandState::enabled(
            debugger.and_then(|d| d.listening_port()).is_some()
                && target.can(&HostRequest::OpenUrl(String::new())),
        ),
        "overlay-toggle" => toggle(settings.is_some_and(|s| s.overlay)),
        "machine-ga-40007" => {
            CommandState::checked(mb.ga().gate_array_type() == GateArrayType::Ga40007_8)
        }
        "machine-ga-40010" => {
            CommandState::checked(mb.ga().gate_array_type() == GateArrayType::Ga40010)
        }
        "machine-language-en" => CommandState::checked(mb.keyboard_layout() == KeyboardLayout::Uk),
        "machine-language-fr" => CommandState {
            enabled: matches!(mb.model(), CpcModel::Cpc6128 | CpcModel::Cpc6128Plus),
            checked: mb.keyboard_layout() == KeyboardLayout::French,
        },
        "machine-disc-interface-default" => {
            CommandState::checked(mb.has_disc_interface() == mb.model().has_disc_interface())
        }
        "machine-disc-interface-ddi1" => CommandState {
            enabled: mb.model() == CpcModel::Cpc464,
            checked: mb.model() == CpcModel::Cpc464 && mb.has_disc_interface(),
        },
        "machine-disc-interface-none" => {
            CommandState::checked(!mb.has_disc_interface() && mb.model().has_disc_interface())
        }
        "rom-load" | "rom-remove" => CommandState::enabled(true),
        "rom-list" => host_can(
            target,
            HostRequest::ShowText {
                title: String::new(),
                text: String::new(),
            },
        ),
        "keyboard-mapping-symbolic" => {
            toggle(settings.is_some_and(|s| s.keyboard == KeyboardMapping::Symbolic))
        }
        "keyboard-mapping-positional" => {
            toggle(settings.is_some_and(|s| s.keyboard == KeyboardMapping::Positional))
        }
        "keyboard-mapping-toggle" => needs_settings,
        #[cfg(feature = "nova")]
        "expansion-nova-toggle" => CommandState {
            enabled: true,
            checked: mb.has_expansion::<crate::nova::Nova>(),
        },
        #[cfg(feature = "nova")]
        "nova-select-file" => CommandState::enabled(files),
        "expansion-dmp2000-toggle" => CommandState::checked(has_dmp2000(mb)),
        "printer-realtime-toggle" => match mb
            .printer_port()
            .and_then(|p| p.device_as::<crate::dmp2000::Dmp2000>())
        {
            Some(printer) => CommandState::checked(printer.realtime()),
            None => CommandState::DISABLED,
        },
        "printer-select-rom" => CommandState::enabled(has_dmp2000(mb)),
        "printer-save-printout" => {
            CommandState::enabled(has_dmp2000(mb) && target.can(&HostRequest::SavePrintout))
        }
        #[cfg(feature = "digiblaster")]
        "expansion-digiblaster-toggle" => CommandState::checked(
            mb.printer_port()
                .is_some_and(|p| p.device_as::<crate::sound_cards::Digiblaster>().is_some()),
        ),
        #[cfg(feature = "playcity")]
        "expansion-playcity-toggle" => {
            CommandState::checked(mb.has_expansion::<crate::sound_cards::PlayCity>())
        }
        #[cfg(feature = "willy")]
        "willy-opl3-toggle" => expansion(mb.has_expansion::<crate::willy::Opl3Lpt>()),
        #[cfg(feature = "willy")]
        "willy-s2p-toggle" => expansion(mb.has_expansion::<crate::willy::S2p>()),
        #[cfg(feature = "willy")]
        "willy-opl3-set-address" | "willy-s2p-select-soundfont" | "willy-s2p-set-address" => {
            needs_settings
        }
        #[cfg(feature = "booster")]
        "booster-none" | "booster-plus" | "booster-mini" => {
            use crate::booster::{Booster, Model};
            let model = mb
                .expansion_port()
                .and_then(|p| p.device::<Booster>())
                .map(Booster::model);
            CommandState {
                enabled: id == "booster-none" || has_settings,
                checked: match id {
                    "booster-none" => model.is_none(),
                    "booster-plus" => model == Some(Model::BoosterPlus),
                    _ => model == Some(Model::MiniBooster),
                },
            }
        }
        #[cfg(feature = "booster")]
        "booster-set-uart"
        | "booster-select-eeprom"
        | "booster-select-flash"
        | "booster-select-adc" => CommandState::enabled(
            has_settings && files && mb.has_expansion::<crate::booster::Booster>(),
        ),
        "audio-stereo" => toggle(settings.is_some_and(|s| s.stereo)),
        "audio-mono" => toggle(settings.is_some_and(|s| !s.stereo)),
        "audio-mute-toggle" => toggle(settings.is_some_and(|s| s.muted)),
        "audio-volume-up" | "audio-volume-down" | "audio-volume-set" => needs_settings,
        "wav-start" | "ym-start" | "video-start" | "screenshot-series-start" => {
            let recording = match id {
                "wav-start" => Recording::Wav,
                "ym-start" => Recording::Ym,
                "video-start" => Recording::Video,
                _ => Recording::ScreenshotSeries,
            };
            // running, it stops
            let running = target.is_recording(recording);
            CommandState {
                enabled: running || target.can(&HostRequest::StartRecording(recording, None)),
                checked: running,
            }
        }
        "wav-stop" => CommandState::enabled(target.is_recording(Recording::Wav)),
        "ym-stop" => CommandState::enabled(target.is_recording(Recording::Ym)),
        "video-stop" => CommandState::enabled(target.is_recording(Recording::Video)),
        "screenshot-series-stop" => {
            CommandState::enabled(target.is_recording(Recording::ScreenshotSeries))
        }
        "screenshot-save" => host_can(target, HostRequest::Screenshot(None)),
        "screenshot-copy" => host_can(target, HostRequest::CopyScreenshot),
        "video-curvature-set" | "video-black-level-set" => needs_settings,
        // a page always draws the CRT on the GPU (or raw on Canvas 2D)
        "video-crt-gpu-toggle" => CommandState {
            enabled: has_settings && cfg!(feature = "crt-gpu") && files,
            checked: settings.is_some_and(|s| s.crt_gpu),
        },
        "video-fullscreen-toggle" => toggle(settings.is_some_and(|s| s.fullscreen)),
        "autotype-paste" => host_can(target, HostRequest::PasteClipboard),
        "autotype-delays" => needs_settings,
        "keyboard-write" => CommandState::enabled(true),
        "snapshot-version-2" | "snapshot-version-3" => toggle(
            settings.is_some_and(|s| s.snapshot_version == if id.ends_with('2') { 2 } else { 3 }),
        ),
        "tools-disc-catalog" => CommandState::enabled(true),
        "tools-disc-convert" => CommandState::enabled(true),
        "preferences-open" => host_can(target, HostRequest::Open(View::Preferences)),
        "help-shortcuts" => host_can(target, HostRequest::Open(View::Shortcuts)),
        "help-about" => host_can(target, HostRequest::Open(View::About)),
        "ui-menu-toggle" => toggle(settings.is_some_and(|s| s.menu_visible)),
        // a card left out of this build
        _ => CommandState::DISABLED,
    }
}
