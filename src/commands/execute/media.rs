//! The discs, tapes, cartridges, snapshots, configurations and CSL scripts: what each command does
//! ([`run`]), then when it is available ([`state`]).

use super::*;

/// Run `command` if it is one of this area's ([`elsewhere`] otherwise).
pub(super) fn run(command: &Command, target: &mut dyn CommandTarget) -> CommandResult {
    let id = command.id;
    match id {
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
            let config = crate::commands::config::save(target);
            save(target, &arg, "bemu.json", config.into_bytes())
        }
        "config-load" => {
            let arg = arg!(
                command,
                "Load a configuration",
                ArgKind::OpenFile(&["json"])
            );
            let (_, bytes) = arg.bytes()?;
            let notes = crate::commands::config::load(target, &String::from_utf8_lossy(&bytes))?;
            if notes.is_empty() {
                said("configuration loaded")
            } else {
                said(format!("configuration loaded, but: {}", notes.join("; ")))
            }
        }
        "config-reset" => {
            crate::commands::config::load(target, "{}")?;
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
        "csl-log-save" => {
            let text = target
                .csl_log()
                .map(|log| log.text())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| CommandError::Unavailable("no CSL script played yet".into()))?;
            let arg = arg!(
                command,
                "Save the CSL log",
                ArgKind::SaveFile(TEXT_FILES, "csl-log.txt")
            );
            save(target, &arg, "csl-log.txt", text.into_bytes())
        }
        "files-watch-toggle" => {
            let settings = settings(target)?;
            settings.watch_files = !settings.watch_files;
            done()
        }
        "snapshot-version-2" | "snapshot-version-3" => {
            settings(target)?.snapshot_version = if id.ends_with('2') { 2 } else { 3 };
            done()
        }
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
    Some(match id {
        "drives-swap" => {
            CommandState::enabled(mb.fdc().has_disk(FdcDrive::A) || mb.fdc().has_disk(FdcDrive::B))
        }
        "discs-save-automatically-toggle" => CommandState {
            enabled: has_settings && files,
            checked: settings.is_some_and(|s| s.save_discs),
        },
        "tape-insert" => CommandState::enabled(true),
        "tape-eject" | "tape-rewind" => CommandState::enabled(mb.tape_deck().is_some()),
        "tape-record" => CommandState {
            enabled: true,
            checked: mb.tape_recorder().is_some(),
        },
        "tape-record-stop" => CommandState::enabled(mb.tape_recorder().is_some()),
        "tape-play" => CommandState {
            enabled: mb.tape_deck().is_some(),
            checked: mb.tape_deck().is_some_and(|deck| deck.playing()),
        },
        "tape-stop" => CommandState::enabled(mb.tape_deck().is_some_and(|deck| deck.playing())),
        "cartridge-insert" | "cartridge-remove" => CommandState::enabled(mb.model().is_plus()),
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
        "csl-log-save" => CommandState::enabled(
            target
                .csl_log()
                .is_some_and(|log| !log.entries().is_empty()),
        ),
        "files-watch-toggle" => CommandState {
            enabled: has_settings && files,
            checked: settings.is_some_and(|s| s.watch_files),
        },
        "snapshot-version-2" | "snapshot-version-3" => toggle(
            settings.is_some_and(|s| s.snapshot_version == if id.ends_with('2') { 2 } else { 3 }),
        ),
        "tools-disc-catalog" => CommandState::enabled(true),
        "tools-disc-convert" => CommandState::enabled(true),
        _ => return None,
    })
}
