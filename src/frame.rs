//! One frame of the machine, as every frontend runs it (the window, the web page, the debug
//! server without a window): the beam to the monitor, the I/O writes to the overlay, each
//! instruction through the debugger when it has something to do (a breakpoint, a step, a
//! counter...), and a CSL script played to its next frame.

use image::RgbImage;

use crate::csl_runner::{CslReport, CslRunner, Progress};
use crate::debug_server::{Beam, DebugServer};
use crate::monitor::Monitor;
use crate::motherboard::Motherboard;
use crate::overlay::Overlay;

/// The T-states of a PAL frame (312 lines of 64 µs).
pub const FRAME_T_STATES: usize = 312 * 64 * 4;

/// A CSL script is played this many T-states at a time, between two looks at its picture.
const CSL_SLICE_T_STATES: usize = 20_000;

/// The I/O writes of the last instruction on `overlay`, where the beam is.
pub fn note_io_writes(overlay: &mut Overlay, motherboard: &mut Motherboard, monitor: &Monitor) {
    let (beam, origin) = (monitor.beam(), monitor.crop_origin());
    for (port, value) in motherboard.take_io_writes() {
        overlay.note(port, value, beam, origin);
    }
}

/// Run `motherboard` to its next whole frame (`Some`), until the debugger stops it (a
/// breakpoint, a step: `None`, the monitor keeping the part of the frame drawn), or for
/// `max_t_states` (a machine whose CRTC makes no frame: `None`). The overlay notes the I/O
/// writes (drawn on the frame by its owner); the debugger, when active, sees each instruction
/// and the frames (its `/api/screenshot` picture).
pub fn run_frame(
    motherboard: &mut Motherboard,
    monitor: &mut Monitor,
    mut debugger: Option<&mut DebugServer>,
    mut overlay: Option<&mut Overlay>,
    max_t_states: usize,
) -> Option<RgbImage> {
    let start = motherboard.t_state_counter().value();
    let mut frame = None;
    while frame.is_none() {
        motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
            monitor.receive(rgb, sync);
            if frame.is_none() {
                frame = monitor.take_frame();
            }
        });
        if let Some(overlay) = overlay.as_deref_mut() {
            note_io_writes(overlay, motherboard, monitor);
        }
        if let Some(debugger) = debugger.as_deref_mut().filter(|d| d.is_active()) {
            let (x, line) = monitor.beam();
            let (crop_x, crop_y) = monitor.crop_origin();
            debugger.set_beam(Beam {
                x,
                line,
                crop_x,
                crop_y,
            });
            debugger.after_instruction(motherboard);
            if debugger.paused() {
                return None;
            }
        }
        if frame.is_none()
            && motherboard.t_state_counter().value().wrapping_sub(start) > max_t_states
        {
            return None;
        }
    }
    if let (Some(frame), Some(debugger)) = (&frame, debugger) {
        debugger.frame_completed(motherboard);
        if debugger.is_active() {
            debugger.set_frame(frame);
        }
    }
    frame
}

/// What a CSL script did in [`play_frame`].
pub enum ScriptStep {
    /// A frame completed: its picture is the runner's `last_frame`.
    Frame,
    /// A frame's time passed without a whole frame.
    Running,
    /// The script ended (its report), or stopped on an error.
    Done(Result<CslReport, String>),
}

/// Play `runner`'s script to its next frame, a frame's time at most (the frontends show a
/// frame a refresh).
pub fn play_frame(runner: &mut CslRunner) -> ScriptStep {
    let mut played = 0;
    while played < FRAME_T_STATES {
        match runner.advance(CSL_SLICE_T_STATES) {
            Ok(Progress::Frame) => return ScriptStep::Frame,
            Ok(Progress::Running) => played += CSL_SLICE_T_STATES,
            Ok(Progress::Done(report)) => return ScriptStep::Done(Ok(report)),
            Err(e) => return ScriptStep::Done(Err(e)),
        }
    }
    ScriptStep::Running
}

/// A script about to play on `runner` takes over from `machine`: the same drives, and the
/// sound goes on through the runner's machine (given back with [`end_script`]).
pub fn start_script(runner: &mut CslRunner, machine: &mut Motherboard) {
    for unit in 0..2 {
        let drive = if unit == 0 {
            crate::fdc::FdcDrive::A
        } else {
            crate::fdc::FdcDrive::B
        };
        runner.set_drive_kind(unit, machine.fdc().drive_kind(drive));
    }
    runner
        .motherboard_mut()
        .psg_mut()
        .take_audio_output_from(machine.psg_mut());
}

/// The machine of a script that ended (or was stopped): it goes on as the frontend's, the
/// sound with it.
pub fn end_script(runner: &mut CslRunner) -> Motherboard {
    std::mem::take(runner.motherboard_mut())
}
