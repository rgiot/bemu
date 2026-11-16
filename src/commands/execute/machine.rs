//! The machine: resets, Gate Array, language, ROMs, keyboard and joysticks: what each command does
//! ([`run`]), then when it is available ([`state`]).

use super::*;

/// Run `command` if it is one of this area's ([`elsewhere`] otherwise).
pub(super) fn run(command: &Command, target: &mut dyn CommandTarget) -> CommandResult {
    let id = command.id;
    match id {
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
        "joystick-1-none"
        | "joystick-1-host"
        | "joystick-1-simulated"
        | "joystick-2-none"
        | "joystick-2-host"
        | "joystick-2-simulated" => {
            let settings = settings(target)?;
            let (slot, value) = match id {
                "joystick-1-none" => (&mut settings.joystick_1, JoystickSource::None),
                "joystick-1-host" => (&mut settings.joystick_1, JoystickSource::Host),
                "joystick-1-simulated" => (&mut settings.joystick_1, JoystickSource::Simulated),
                "joystick-2-none" => (&mut settings.joystick_2, JoystickSource::None),
                "joystick-2-host" => (&mut settings.joystick_2, JoystickSource::Host),
                _ => (&mut settings.joystick_2, JoystickSource::Simulated),
            };
            *slot = value;
            if value == JoystickSource::Simulated {
                target.request(HostRequest::Open(View::Joystick)).ok();
            }
            said(format!(
                "joystick {}: {}",
                id.trim_start_matches("joystick-"),
                value.name()
            ))
        }
        "joystick-simulated-window" => host(target, HostRequest::Open(View::Joystick)),
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
        ..
    } = *cx;
    let toggle = |on: bool| cx.toggle(on);
    let needs_settings = CommandState::enabled(has_settings);
    Some(match id {
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
        "joystick-1-none" | "joystick-1-host" | "joystick-1-simulated" => {
            let source = id.strip_prefix("joystick-1-").unwrap();
            CommandState::checked(
                settings.is_some_and(|s| s.joystick_1 == source.parse().unwrap_or_default()),
            )
        }
        "joystick-2-none" | "joystick-2-host" | "joystick-2-simulated" => {
            let source = id.strip_prefix("joystick-2-").unwrap();
            CommandState::checked(
                settings.is_some_and(|s| s.joystick_2 == source.parse().unwrap_or_default()),
            )
        }
        "joystick-simulated-window" => CommandState::enabled(true),
        "keyboard-write" => CommandState::enabled(true),
        "keyboard-view" => CommandState::enabled(true),
        _ => return None,
    })
}
