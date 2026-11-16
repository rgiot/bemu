//! The expansions: Albireo, M4, Nova, the printer, Dandanator, Willy, the boosters...: what each
//! command does ([`run`]), then when it is available ([`state`]).

use super::*;

/// Run `command` if it is one of this area's ([`elsewhere`] otherwise).
pub(super) fn run(command: &Command, target: &mut dyn CommandTarget) -> CommandResult {
    let id = command.id;
    match id {
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
        _ => Err(elsewhere()),
    }
}

/// How `id` shows, if it is one of this area's.
#[allow(unused_variables)]
pub(super) fn state(id: &str, cx: &StateCx) -> Option<CommandState> {
    let StateCx {
        target,
        mb,
        settings,
        has_settings,
        debugger,
        files,
        ..
    } = *cx;
    let toggle = |on: bool| cx.toggle(on);
    let needs_settings = CommandState::enabled(has_settings);
    let expansion = |plugged: bool| CommandState::checked(plugged);
    Some(match id {
        #[cfg(feature = "albireo")]
        "albireo-select-path" => CommandState::enabled(has_settings),
        #[cfg(feature = "albireo")]
        "expansion-albireo-toggle" => CommandState {
            enabled: has_settings,
            checked: mb.has_expansion::<crate::ch376::Albireo>(),
        },
        #[cfg(feature = "albireo")]
        "albireo-eject" => CommandState::enabled(mb.has_expansion::<crate::ch376::Albireo>()),
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
        _ => return None,
    })
}
