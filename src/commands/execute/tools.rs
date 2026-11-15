//! The interface and the tools: auto-type, windows, zoom, exit: what each command does ([`run`]),
//! then when it is available ([`state`]).

use super::*;

/// Run `command` if it is one of this area's ([`elsewhere`] otherwise).
pub(super) fn run(command: &Command, target: &mut dyn CommandTarget) -> CommandResult {
    let id = command.id;
    match id {
        "exit" => match target.request(HostRequest::Quit) {
            Err(CommandError::Unsupported) => {
                debugger(target)?.0.request_quit();
                done()
            }
            other => other.map(|()| Outcome::Done(None)),
        },
        "ui-zoom-in" | "ui-zoom-out" | "ui-zoom-reset" => {
            let settings = settings(target)?;
            settings.ui_zoom = match id {
                "ui-zoom-in" => ui_zoom_step(settings.ui_zoom, 1),
                "ui-zoom-out" => ui_zoom_step(settings.ui_zoom, -1),
                _ => 1.0,
            };
            done()
        }
        "autotype-text" | "autotype-file" => {
            let text = if id == "autotype-text" {
                // `\n` typed in a field is RETURN, as the prompt says (a CSL string has already
                // turned its own into newlines)
                arg!(command, "Type (\\n: RETURN, \\(WAIT n)...)", ArgKind::Text)
                    .text()
                    .replace("\\n", "\n")
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
            let delays: crate::commands::TypingDelays =
                arg.text().parse().map_err(CommandError::Invalid)?;
            settings(target)?.typing = Some(delays);
            if let Some(debugger) = target.debugger_mut() {
                debugger.set_key_delays(delays.press_us, delays.after_key_us);
            }
            done()
        }
        "autotype-paste" => host(target, HostRequest::PasteClipboard),
        "view-memory-new" => host(target, HostRequest::OpenAnother(View::Memory)),
        "view-disassembly-new" => host(target, HostRequest::OpenAnother(View::Disassembly)),
        "ui-menu-toggle" => {
            let settings = settings(target)?;
            settings.menu_visible = !settings.menu_visible;
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
        settings,
        has_settings,
        debugger,
        ..
    } = *cx;
    let toggle = |on: bool| cx.toggle(on);
    let needs_settings = CommandState::enabled(has_settings);
    Some(match id {
        // a page decides itself whether it closes (its embedder); the debug server quits
        "exit" => CommandState::enabled(target.can(&HostRequest::Quit) || !has_settings),
        "ui-zoom-in" => CommandState::enabled(settings.is_some_and(|s| s.ui_zoom < UI_ZOOM_MAX)),
        "ui-zoom-out" => CommandState::enabled(settings.is_some_and(|s| s.ui_zoom > UI_ZOOM_MIN)),
        "ui-zoom-reset" => {
            CommandState::enabled(settings.is_some_and(|s| (s.ui_zoom - 1.0).abs() > 1e-3))
        }
        "autotype-paste" => host_can(target, HostRequest::PasteClipboard),
        "autotype-delays" => needs_settings,
        "view-memory-new" => host_can(target, HostRequest::OpenAnother(View::Memory)),
        "view-disassembly-new" => host_can(target, HostRequest::OpenAnother(View::Disassembly)),
        "ui-menu-toggle" => toggle(settings.is_some_and(|s| s.menu_visible)),
        _ => return None,
    })
}
