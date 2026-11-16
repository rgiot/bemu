//! The debugger: running and stepping, breakpoints, counters, symbols, the edits of the machine,
//! BASIC, the debug server: what each command does ([`run`]), then when it is available
//! ([`state`]).

use super::*;

/// Run `command` if it is one of this area's ([`elsewhere`] otherwise).
pub(super) fn run(command: &Command, target: &mut dyn CommandTarget) -> CommandResult {
    let id = command.id;
    match id {
        "debug-run" => set_paused(target, false),
        "debug-pause" => set_paused(target, true),
        // F8
        "debug-pause-toggle" => {
            let paused = paused(target);
            set_paused(target, !paused)
        }
        "debug-reset" | "machine-reset-soft" | "machine-reset-hard" => {
            let hard = id == "machine-reset-hard";
            if hard {
                target.power_cycle();
            } else {
                target.motherboard_mut().restart();
            }
            if let Some(debugger) = target.debugger_mut() {
                debugger.reset_done(hard);
                // a reset starts the machine again, even stopped in the debugger
                debugger.set_paused(false);
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
        "ga-set-ink" => {
            let arg = arg!(
                command,
                "Pen (0-15, 16 or B the border) then ink (0-26): 1 24",
                ArgKind::Text
            );
            let text = arg.text();
            let mut words = text.split_whitespace();
            let pen = match words.next().map(str::to_uppercase).as_deref() {
                Some("B" | "BORDER") => 16,
                Some(word) => word
                    .parse::<u8>()
                    .ok()
                    .filter(|&pen| pen <= 16)
                    .ok_or_else(|| CommandError::Invalid(format!("{word}: not a pen")))?,
                None => return Err(CommandError::Invalid("PEN INK".into())),
            };
            let ink = words
                .next()
                .and_then(|word| word.parse::<u8>().ok())
                .filter(|&ink| ink <= 26)
                .ok_or_else(|| CommandError::Invalid("an ink 0-26".into()))?;
            target
                .motherboard_mut()
                .ga_mut()
                .set_ink(crate::ga::Pen::from(pen), crate::ga::Ink::from(ink));
            done()
        }
        "ga-set-mode" => {
            let arg = arg!(
                command,
                "Screen mode",
                ArgKind::Number {
                    min: 0,
                    max: 3,
                    default: 1
                }
            );
            let mode = arg
                .number()
                .ok()
                .filter(|mode| (0..=3).contains(mode))
                .ok_or_else(|| CommandError::Invalid("a mode 0-3".into()))?
                as u8;
            target
                .motherboard_mut()
                .ga_mut()
                .set_video_mode(crate::ga::VideoMode::from(mode));
            done()
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
                // a shadow pair: its two halves (A2 and F2...), as the debug API names them
                "AF'" | "BC'" | "DE'" | "HL'" => {
                    let (high, low) = match name.as_str() {
                        "AF'" => ("A2", "F2"),
                        "BC'" => ("B2", "C2"),
                        "DE'" => ("D2", "E2"),
                        _ => ("H2", "L2"),
                    };
                    let (debugger, mb) = debugger(target)?;
                    debugger
                        .api(
                            "POST",
                            "/api/z80",
                            &[],
                            &format!("{{\"{high}\":{},\"{low}\":{}}}", value >> 8, value & 0xff),
                            mb,
                        )
                        .map_err(CommandError::Invalid)?;
                }
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
        "history-record-toggle" => {
            let debugger = target.debugger_mut().ok_or(CommandError::Unsupported)?;
            debugger.set_recording_history(!debugger.records_history());
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
                    default: crate::commands::DEFAULT_DEBUG_PORT as i64
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
        paused,
        ..
    } = *cx;
    let toggle = |on: bool| cx.toggle(on);
    let needs_debugger = CommandState::enabled(debugger.is_some());
    Some(match id {
        "debug-run" => CommandState::enabled(paused),
        "debug-pause" => CommandState::enabled(!paused),
        "debug-pause-toggle" => CommandState {
            enabled: debugger.is_some() || cx.has_settings,
            checked: paused,
        },
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
        "debug-set-pc" | "memory-poke" | "z80-set-register" | "ga-set-ink" | "ga-set-mode" => {
            CommandState::enabled(true)
        }
        "breakpoints-clear" => CommandState::enabled(debugger.is_some_and(|d| d.has_breakpoints())),
        "counter-toggle" | "counter-reset" | "counter-delete" => {
            CommandState::enabled(debugger.is_some_and(|d| !d.counters().is_empty()))
        }
        "history-record-toggle" => CommandState {
            enabled: debugger.is_some(),
            checked: debugger.is_some_and(|d| d.records_history()),
        },
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
        _ => return None,
    })
}
