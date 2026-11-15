//! The radio families of the menus: the items of one submenu, `<prefix><choice>`, each family
//! what its choice does and when a choice is available or chosen, side by side.

use super::*;

/// A family: its id prefix, what a choice (the id's rest) does, how a choice shows.
pub(super) struct Family {
    pub prefix: &'static str,
    run: fn(&str, &str, &mut dyn CommandTarget) -> CommandResult,
    state: fn(&str, &str, &StateCx) -> CommandState,
}

/// The choice `id` of its family, run: `None` when `id` is no family's.
pub(super) fn run(id: &str, target: &mut dyn CommandTarget) -> Option<CommandResult> {
    let family = FAMILIES.iter().find(|f| id.starts_with(f.prefix))?;
    Some((family.run)(id, &id[family.prefix.len()..], target))
}

/// How the choice `id` of its family shows: `None` when `id` is no family's.
pub(super) fn state(id: &str, cx: &StateCx) -> Option<CommandState> {
    let family = FAMILIES.iter().find(|f| id.starts_with(f.prefix))?;
    Some((family.state)(id, &id[family.prefix.len()..], cx))
}

pub(super) static FAMILIES: &[Family] = &[
    Family {
        prefix: "machine-crtc-",
        run: machine_crtc_run,
        state: machine_crtc_state,
    },
    Family {
        prefix: "machine-platform-",
        run: machine_platform_run,
        state: machine_platform_state,
    },
    Family {
        prefix: "machine-brand-",
        run: machine_brand_run,
        state: machine_brand_state,
    },
    Family {
        prefix: "machine-ram-",
        run: machine_ram_run,
        state: machine_ram_state,
    },
    Family {
        prefix: "speed-",
        run: speed_run,
        state: speed_state,
    },
    Family {
        prefix: "video-rendering-",
        run: video_rendering_run,
        state: video_rendering_state,
    },
    Family {
        prefix: "printer-charset-",
        run: printer_charset_run,
        state: printer_charset_state,
    },
    Family {
        prefix: "keyboard-host-layout-",
        run: keyboard_host_layout_run,
        state: keyboard_host_layout_state,
    },
    Family {
        prefix: "rom-set-",
        run: rom_set_run,
        state: rom_set_state,
    },
];

// machine-crtc-*
fn machine_crtc_run(id: &str, rest: &str, target: &mut dyn CommandTarget) -> CommandResult {
    let n: u8 = rest.parse().map_err(|_| CommandError::Unknown(id.into()))?;
    let crtc = CrtcType::try_from(n).map_err(|_| CommandError::Unknown(id.into()))?;
    target
        .motherboard_mut()
        .change_crtc(crtc)
        .map_err(CommandError::Unavailable)?;
    said(format!("CRTC {n}"))
}

fn machine_crtc_state(_: &str, rest: &str, cx: &StateCx) -> CommandState {
    let StateCx { mb, .. } = *cx;
    let n: u8 = rest.parse().unwrap_or(0);
    return CommandState {
        enabled: !mb.model().is_plus() || n == 3,
        checked: mb.crtc().crtc_type() as u8 == n,
    };
}

// machine-platform-*
fn machine_platform_run(id: &str, rest: &str, target: &mut dyn CommandTarget) -> CommandResult {
    let (_, model) = PLATFORMS
        .iter()
        .find(|(name, _)| *name == rest)
        .ok_or_else(|| CommandError::Unknown(id.into()))?;
    target.motherboard_mut().change_model(*model);
    said(model.name())
}

fn machine_platform_state(_: &str, rest: &str, cx: &StateCx) -> CommandState {
    let StateCx { mb, .. } = *cx;
    return CommandState::checked(
        PLATFORMS
            .iter()
            .any(|(n, m)| *n == rest && *m == mb.model()),
    );
}

// machine-brand-*
fn machine_brand_run(id: &str, rest: &str, target: &mut dyn CommandTarget) -> CommandResult {
    let rest = if rest == "lsp" { "lsp" } else { rest };
    let (_, brand) = BRANDS
        .iter()
        .find(|(name, _)| *name == rest)
        .ok_or_else(|| CommandError::Unknown(id.into()))?;
    target.motherboard_mut().ppi_mut().set_brand(*brand);
    said("brand changed: the firmware shows it at its next reset")
}

fn machine_brand_state(_: &str, rest: &str, cx: &StateCx) -> CommandState {
    let StateCx { mb, .. } = *cx;
    return CommandState::checked(
        BRANDS
            .iter()
            .any(|(n, b)| *n == rest && *b == mb.ppi().brand()),
    );
}

// machine-ram-*
fn machine_ram_run(id: &str, rest: &str, target: &mut dyn CommandTarget) -> CommandResult {
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
    said(format!("{}K of RAM", total_ram_kb(target.motherboard())))
}

fn machine_ram_state(_: &str, rest: &str, cx: &StateCx) -> CommandState {
    let StateCx { mb, .. } = *cx;
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

// speed-*
fn speed_run(id: &str, rest: &str, target: &mut dyn CommandTarget) -> CommandResult {
    // F10: the maximum speed, or back to real time
    if rest == "max-toggle" {
        let settings = settings(target)?;
        settings.speed = if settings.speed == Speed::Max {
            Speed::Factor(1.0)
        } else {
            Speed::Max
        };
        return done();
    }
    let (_, speed) = SPEEDS
        .iter()
        .find(|(name, _)| *name == rest)
        .ok_or_else(|| CommandError::Unknown(id.into()))?;
    settings(target)?.speed = *speed;
    done()
}

fn speed_state(_: &str, rest: &str, cx: &StateCx) -> CommandState {
    let StateCx { settings, .. } = *cx;
    if rest == "max-toggle" {
        return cx.toggle(settings.is_some_and(|s| s.speed == Speed::Max));
    }
    match settings {
        Some(s) => CommandState::checked(
            SPEEDS
                .iter()
                .any(|(n, speed)| *n == rest && *speed == s.speed),
        ),
        None => CommandState::DISABLED,
    }
}

// video-rendering-*
fn video_rendering_run(id: &str, rest: &str, target: &mut dyn CommandTarget) -> CommandResult {
    let settings = settings(target)?;
    settings.display = match rest {
        "next" => settings.display.next(),
        name => RENDERINGS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, mode)| *mode)
            .ok_or_else(|| CommandError::Unknown(id.into()))?,
    };
    done()
}

fn video_rendering_state(_: &str, rest: &str, cx: &StateCx) -> CommandState {
    let StateCx {
        target, settings, ..
    } = *cx;
    match settings {
        Some(_) if rest == "next" => CommandState::enabled(!target.is_recording(Recording::Video)),
        Some(s) => CommandState::checked(
            RENDERINGS
                .iter()
                .any(|(n, m)| *n == rest && *m == s.display),
        ),
        None => CommandState::DISABLED,
    }
}

// printer-charset-*
fn printer_charset_run(id: &str, rest: &str, target: &mut dyn CommandTarget) -> CommandResult {
    let set = PRINTER_CHARSETS
        .iter()
        .position(|name| *name == rest)
        .ok_or_else(|| CommandError::Unknown(id.into()))?;
    dmp2000_mut(target.motherboard_mut())?.set_international_set(set as u8);
    done()
}

fn printer_charset_state(_: &str, rest: &str, cx: &StateCx) -> CommandState {
    let StateCx { mb, .. } = *cx;
    let printer = mb
        .printer_port()
        .and_then(|p| p.device_as::<crate::dmp2000::Dmp2000>());
    match printer {
        Some(printer) => CommandState::checked(
            PRINTER_CHARSETS.get(usize::from(printer.international_set())) == Some(&rest),
        ),
        None => CommandState::DISABLED,
    }
}

// keyboard-host-layout-*
fn keyboard_host_layout_run(id: &str, rest: &str, target: &mut dyn CommandTarget) -> CommandResult {
    settings(target)?.host_layout = match rest {
        "auto" => None,
        "qwerty" => Some(HostLayout::Qwerty),
        "azerty" => Some(HostLayout::Azerty),
        "qwertz" => Some(HostLayout::Qwertz),
        _ => return Err(CommandError::Unknown(id.into())),
    };
    done()
}

fn keyboard_host_layout_state(_: &str, rest: &str, cx: &StateCx) -> CommandState {
    let StateCx { settings, .. } = *cx;
    match settings {
        Some(s) => CommandState::checked(matches!(
            (rest, s.host_layout),
            ("auto", None)
                | ("qwerty", Some(HostLayout::Qwerty))
                | ("azerty", Some(HostLayout::Azerty))
                | ("qwertz", Some(HostLayout::Qwertz))
        )),
        None => CommandState::DISABLED,
    }
}

// rom-set-*
fn rom_set_run(id: &str, _: &str, target: &mut dyn CommandTarget) -> CommandResult {
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

fn rom_set_state(id: &str, _: &str, cx: &StateCx) -> CommandState {
    let StateCx { mb, .. } = *cx;
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
