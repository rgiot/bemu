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
use crate::joystick::JoystickSource;
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
/// What the commands' states read, once for all.
#[derive(Clone, Copy)]
pub(super) struct StateCx<'a> {
    target: &'a dyn CommandTarget,
    mb: &'a Mb,
    settings: Option<&'a super::HostSettings>,
    has_settings: bool,
    debugger: Option<&'a crate::debug_server::DebugServer>,
    paused: bool,
    /// The host has files (not a page).
    files: bool,
}

impl<'a> StateCx<'a> {
    fn of(target: &'a dyn CommandTarget) -> Self {
        let settings = target.settings();
        Self {
            target,
            mb: target.motherboard(),
            settings,
            has_settings: settings.is_some(),
            debugger: target.debugger(),
            paused: paused(target),
            files: target.has_file_system(),
        }
    }

    /// A setting on or off (unavailable without the host's settings).
    fn toggle(&self, on: bool) -> CommandState {
        if self.has_settings {
            CommandState::checked(on)
        } else {
            CommandState::DISABLED
        }
    }
}

/// An area's answer for a command not its own: another area's (see [`execute`]).
fn elsewhere() -> CommandError {
    CommandError::Unknown(String::new())
}

fn is_elsewhere(error: &CommandError) -> bool {
    matches!(error, CommandError::Unknown(id) if id.is_empty())
}

/// The areas of the commands, each its commands' actions and their states side by side.
mod choices;
mod debug;
mod devices;
mod machine;
mod media;
mod sound_video;
mod tools;

type AreaRun = fn(&Command, &mut dyn CommandTarget) -> CommandResult;
type AreaState = fn(&str, &StateCx) -> Option<CommandState>;

const AREA_RUNS: [AreaRun; 6] = [
    media::run,
    debug::run,
    machine::run,
    devices::run,
    sound_video::run,
    tools::run,
];

const AREA_STATES: [AreaState; 6] = [
    media::state,
    debug::state,
    machine::state,
    devices::state,
    sound_video::state,
    tools::state,
];

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
    if let Some(result) = disc_command(&command, target) {
        return result;
    }
    if let Some(view) = View::opened_by(id) {
        return host(target, HostRequest::Open(view));
    }
    if let Some(result) = choices::run(id, target) {
        return result;
    }
    for run in AREA_RUNS {
        match run(&command, target) {
            Err(e) if is_elsewhere(&e) => continue,
            result => return result,
        }
    }
    Err(CommandError::Unknown(id.to_owned()))
}

fn host_can(target: &dyn CommandTarget, request: HostRequest) -> CommandState {
    CommandState::enabled(target.can(&request))
}

/// How the command `id` shows now in the menus.
pub fn state(id: &str, target: &dyn CommandTarget) -> CommandState {
    if let Some(state) = disc_state(id, target) {
        return state;
    }
    let cx = StateCx::of(target);
    if let Some(view) = View::opened_by(id) {
        let state = host_can(target, HostRequest::Open(view));
        return CommandState::enabled(state.enabled && view.fits(cx.mb.model()));
    }
    if let Some(state) = choices::state(id, &cx) {
        return state;
    }
    AREA_STATES
        .iter()
        .find_map(|state| state(id, &cx))
        // a card left out of this build
        .unwrap_or(CommandState::DISABLED)
}

/// The interface's zoom: 50 % to 300 %.
const UI_ZOOM_MIN: f32 = 0.5;
const UI_ZOOM_MAX: f32 = 3.0;

/// The next zoom up (`direction` 1) or down (-1): the steps of 10 % (25 % above 200 %).
fn ui_zoom_step(zoom: f32, direction: i32) -> f32 {
    let percent = (zoom * 100.0).round() as i32;
    let step = if percent > 200 || (percent == 200 && direction > 0) {
        25
    } else {
        10
    };
    // to the next multiple of the step, so 115 % goes to 120 % or 110 %
    let next = if direction > 0 {
        (percent / step + 1) * step
    } else {
        (percent + step - 1) / step * step - step
    };
    (next as f32 / 100.0).clamp(UI_ZOOM_MIN, UI_ZOOM_MAX)
}

#[cfg(test)]
mod ui_zoom_test {
    use super::ui_zoom_step;

    #[test]
    fn the_interface_zooms_by_steps() {
        assert_eq!(ui_zoom_step(1.0, 1), 1.1);
        assert_eq!(ui_zoom_step(1.0, -1), 0.9);
        assert_eq!(ui_zoom_step(1.15, 1), 1.2);
        assert_eq!(ui_zoom_step(1.15, -1), 1.1);
        assert_eq!(ui_zoom_step(2.0, 1), 2.25);
        assert_eq!(ui_zoom_step(2.25, -1), 2.0);
        assert_eq!(ui_zoom_step(0.5, -1), 0.5);
        assert_eq!(ui_zoom_step(3.0, 1), 3.0);
    }
}
