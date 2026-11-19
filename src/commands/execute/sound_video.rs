//! The sound and the picture: audio, recordings, screenshots, the CRT: what each command does
//! ([`run`]), then when it is available ([`state`]).

use super::*;

/// Run `command` if it is one of this area's ([`elsewhere`] otherwise).
pub(super) fn run(command: &Command, target: &mut dyn CommandTarget) -> CommandResult {
    let id = command.id;
    match id {
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
        "video-brightness-set" => {
            let default = target.settings().map_or(0, |s| s.brightness) as i64;
            let arg = arg!(
                command,
                "Brightness (-100 to 100)",
                ArgKind::Number {
                    min: -100,
                    max: 100,
                    default
                }
            );
            settings(target)?.brightness = arg.number()?.clamp(-100, 100) as i8;
            done()
        }
        "video-hhold-set" => {
            let default = target.settings().map_or(0, |s| s.hhold) as i64;
            let arg = arg!(
                command,
                "H-hold (-100 to 100)",
                ArgKind::Number {
                    min: -100,
                    max: 100,
                    default
                }
            );
            settings(target)?.hhold = arg.number()?.clamp(-100, 100) as i8;
            done()
        }
        "video-vhold-set" => {
            let default = target.settings().map_or(0, |s| s.vhold) as i64;
            let arg = arg!(
                command,
                "V-hold (-100 to 100)",
                ArgKind::Number {
                    min: -100,
                    max: 100,
                    default
                }
            );
            settings(target)?.vhold = arg.number()?.clamp(-100, 100) as i8;
            done()
        }
        "video-crt-gpu-toggle" => {
            let settings = settings(target)?;
            settings.crt_gpu = !settings.crt_gpu;
            done()
        }
        "video-free-zoom-toggle" => {
            let settings = settings(target)?;
            settings.free_zoom = !settings.free_zoom;
            done()
        }
        "video-native-zoom-toggle" => {
            let settings = settings(target)?;
            settings.native_zoom = !settings.native_zoom;
            done()
        }
        "video-fullscreen-toggle" => {
            let settings = settings(target)?;
            settings.fullscreen = !settings.fullscreen;
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
        files,
        ..
    } = *cx;
    let toggle = |on: bool| cx.toggle(on);
    let needs_settings = CommandState::enabled(has_settings);
    Some(match id {
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
        "video-curvature-set" | "video-brightness-set" | "video-hhold-set" | "video-vhold-set" => {
            needs_settings
        }
        // a page always draws the CRT on the GPU (or raw on Canvas 2D)
        "video-crt-gpu-toggle" => CommandState {
            enabled: has_settings && cfg!(feature = "crt-gpu") && files,
            checked: settings.is_some_and(|s| s.crt_gpu),
        },
        "video-free-zoom-toggle" => toggle(settings.is_some_and(|s| s.free_zoom)),
        "video-native-zoom-toggle" => toggle(settings.is_some_and(|s| s.native_zoom)),
        "video-fullscreen-toggle" => toggle(settings.is_some_and(|s| s.fullscreen)),
        _ => return None,
    })
}
