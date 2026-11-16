//! HTTP debug server compatible with the AMSpiriT-Lite web API
//! (<https://github.com/AMSpiriT-Emulator/amspirit-releases/blob/main/docs/lite/web_api.md>),
//! meant to be driven by debug front ends such as `cpclib-dap`.
//!
//! Same contract as AMSpiriT's: loopback only, port 6128 by default, JSON
//! bodies whose keys are looked up at any depth, `400 {"error","field"}` for
//! anything the endpoint cannot use, `405` with an `Allow` header for a
//! method the path does not document, and *commands* (anything changing the
//! machine) answered at once with a `cmd_seq` that `emu.applied_cmd_seq`
//! reaches once the command is applied and the emulation ran after it.
//!
//! The HTTP side runs on its own thread and only parses requests: every
//! request is answered by the emulation thread, from [`DebugServer::poll`],
//! between two instructions - so a reply always describes a consistent
//! machine. The emulation loop drives the server with [`DebugServer::poll`],
//! [`DebugServer::after_instruction`] and [`DebugServer::frame_completed`],
//! and stops running while [`DebugServer::paused`].
//!
//! Every request is parsed into the types of [`bemu_api`] (the `cpclib-bemu-api` crate, shared
//! with clients) and every reply and event built from them; the OpenAPI description
//! (`/api/doc?format=openapi`) is generated from the same types (`http.rs`, `api_doc.rs`).
//!
//! The routes ([`ROUTES`]): `ping`, `state`, `z80`, `ram` (every 16K bank, or as the CPU sees
//! it), `exec`, `step` (in, over, out, a frame), `run_to`, `config` (pause/resume, resets, the
//! history's size), `z80_bp` (CPU addresses resolved to the RAM bank or ROM holding them,
//! `Bnn:hhhh` physical locations), `keytype`, `events` (Server-Sent Events), `history`,
//! `screenshot`, `media`, `snapshot`, the chips (`memmap`, `crtc`, `ga`, `psg`, `fdc`, `drive`),
//! BASIC (`basic`, `basic_state`, `basic_listing`, `basic_export`, `basic_bp`, `basic_step`,
//! `basic_runto`, see [`crate::basic_program`]), the beam (`beam`, `raster_bp`, in AMSpiriT's
//! video buffer coordinates or in NOPs: 64 a line, 19968 a frame) and `doc`.
//!
//! Extensions to the AMSpiriT API - listed in `/api/doc` with `"extension":true`, and in
//! `/api/ping`'s `features` - add what debuggers of rasm/basm/ACE programs need:
//! `/api/breakpoints` for advanced breakpoints (memory/IO watchpoints, ranges and masks, stop or
//! watch, conditions, see [`crate::breakpoints`]), the `watch` event for watch hits,
//! `reason`/`kind`/`name`/`access`/`beam`/`counters`/`cmd_seq` in `z80_bp` events, `/api/counters`
//! (named counters of the NOPs run), `/api/run_to`, the step modes, the symbols. Replies identify
//! this emulator with `"frontend":"bemu"`.

mod api_doc;
#[cfg(not(target_arch = "wasm32"))]
mod http;

use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use bemu_api::*;
use bemu_api as api;
use serde_json::{json, Value};

use crate::basic_program::{self, BasicState};
use crate::breakpoints::{
    AdvancedBreakpoint, BreakpointAccess, BreakpointKind, Condition, ConditionContext,
    CounterAction, RunMode,
};
use crate::commands::{Arg, Command, CommandTarget, Outcome};
use crate::keyboard::CpcKey;
use crate::memory::PhysicalAddress;
use crate::monitor::Monitor;
use crate::motherboard::{BusAccess, BusAccessKind, Motherboard};
use crate::z80::{HasValue, InterruptMode, Reg16, Reg8};

/// AMSpiriT's port, the CPC 6128's name.
pub const DEFAULT_PORT: u16 = 6128;

/// T-states in one emulated microsecond (AMSpiriT's "ticks" are NOPs).
const T_STATES_PER_NOP: usize = 4;
/// A typed key is held 2 frames then released 2 frames (40ms each).
const KEY_T_STATES: usize = 4_000_000 / 25;

/// One HTTP request, handed to the emulation thread.
#[derive(Debug)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    /// The raw body (media uploads are binary).
    pub body: Vec<u8>,
}

impl ApiRequest {
    /// The body as text (JSON, breakpoint lists).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// The answer to an [`ApiRequest`].
#[derive(Debug, PartialEq)]
pub struct ApiResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// Extra headers (`Allow` of a 405, screenshot geometry).
    pub headers: Vec<(String, String)>,
}

impl ApiResponse {
    fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: value.to_string().into_bytes(),
            headers: Vec::new(),
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn ok(value: Value) -> Self {
        Self::json(200, value)
    }
}

/// A request the endpoint cannot use: `400 {"error", "field"}`.
#[derive(Debug)]
struct ApiError {
    error: String,
    field: Option<String>,
}

impl ApiError {
    fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            field: None,
        }
    }

    fn field(field: &str, error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            field: Some(field.to_owned()),
        }
    }

    fn response(self) -> ApiResponse {
        let mut body = json!({ "error": self.error });
        if let Some(field) = self.field {
            body["field"] = json!(field);
        }
        ApiResponse::json(400, body)
    }
}

type ApiResult = Result<ApiResponse, ApiError>;

/// One documented path: its methods and summary, for routing, `405` and
/// `GET /api/doc`.
struct Route {
    path: &'static str,
    methods: &'static [&'static str],
    summary: &'static str,
    /// Not part of the AMSpiriT API.
    extension: bool,
}

const ROUTES: &[Route] = &[
    Route { path: "/api/ping", methods: &["GET"], summary: "connectivity probe, with the emu object", extension: false },
    Route { path: "/api/state", methods: &["GET"], summary: "z80, ga, crtc, psg and emu state", extension: false },
    Route { path: "/api/z80", methods: &["GET", "POST"], summary: "read / write the Z80 registers", extension: false },
    Route { path: "/api/ram", methods: &["GET", "POST"], summary: "read / write RAM (bank, view=raw|cpu), optional jump", extension: false },
    Route { path: "/api/exec", methods: &["POST"], summary: "set PC", extension: false },
    Route { path: "/api/step", methods: &["POST"], summary: "execute one instruction and pause (?mode=over|out|frame: step over, out, a frame)", extension: false },
    Route { path: "/api/run_to", methods: &["POST"], summary: "run to an address or symbol (addr), or a beam position (line, nop), once", extension: true },
    Route { path: "/api/config", methods: &["GET", "POST"], summary: "pause/resume, soft/hard reset", extension: false },
    Route { path: "/api/z80_bp", methods: &["POST", "DELETE"], summary: "replace the PC breakpoints (?add=1: add them): CPU addresses, symbols (label, label+n) or Bnn:hhhh, comma-separated; DELETE removes those listed", extension: false },
    Route { path: "/api/symbols", methods: &["GET"], summary: "the symbols (snapshot SYMB/REMU labels, --sym files), by address", extension: true },
    Route { path: "/api/keytype", methods: &["POST"], summary: "type text on the CPC keyboard (\\r = RETURN)", extension: false },
    Route { path: "/api/events", methods: &["GET"], summary: "Server-Sent Events: frame, z80_bp, basic_bp, basic_error, pause, reset, watch (?topics=)", extension: false },
    Route { path: "/api/breakpoints", methods: &["GET", "POST", "DELETE"], summary: "advanced breakpoints: list, add (JSON or REMU acebreak text), remove (?index=)", extension: true },
    Route { path: "/api/quit", methods: &["POST"], summary: "leave the emulator", extension: false },
    Route { path: "/api/history", methods: &["GET"], summary: "last executed instructions (?n=), bytes as fetched and as now", extension: false },
    Route { path: "/api/screenshot", methods: &["GET"], summary: "frame as PNG: crop=1 visible 768x272 | crop=0 whole 1024x350; live=1 the frame being drawn; full=0 over the previous one", extension: false },
    Route { path: "/api/snapshot", methods: &["GET"], summary: "the machine's state as a version 3 .sna file (save it, load it back with /api/media)", extension: true },
    Route { path: "/api/media", methods: &["POST"], summary: "load a .sna/.dsk/.cdt (by content) or .bin/.amsdos (?name=, ?drive=)", extension: false },
    Route { path: "/api/memmap", methods: &["GET"], summary: "what is mapped in each 16K region (ROM, absolute RAM bank), RMR, MMR", extension: false },
    Route { path: "/api/crtc", methods: &["GET"], summary: "CRTC registers, selected register and counters", extension: false },
    Route { path: "/api/ga", methods: &["GET"], summary: "Gate Array: mode, pens, inks, RMR and banking", extension: false },
    Route { path: "/api/psg", methods: &["GET"], summary: "PSG registers, decoded", extension: false },
    Route { path: "/api/fdc", methods: &["GET"], summary: "FDC status registers, phase, motor and heads; the discs as the head sees them", extension: false },
    Route { path: "/api/asic", methods: &["GET"], summary: "CPC+ ASIC: lock, palette, registers, DMA channels, sprites", extension: true },
    Route { path: "/api/drive", methods: &["POST"], summary: "a drive (?drive=0|1): its kind (kind=3|3.5), turn its disc over (flip=1, 3\" drive)", extension: true },
    Route { path: "/api/beam", methods: &["GET"], summary: "beam position (?crop=1: relative to the screenshot), also in NOPs: nop, line, frame_nop", extension: false },
    Route { path: "/api/raster_bp", methods: &["GET", "POST"], summary: "beam breakpoint: ?x=&y= (or line=, nop=, frame_nop=), enable=0 clears, add=1 keeps the others", extension: false },
    Route { path: "/api/counters", methods: &["GET", "POST", "DELETE"], summary: "named NOP counters: list, {name, action: create|reset|start|stop}, delete (?name=)", extension: true },
    Route { path: "/api/basic", methods: &["POST"], summary: "load a BASIC program (text body) in place of the one in RAM (?reset=1, ?run=1)", extension: false },
    Route { path: "/api/basic_state", methods: &["GET"], summary: "BASIC program and variable areas, line and statement being run", extension: false },
    Route { path: "/api/basic_export", methods: &["GET"], summary: "the BASIC program in RAM as text", extension: false },
    Route { path: "/api/basic_listing", methods: &["GET"], summary: "the BASIC program as lines of statements, with their addresses", extension: false },
    Route { path: "/api/basic_step", methods: &["POST"], summary: "run to the next BASIC statement (?mode=line: line)", extension: false },
    Route { path: "/api/basic_bp", methods: &["GET", "POST"], summary: "BASIC line breakpoints: comma-separated line numbers, empty clears", extension: false },
    Route { path: "/api/basic_runto", methods: &["POST"], summary: "run to a BASIC line (?line=) or statement (?addr=), once", extension: false },
    Route { path: "/api/doc", methods: &["GET"], summary: "this documentation: ?endpoint= one in detail, ?format=openapi|html", extension: false }
];

/// Where a BASIC step or run-to stops.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BasicTarget {
    /// The next statement run.
    Statement,
    /// The next line, or the same line run again from its start: from
    /// (line, statement).
    LineAfter(Option<u16>),
    /// The start of a line.
    Line(u16),
    /// A statement, by its address in `/api/basic_listing`.
    Address(u16),
}

/// A program to load once the firmware booted after a reset.
struct PendingBasic {
    source: String,
    run: bool,
    at_t_states: usize,
}

/// T-states the firmware takes, after a reset, before BASIC is ready.
pub const BASIC_BOOT_T_STATES: usize = 4_000_000 * 3 / 2;

/// The beam: where the monitor draws, in its ticks (16 per NOP, 1024 for a
/// 64 NOP line) and lines since the frame started, with the origin of the
/// visible picture (what the screenshot shows).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Beam {
    pub x: usize,
    pub line: usize,
    pub crop_x: isize,
    pub crop_y: usize,
}

/// Ticks of the monitor in one NOP (1us), NOPs in a line and lines in a
/// standard frame: 64 NOPs a line, 19968 a frame.
pub const BEAM_TICKS_PER_NOP: usize = 16;
pub const NOPS_PER_LINE: usize = 64;
pub const LINES_PER_FRAME: usize = 312;
const BEAM_TICKS_PER_LINE: usize = BEAM_TICKS_PER_NOP * NOPS_PER_LINE;

impl Beam {
    /// Ticks since the frame started (lines counted as 1024 ticks).
    fn linear(&self) -> usize {
        self.line * BEAM_TICKS_PER_LINE + self.x
    }

    fn nop(&self) -> usize {
        self.x / BEAM_TICKS_PER_NOP
    }

    fn event_beam(&self) -> EventBeam {
        EventBeam {
            x: self.x as u64,
            y: self.line as u64,
            nop: self.nop() as u64,
            line: self.line as u64,
            frame_nop: (self.line * NOPS_PER_LINE + self.nop()) as u64,
        }
    }

    /// The beam, `x` and `y` relative to the visible picture with `crop`.
    fn state(&self, crop: bool) -> BeamState {
        let (x, y) = if crop {
            (self.x as i64 - self.crop_x as i64, self.line as i64 - self.crop_y as i64)
        } else {
            (self.x as i64, self.line as i64)
        };
        BeamState {
            x,
            y,
            nop: self.nop() as u64,
            line: self.line as u64,
            frame_nop: (self.line * NOPS_PER_LINE + self.nop()) as u64,
            cx: self.crop_x as i64,
            cy: self.crop_y as i64,
            cw: SCREENSHOT_SIZE.0 as u64,
            ch: SCREENSHOT_SIZE.1 as u64,
        }
    }
}

/// A beam breakpoint: a position in the frame, or (no line) a position in
/// every line.
#[derive(Clone, Debug)]
struct RasterBreakpoint {
    /// The line; `None`: every line.
    line: Option<usize>,
    /// In monitor ticks.
    x: usize,
    /// Shown in its stop event.
    name: Option<String>,
}

impl RasterBreakpoint {
    /// Whether the beam went past it between `before` and `after` (one
    /// instruction).
    fn passed(&self, before: Beam, after: Beam) -> bool {
        match self.line {
            Some(line) => {
                let target = line * BEAM_TICKS_PER_LINE + self.x;
                let (before, after) = (before.linear(), after.linear());
                if after >= before {
                    before < target && target <= after
                } else {
                    // A new frame started.
                    target > before || target <= after
                }
            }
            None if after.line == before.line && after.x >= before.x => {
                before.x < self.x && self.x <= after.x
            }
            None => self.x > before.x || self.x <= after.x,
        }
    }

    fn typed(&self, index: usize) -> api::RasterBreakpoint {
        api::RasterBreakpoint {
            index: index as u64,
            x: self.x as u64,
            nop: (self.x / BEAM_TICKS_PER_NOP) as u64,
            name: self.name.clone(),
            y: self.line.map(|line| line as u64),
            line: self.line.map(|line| line as u64),
            frame_nop: self
                .line
                .map(|line| (line * NOPS_PER_LINE + self.x / BEAM_TICKS_PER_NOP) as u64),
        }
    }
}

/// A named counter of emulated NOPs (T-states / 4), running or stopped.
#[derive(Clone, Debug)]
struct NopCounter {
    t_states: u64,
    running: bool,
}

/// The server: owned by the emulation thread.
pub struct DebugServer {
    requests: Receiver<(ApiRequest, Sender<ApiResponse>)>,
    paused: bool,
    /// Last `cmd_seq` given, and the last one applied.
    cmd_seq: u64,
    applied_cmd_seq: u64,
    /// Commands given but waiting for the emulation to run (resume) or
    /// for the instruction to be executed (step).
    unapplied: bool,
    stepping: bool,
    ram_seq: u64,
    /// `z80_bp`: execution breakpoints on physical locations.
    exec_breakpoints: HashSet<PhysicalAddress>,
    /// `/api/breakpoints` (extension).
    advanced: Vec<AdvancedBreakpoint>,
    events: EventHub,
    /// The events for a client in the same process (the web page's debugger), see
    /// [`Self::local_events`]; subscribed on its first call.
    local_events: Option<Arc<(Mutex<SseQueue>, Condvar)>>,
    /// Pause state last reported by a `pause` event.
    reported_paused: bool,
    frames: u64,
    fps: f64,
    fps_window: (Instant, u64),
    typing: VecDeque<Vec<CpcKey>>,
    typed_pressed: Option<Vec<CpcKey>>,
    typing_next_at: usize,
    /// Stops reported (breakpoint hits), for the emulation loop to show.
    last_stop: Option<u16>,
    quit: bool,
    /// Executed instructions, oldest first: (PC, 4 bytes as fetched).
    history: VecDeque<(u16, [u8; 4])>,
    history_size: usize,
    /// Record the history even with nothing else to do after each instruction (the History
    /// window's Record): the emulation then runs through the debugger.
    recording_history: bool,
    /// The instruction about to be executed, recorded once it has been.
    fetched: Option<(u16, [u8; 4])>,
    /// The last complete frame, for `GET /api/screenshot`.
    last_frame: Option<image::RgbImage>,
    /// The beam now and after the previous instruction.
    beam: Option<Beam>,
    previous_beam: Option<Beam>,
    raster_breakpoints: Vec<RasterBreakpoint>,
    counters: std::collections::BTreeMap<String, NopCounter>,
    /// T-state counter after the previous instruction, for the counters.
    counted_t_states: usize,
    basic_breakpoints: std::collections::BTreeSet<u16>,
    /// A BASIC step (until the next statement or line) or run-to.
    basic_target: Option<BasicTarget>,
    pending_basic: Option<PendingBasic>,
    /// Names of addresses, for the breakpoints and the history.
    symbols: crate::symbols::SymbolTable,
    /// Run until the CPU reaches this address (step over, run to), then pause.
    run_to: Option<u16>,
    /// The beam kept is from before the debugger was idle (see `forget_beam`).
    beam_stale: bool,
    /// A typed key is held this long, then released this long (T-states).
    key_press_t_states: usize,
    key_gap_t_states: usize,
    /// Run until the beam passes this position, once (Debug > Run to > Beam position).
    run_to_beam: Option<RasterBreakpoint>,
    /// Step out: pause once the stack is above this SP (the routine returned).
    step_out_sp: Option<u16>,
    /// Pause at the end of the frame.
    step_frame: bool,
    /// The HTTP server, when the API is served.
    #[cfg(not(target_arch = "wasm32"))]
    listener: Option<http::Listener>,
}

/// AMSpiriT's default depth of `GET /api/history`.
const DEFAULT_HISTORY_SIZE: usize = 20;

/// One SSE client: the events waiting to be written to it.
struct SseClient {
    topics: Option<Vec<String>>,
    queue: Arc<(Mutex<SseQueue>, Condvar)>,
}

#[derive(Default)]
struct SseQueue {
    events: VecDeque<String>,
    closed: bool,
}

/// The Server-Sent Events clients, shared by the HTTP thread (which adds
/// them) and the emulation thread (which publishes).
#[derive(Clone, Default)]
pub struct EventHub {
    clients: Arc<Mutex<Vec<SseClient>>>,
}

/// AMSpiriT's limits: 8 clients, 32 queued events each (oldest dropped).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the HTTP server is the desktop's
const MAX_SSE_CLIENTS: usize = 8;
const MAX_SSE_QUEUE: usize = 32;
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the HTTP server is the desktop's
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);
/// `frame` events are sent every 10 frames (~5Hz).
const FRAME_EVENT_EVERY: u64 = 10;

impl EventHub {
    fn has_clients(&self) -> bool {
        !self.clients.lock().unwrap().is_empty()
    }

    fn publish<T: serde::Serialize>(&self, topic: &str, data: &T) {
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|client| !client.queue.0.lock().unwrap().closed);
        let mut message = None;
        for client in clients.iter() {
            if client
                .topics
                .as_ref()
                .is_some_and(|topics| !topics.iter().any(|t| t == topic))
            {
                continue;
            }
            let message = message.get_or_insert_with(|| {
                let data = serde_json::to_string(data).expect("the events are JSON");
                format!("event: {topic}\ndata: {data}\n\n")
            });
            let (queue, ready) = &*client.queue;
            let mut queue = queue.lock().unwrap();
            queue.events.push_back(message.clone());
            if queue.events.len() > MAX_SSE_QUEUE {
                queue.events.pop_front();
            }
            ready.notify_one();
        }
    }

    fn subscribe(&self, topics: Option<Vec<String>>) -> Option<Arc<(Mutex<SseQueue>, Condvar)>> {
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|client| !client.queue.0.lock().unwrap().closed);
        if clients.len() >= MAX_SSE_CLIENTS {
            return None;
        }
        let queue = Arc::new((Mutex::new(SseQueue::default()), Condvar::new()));
        clients.push(SseClient {
            topics,
            queue: Arc::clone(&queue),
        });
        Some(queue)
    }
}

impl DebugServer {
    /// Listen on `127.0.0.1:port` (loopback only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start(port: u16) -> Result<Self, String> {
        let mut debug_server = Self::offline();
        debug_server.listen(port)?;
        Ok(debug_server)
    }

    /// The debugger without a server: breakpoints, stepping, counters and history for the
    /// window's menus (see [`crate::commands`]); [`Self::listen`] serves it later.
    pub fn offline() -> Self {
        Self::with_requests(mpsc::channel().1)
    }

    /// Serve the API on `127.0.0.1:port`, in place of the requests served so far.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn listen(&mut self, port: u16) -> Result<(), String> {
        self.stop_listening();
        let (sender, requests) = mpsc::channel();
        self.listener = Some(http::listen(port, sender, self.events.clone())?);
        self.requests = requests;
        Ok(())
    }

    /// Stop serving the API (the debugger stays, breakpoints and all).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn stop_listening(&mut self) {
        // dropped: stopped
        self.listener = None;
        self.requests = mpsc::channel().1;
    }

    /// The port the API is served on, if it is.
    pub fn listening_port(&self) -> Option<u16> {
        #[cfg(not(target_arch = "wasm32"))]
        return self.listener.as_ref().map(|listener| listener.port);
        #[cfg(target_arch = "wasm32")]
        None
    }

    /// A server fed by `requests` instead of a socket (tests, embedding).
    pub fn with_requests(requests: Receiver<(ApiRequest, Sender<ApiResponse>)>) -> Self {
        Self {
            requests,
            paused: false,
            cmd_seq: 0,
            applied_cmd_seq: 0,
            unapplied: false,
            stepping: false,
            ram_seq: 0,
            exec_breakpoints: HashSet::new(),
            advanced: Vec::new(),
            events: EventHub::default(),
            local_events: None,
            reported_paused: false,
            frames: 0,
            fps: 0.0,
            fps_window: (Instant::now(), 0),
            typing: VecDeque::new(),
            typed_pressed: None,
            typing_next_at: 0,
            last_stop: None,
            quit: false,
            history: VecDeque::new(),
            history_size: DEFAULT_HISTORY_SIZE,
            recording_history: false,
            fetched: None,
            last_frame: None,
            beam: None,
            previous_beam: None,
            raster_breakpoints: Vec::new(),
            counters: Default::default(),
            counted_t_states: 0,
            basic_breakpoints: Default::default(),
            basic_target: None,
            pending_basic: None,
            symbols: Default::default(),
            run_to: None,
            run_to_beam: None,
            key_press_t_states: KEY_T_STATES,
            key_gap_t_states: KEY_T_STATES,
            beam_stale: false,
            step_out_sp: None,
            step_frame: false,
            #[cfg(not(target_arch = "wasm32"))]
            listener: None,
        }
    }

    /// Whether the emulation must stay stopped.
    /// The events published since the previous call, as `/api/events` streams them (`event:
    /// NAME\ndata: JSON\n\n` each), for a client in the same process: the web page, which has no
    /// HTTP server. The first call subscribes (nothing before it is kept); at most 32 are kept
    /// between two calls, the oldest dropped, as for an HTTP client.
    pub fn local_events(&mut self) -> Vec<String> {
        let events = self.events.clone();
        let queue = self.local_events.get_or_insert_with(|| {
            events
                .subscribe(None)
                .unwrap_or_else(|| Arc::new((Mutex::new(SseQueue::default()), Condvar::new())))
        });
        let mut queue = queue.0.lock().unwrap();
        queue.events.drain(..).collect()
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Pause or resume from the emulator's side (the window's F8): the clients get the `pause`
    /// event, as for a breakpoint.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if paused {
            self.cancel_run_targets();
        }
        self.report_pause();
    }

    /// A stop (a breakpoint, a pause) ends a step over / out, a run to an address or a beam
    /// position, a step of a frame: a later Run runs.
    fn cancel_run_targets(&mut self) {
        self.run_to = None;
        self.run_to_beam = None;
        self.step_out_sp = None;
        self.step_frame = false;
    }

    /// Whether `POST /api/quit` asked the emulator to leave.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Hand the last complete frame over, for `GET /api/screenshot`.
    pub fn set_frame(&mut self, frame: &image::RgbImage) {
        match &mut self.last_frame {
            Some(last) if last.dimensions() == frame.dimensions() => last.copy_from_slice(frame),
            last => *last = Some(frame.clone()),
        }
    }

    /// Where the monitor's beam is: to call before
    /// [`Self::after_instruction`] for `/api/beam` and the raster
    /// breakpoints.
    pub fn set_beam(&mut self, beam: Beam) {
        self.previous_beam = self.beam.replace(beam);
        if std::mem::take(&mut self.beam_stale) {
            self.previous_beam = None;
        }
    }

    /// The breakpoint the CPU stopped on since the last call, if any.
    pub fn take_stop(&mut self) -> Option<u16> {
        self.last_stop.take()
    }

    /// Answer every pending request.
    pub fn poll(&mut self, motherboard: &mut Motherboard) {
        self.poll_with(motherboard, None);
    }

    /// [`Self::poll`], with the monitor drawing the picture: a screenshot
    /// can then show the frame being drawn (`live=1`) and the whole frame
    /// (`crop=0`, see [`Monitor::keep_full_frames`]).
    pub fn poll_with(&mut self, motherboard: &mut Motherboard, monitor: Option<&Monitor>) {
        while let Ok((request, reply)) = self.requests.try_recv() {
            let response = self.handle_with(&request, motherboard, monitor);
            let _ = reply.send(response);
        }
        // Paused: nothing will run after the commands, they are applied.
        if self.paused && !self.stepping {
            self.mark_applied();
        }
        self.report_pause();
    }

    fn report_pause(&mut self) {
        // A step resumes for one instruction only: not a transition.
        if self.paused != self.reported_paused && !self.stepping {
            self.reported_paused = self.paused;
            self.events.publish(
                "pause",
                &PauseEvent {
                    paused: self.paused,
                    cmd_seq: self.cmd_seq,
                },
            );
        }
    }

    /// Whether some breakpoint needs the bus accesses.
    fn watches_bus(&self) -> bool {
        self.advanced.iter().any(|b| b.kind != BreakpointKind::Exec)
    }

    /// To call after each instruction the emulation executes.
    pub fn after_instruction(&mut self, motherboard: &mut Motherboard) {
        self.mark_applied();
        let stepped = std::mem::take(&mut self.stepping);
        self.count_nops(motherboard);
        if let Some(executed) = self.fetched.take() {
            if self.history.len() >= self.history_size {
                self.history.pop_front();
            }
            self.history.push_back(executed);
        }
        // Data accesses of the instruction just executed.
        for access in motherboard.take_bus_accesses() {
            for index in 0..self.advanced.len() {
                if self.advanced[index].hit_by_access(&access) {
                    self.hit(index, Some(access), motherboard);
                }
            }
        }
        // The instruction about to be executed.
        let pc = motherboard.pc();
        if !self.exec_breakpoints.is_empty()
            && self
                .exec_breakpoints
                .contains(&motherboard.physical_address(pc))
        {
            self.stop(motherboard, StopInfo::of("exec"));
        }
        for index in 0..self.advanced.len() {
            if self.advanced[index].hit_by_execution(pc) {
                self.hit(index, None, motherboard);
            }
        }
        if (!self.basic_breakpoints.is_empty() || self.basic_target.is_some())
            && basic_program::at_statement(motherboard)
        {
            self.basic_statement(motherboard);
        }
        if self
            .pending_basic
            .as_ref()
            .is_some_and(|p| motherboard.t_state_counter().value() >= p.at_t_states)
        {
            let pending = self.pending_basic.take().unwrap();
            // asked before the machine was ready: its error has no request left to answer
            if let Err(error) = self.load_basic(&pending.source, pending.run, motherboard) {
                self.events
                    .publish("basic_error", &BasicErrorEvent { error });
            }
        }
        if let (Some(before), Some(after)) = (self.previous_beam.take(), self.beam) {
            if let Some(index) = self
                .raster_breakpoints
                .iter()
                .position(|brk| brk.passed(before, after))
            {
                let info = StopInfo {
                    index: Some(index as u64),
                    name: self.raster_breakpoints[index].name.clone(),
                    ..StopInfo::of("raster")
                };
                self.stop(motherboard, info);
            }
        }
        // the end of a single step, told unless something else stopped it first
        if stepped && !self.paused {
            self.stop(motherboard, StopInfo::of("step"));
        }
        if stepped {
            self.paused = true;
        }
        if self.run_to == Some(pc) {
            self.stop(motherboard, StopInfo::of("run_to"));
        }
        // the routine returned: the stack above where it was (SP may wrap past &FFFF)
        if self
            .step_out_sp
            .is_some_and(|sp| (motherboard.sp().wrapping_sub(sp) as i16) > 0 && !stepped)
        {
            self.stop(motherboard, StopInfo::of("step_out"));
        }
        if let (Some(target), Some(before), Some(after)) =
            (self.run_to_beam.as_ref(), self.previous_beam, self.beam)
        {
            if target.passed(before, after) {
                self.stop(motherboard, StopInfo::of("run_to_beam"));
            }
        }
        self.fetched = Some((pc, instruction_bytes(motherboard, pc)));
        self.report_pause();
        self.type_due_keys(motherboard);
    }

    /// An advanced breakpoint hit, if its condition holds: counted, and
    /// past its pass count, acting on its counters then stopping, reporting
    /// (watch) or nothing more (count).
    fn hit(&mut self, index: usize, access: Option<BusAccess>, motherboard: &Motherboard) {
        let brk = &mut self.advanced[index];
        if let Some(condition) = &brk.condition {
            if !condition.holds(&MachineContext {
                motherboard,
                access,
                beam: self.beam,
            }) {
                return;
            }
        }
        brk.hits += 1;
        if brk.hits <= brk.after {
            return;
        }
        let brk = brk.clone();
        for (action, name) in &brk.counters {
            self.act_on_counter(*action, name);
        }
        let access = access.map(|access| EventAccess {
            access_type: match access.kind {
                BusAccessKind::MemoryRead | BusAccessKind::IoRead => "read",
                BusAccessKind::MemoryWrite | BusAccessKind::IoWrite => "write",
            }
            .to_owned(),
            addr: hex16(access.address),
            value: access.value,
        });
        match brk.run {
            RunMode::Stop => self.stop(
                motherboard,
                StopInfo {
                    index: Some(index as u64),
                    name: brk.name.clone(),
                    hits: Some(brk.hits),
                    access,
                    ..StopInfo::of(kind_name(brk.kind))
                },
            ),
            RunMode::Watch => {
                let event = WatchEvent {
                    registers: registers_event(motherboard),
                    kind: kind_name(brk.kind).to_owned(),
                    index: index as u64,
                    hits: brk.hits,
                    name: brk.name.clone(),
                    access,
                    counters: self.counters_map(),
                };
                self.events.publish("watch", &event);
            }
            RunMode::Count => {}
        }
    }

    /// A breakpoint's counter action (the instruction just executed is
    /// already counted).
    fn act_on_counter(&mut self, action: CounterAction, name: &str) {
        let counter = match (action, self.counters.get_mut(name)) {
            (_, Some(counter)) => counter,
            (CounterAction::Stop, None) => return,
            (_, None) => self.counters.entry(name.to_owned()).or_insert(NopCounter {
                t_states: 0,
                running: false,
            }),
        };
        match action {
            CounterAction::Start => counter.running = true,
            CounterAction::Stop => counter.running = false,
            CounterAction::Reset => counter.t_states = 0,
        }
    }

    fn stop(&mut self, motherboard: &Motherboard, info: StopInfo) {
        self.paused = true;
        self.cancel_run_targets();
        self.last_stop = Some(motherboard.pc());
        let event = StopEvent {
            registers: registers_event(motherboard),
            reason: stop_reason(info.kind),
            kind: info.kind.to_owned(),
            cmd_seq: self.cmd_seq,
            ticks: (motherboard.t_state_counter().value() / T_STATES_PER_NOP) as u64,
            beam: self.beam.map(|beam| beam.event_beam()),
            index: info.index,
            name: info.name,
            hits: info.hits,
            access: info.access,
            counters: self.counters_map(),
        };
        self.events.publish("z80_bp", &event);
    }

    /// At the start of a BASIC statement: stop on a line breakpoint, at the
    /// end of a step, at a run-to target.
    fn basic_statement(&mut self, motherboard: &Motherboard) {
        let state = BasicState::read(motherboard);
        let Some(line) = state.line else {
            return; // direct mode
        };
        let reason = match self.basic_target {
            Some(BasicTarget::Statement) => Some(BasicStopReason::Step),
            Some(BasicTarget::LineAfter(from)) if from != Some(line) || state.first_statement => {
                Some(BasicStopReason::Step)
            }
            Some(BasicTarget::Line(target)) if target == line && state.first_statement => {
                Some(BasicStopReason::RunTo)
            }
            Some(BasicTarget::Address(target)) if target == state.stmt_addr => {
                Some(BasicStopReason::RunTo)
            }
            _ if state.first_statement && self.basic_breakpoints.contains(&line) => {
                Some(BasicStopReason::Breakpoint)
            }
            _ => None,
        };
        if let Some(reason) = reason {
            self.basic_target = None;
            self.paused = true;
            self.cancel_run_targets();
            self.last_stop = Some(motherboard.pc());
            let event = BasicStopEvent {
                line,
                addr: hex16(state.stmt_addr),
                reason,
                counters: self.counters_map(),
            };
            self.events.publish("basic_bp", &event);
        }
    }

    /// Add the T-states of the instruction just executed to the running
    /// counters (a reset of the machine restarts its T-state counter).
    fn count_nops(&mut self, motherboard: &Motherboard) {
        let now = motherboard.t_state_counter().value();
        let elapsed = now.checked_sub(self.counted_t_states).unwrap_or(now) as u64;
        self.counted_t_states = now;
        for counter in self.counters.values_mut().filter(|c| c.running) {
            counter.t_states += elapsed;
        }
    }

    /// The counters' NOPs, by name, for the events; `None` without counters.
    fn counters_map(&self) -> Option<std::collections::BTreeMap<String, u64>> {
        (!self.counters.is_empty()).then(|| {
            self.counters
                .iter()
                .map(|(name, counter)| (name.clone(), counter.t_states / T_STATES_PER_NOP as u64))
                .collect()
        })
    }

    /// To call once per completed video frame.
    pub fn frame_completed(&mut self, motherboard: &Motherboard) {
        self.frames += 1;
        if std::mem::take(&mut self.step_frame) {
            self.paused = true;
            self.report_pause();
        }
        if self.frames.is_multiple_of(FRAME_EVENT_EVERY) && self.events.has_clients() {
            let event = FrameEvent {
                registers: registers_event(motherboard),
                fps: self.fps,
                frame: self.frames,
                paused: self.paused,
            };
            self.events.publish("frame", &event);
        }
        let elapsed = self.fps_window.0.elapsed();
        if elapsed >= Duration::from_secs(1) {
            self.fps = (self.frames - self.fps_window.1) as f64 / elapsed.as_secs_f64();
            self.fps_window = (Instant::now(), self.frames);
        }
    }

    fn mark_applied(&mut self) {
        if self.unapplied {
            self.applied_cmd_seq = self.cmd_seq;
            self.unapplied = false;
        }
    }

    fn command(&mut self) -> u64 {
        self.cmd_seq += 1;
        self.unapplied = true;
        self.cmd_seq
    }

    // ------------------------------------------------- run control (commands)

    /// Run one instruction, then pause (`POST /api/step`, Debug > Step-in).
    pub fn step(&mut self) {
        self.paused = false;
        self.stepping = true;
        self.command();
    }

    /// Run one instruction, a `CALL`, `RST` or a `DJNZ` / block loop as a whole: a temporary
    /// breakpoint after it (Debug > Step over).
    pub fn step_over(&mut self, motherboard: &Motherboard) {
        let pc = motherboard.pc();
        let next = motherboard.disassemble_next_instruction();
        let opcode = motherboard.read_byte(pc);
        let ed = motherboard.read_byte(pc.wrapping_add(1));
        // the firmware's RST &08 (LOW JUMP), &10 (SIDE CALL) and &18 (FAR CALL) are followed by
        // a 2-byte address and return after it; RST &00 (reset) and &28 (FIRM JUMP) don't return
        let rst = (opcode & 0b1100_0111 == 0b1100_0111).then_some(opcode & 0x38);
        let after = match rst {
            Some(0x08 | 0x10 | 0x18) => Some(pc.wrapping_add(3)),
            Some(0x00 | 0x28) => None,
            Some(_) => Some(pc.wrapping_add(1)),
            None => {
                let runs_whole = opcode == 0xCD                     // CALL nn
                    || opcode & 0b1100_0111 == 0b1100_0100          // CALL cc,nn
                    || opcode == 0x10                               // DJNZ
                    || (opcode == 0xED && matches!(ed, 0xB0..=0xB3 | 0xB8..=0xBB)); // LDIR...
                runs_whole.then(|| pc.wrapping_add(next.len() as u16))
            }
        };
        match after {
            Some(address) => self.run_to(address),
            None => self.step(),
        }
    }

    /// Run until the current routine returns: the stack goes above where it is now (Debug >
    /// Step-out).
    pub fn step_out(&mut self, motherboard: &Motherboard) {
        self.step_out_sp = Some(motherboard.sp());
        self.paused = false;
        self.command();
    }

    /// Run until the CPU reaches `address`, then pause.
    pub fn run_to(&mut self, address: u16) {
        self.run_to = Some(address);
        self.paused = false;
        self.command();
    }

    /// Run until the beam reaches `line` (of the frame, as the monitor counts them) and NOP
    /// `nop` of it, once, then pause.
    pub fn run_to_beam(&mut self, line: usize, nop: usize) {
        self.run_to_beam = Some(RasterBreakpoint {
            line: Some(line),
            x: nop.min(NOPS_PER_LINE - 1) * BEAM_TICKS_PER_NOP,
            name: None,
        });
        self.forget_beam();
        self.paused = false;
        self.command();
    }

    /// The beam seen long ago is not the one before the next instruction (the debugger was
    /// idle meanwhile): no raster stop compares against it.
    fn forget_beam(&mut self) {
        // the current one stays: positions in the picture are cropped against it; it is not
        // the one before the next instruction
        self.previous_beam = None;
        self.beam_stale = true;
    }

    /// Run to the end of the frame, then pause.
    pub fn step_frame(&mut self) {
        self.step_frame = true;
        self.paused = false;
        self.command();
    }

    /// The number of instructions the history keeps (resizing empties it, as in AMSpiriT).
    pub fn set_history_size(&mut self, size: usize) {
        self.history_size = size.clamp(1, 100_000);
        self.history.clear();
    }

    pub fn history_size(&self) -> usize {
        self.history_size
    }

    /// The executed instructions, oldest first: (PC, the bytes as fetched).
    pub fn history(&self) -> impl Iterator<Item = &(u16, [u8; 4])> {
        self.history.iter()
    }

    /// Tell the debugger's clients the machine was reset.
    pub fn reset_done(&mut self, hard: bool) {
        self.events.publish("reset", &ResetEvent { hard });
        self.command();
    }

    /// The emulator is asked to leave (`POST /api/quit`, File > Exit).
    pub fn request_quit(&mut self) {
        self.quit = true;
        self.command();
    }

    /// The names known for addresses.
    pub fn symbols(&self) -> &crate::symbols::SymbolTable {
        &self.symbols
    }

    /// Every breakpoint gone: execution, memory / I/O, raster and BASIC ones.
    pub fn clear_breakpoints(&mut self, motherboard: &mut Motherboard) {
        self.exec_breakpoints.clear();
        self.advanced.clear();
        self.raster_breakpoints.clear();
        self.basic_breakpoints.clear();
        motherboard.watch_bus(false);
        self.command();
    }

    /// Whether the debugger has something to do after each instruction (a server, a
    /// breakpoint, a step, keys to type, a counter): the window calls
    /// [`Self::after_instruction`] only then, the emulation running at full speed otherwise.
    pub fn is_active(&self) -> bool {
        // served, over HTTP or to the page
        self.listening_port().is_some()
            || self.local_events.is_some()
            || self.has_breakpoints()
            || self.stepping
            || self.run_to.is_some()
            || self.run_to_beam.is_some()
            || self.step_out_sp.is_some()
            || self.step_frame
            || self.is_typing()
            || self.pending_basic.is_some()
            || self.basic_target.is_some()
            || !self.counters.is_empty()
            || self.recording_history
    }

    /// Record the instructions run while nothing else makes the debugger active.
    pub fn set_recording_history(&mut self, record: bool) {
        self.recording_history = record;
    }

    pub fn records_history(&self) -> bool {
        self.recording_history
    }

    /// Whether some breakpoint is set.
    pub fn has_breakpoints(&self) -> bool {
        !self.exec_breakpoints.is_empty()
            || !self.advanced.is_empty()
            || !self.raster_breakpoints.is_empty()
            || !self.basic_breakpoints.is_empty()
    }

    /// Type `text` on the CPC's keyboard, a key every 2 frames (`POST /api/keytype`).
    pub fn type_text(
        &mut self,
        text: &str,
        layout: crate::keyboard::KeyboardLayout,
    ) -> Result<(), String> {
        let mut keys = Vec::new();
        for c in text.chars() {
            let c = if c == '\r' { '\n' } else { c };
            keys.push(
                layout
                    .keys_of_char(c)
                    .ok_or_else(|| format!("cannot type {c:?}"))?,
            );
        }
        self.typing.extend(keys);
        self.command();
        Ok(())
    }

    /// Type the keys of `keys`, one combination after the other (see `csl_runner::keys_of_text`).
    pub fn type_keys(&mut self, keys: Vec<Vec<CpcKey>>) {
        self.typing.extend(keys);
        self.command();
    }

    /// How long a typed key is held and the pause after it, in microseconds (2 frames each by
    /// default, as AMSpiriT types).
    pub fn set_key_delays(&mut self, press_us: u64, after_key_us: u64) {
        self.key_press_t_states = (press_us as usize * T_STATES_PER_NOP).max(1);
        self.key_gap_t_states = (after_key_us as usize * T_STATES_PER_NOP).max(1);
    }

    /// Whether typed keys are still to come.
    pub fn is_typing(&self) -> bool {
        !self.typing.is_empty() || self.typed_pressed.is_some()
    }

    /// The counters' names and NOPs, and whether each runs.
    pub fn counters(&self) -> Vec<(String, u64, bool)> {
        self.counters
            .iter()
            .map(|(name, c)| {
                (
                    name.clone(),
                    c.t_states / T_STATES_PER_NOP as u64,
                    c.running,
                )
            })
            .collect()
    }

    /// What a `GET` route of the API shows, for the debugger's windows (the same data): `z80`,
    /// `crtc`, `ga`, `psg`, `fdc`, `memmap`, `breakpoints`, `symbols`, `counters`, `raster_bp`,
    /// `beam`, `state`, `basic_state`.
    pub fn inspect(&self, what: &str, motherboard: &Motherboard) -> Value {
        match what {
            "z80" => z80(motherboard),
            "crtc" => self.crtc(motherboard),
            "ga" => ga(motherboard),
            "psg" => psg(motherboard),
            "fdc" => fdc(motherboard),
            "asic" => asic(motherboard),
            "memmap" => memmap(motherboard),
            "breakpoints" => to_json(&self.list_breakpoints()),
            "symbols" => to_json(&self.list_symbols()),
            "counters" => to_json(&Counters {
                counters: self.list_counters(),
            }),
            "raster_bp" => to_json(&RasterBreakpoints {
                breakpoints: self.list_raster_breakpoints(),
            }),
            "beam" => self.beam.map_or(Value::Null, beam_json),
            "state" => self.state(motherboard),
            "basic_state" => basic_state(motherboard),
            _ => Value::Null,
        }
    }

    /// The beam, while the debugger follows it (paused, stepping).
    pub fn beam(&self) -> Option<Beam> {
        self.beam
    }

    /// Answer an API call made from inside the emulator (the menus): the same code as the
    /// HTTP route, its JSON reply, or its error.
    pub fn api(
        &mut self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: &str,
        motherboard: &mut Motherboard,
    ) -> Result<Value, String> {
        let request = ApiRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            query: query
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
        };
        let response = self.handle(&request, motherboard);
        let text = String::from_utf8_lossy(&response.body).into_owned();
        let value: Value = if response.content_type == "application/json" {
            serde_json::from_str(&text).unwrap_or(Value::Null)
        } else {
            Value::String(text.clone())
        };
        if response.status == 200 {
            Ok(value)
        } else {
            Err(value
                .get("error")
                .and_then(Value::as_str)
                .map_or(text, str::to_owned))
        }
    }

    /// Route `request`.
    pub fn handle(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResponse {
        self.handle_with(request, motherboard, None)
    }

    /// [`Self::handle`], with the monitor (see [`Self::poll_with`]).
    pub fn handle_with(
        &mut self,
        request: &ApiRequest,
        motherboard: &mut Motherboard,
        monitor: Option<&Monitor>,
    ) -> ApiResponse {
        let Some(route) = ROUTES.iter().find(|route| route.path == request.path) else {
            return ApiResponse::json(404, json!({ "error": "not found", "path": request.path }));
        };
        if !route.methods.contains(&request.method.as_str()) {
            let mut response = ApiResponse::json(
                405,
                json!({ "error": "method not allowed", "allow": route.methods }),
            );
            response
                .headers
                .push(("Allow".to_owned(), route.methods.join(", ")));
            return response;
        }
        let result = match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/ping") => Ok(reply(&self.ping(motherboard))),
            ("GET", "/api/state") => Ok(reply(&self.state_typed(motherboard))),
            ("GET", "/api/z80") => Ok(reply(&z80_registers(motherboard))),
            ("POST", "/api/z80") => self.set_z80(request, motherboard),
            ("GET", "/api/ram") => read_ram(request, motherboard),
            ("POST", "/api/ram") => self.write_ram(request, motherboard),
            ("POST", "/api/exec") => self.exec(request, motherboard),
            ("POST", "/api/step") => self.step_request(request, motherboard),
            ("POST", "/api/run_to") => self.run_to_request(request, motherboard),
            ("GET", "/api/config") => Ok(reply(&self.get_config(motherboard))),
            ("POST", "/api/config") => self.config(request, motherboard),
            ("POST", "/api/z80_bp") => self.set_breakpoints(request, motherboard),
            ("DELETE", "/api/z80_bp") => self.remove_exec_breakpoints(request, motherboard),
            ("GET", "/api/breakpoints") => Ok(reply(&self.list_breakpoints())),
            ("GET", "/api/symbols") => Ok(reply(&self.list_symbols())),
            ("POST", "/api/breakpoints") => self.add_breakpoint(request, motherboard),
            ("DELETE", "/api/breakpoints") => self.remove_breakpoints(request, motherboard),
            // Answered by the HTTP thread (see `serve`): only reached
            // through `handle` directly.
            ("GET", "/api/events") => Err(ApiError::new("events are streamed by the HTTP server")),
            ("POST", "/api/keytype") => self.keytype(request, motherboard.keyboard_layout()),
            ("GET", "/api/doc") => doc(request),
            ("POST", "/api/quit") => self
                .run_command("exit", None, motherboard)
                .map(|()| reply(&CommandAck::new(self.cmd_seq))),
            ("GET", "/api/history") => self.history_list(request, motherboard),
            ("GET", "/api/screenshot") => self.screenshot(request, monitor),
            ("POST", "/api/media") => self.media(request, motherboard),
            ("GET", "/api/snapshot") => crate::snapshot::save_sna_bytes(motherboard)
                .map_err(ApiError::new)
                .map(|body| ApiResponse {
                    status: 200,
                    content_type: "application/octet-stream",
                    body,
                    headers: vec![(
                        "Content-Disposition".into(),
                        "attachment; filename=\"bemu.sna\"".into(),
                    )],
                }),
            ("GET", "/api/memmap") => memmap_request(request, motherboard),
            ("GET", "/api/crtc") => Ok(reply(&self.crtc_state(motherboard))),
            ("GET", "/api/ga") => Ok(reply(&ga_state(motherboard))),
            ("GET", "/api/psg") => Ok(reply(&psg_state(motherboard))),
            ("GET", "/api/fdc") => Ok(reply(&fdc_state(motherboard))),
            ("GET", "/api/asic") => Ok(reply(&crate::state::asic(motherboard))),
            ("POST", "/api/drive") => self.drive_command(request, motherboard),
            ("GET", "/api/beam") => self.beam_position(request),
            ("GET", "/api/raster_bp") => Ok(reply(&RasterBreakpoints {
                breakpoints: self.list_raster_breakpoints(),
            })),
            ("POST", "/api/raster_bp") => self.set_raster_breakpoint(request),
            ("GET", "/api/counters") => Ok(reply(&Counters {
                counters: self.list_counters(),
            })),
            ("POST", "/api/counters") => self.counter_command(request, motherboard),
            ("POST", "/api/basic") => self.basic_load_request(request, motherboard),
            ("GET", "/api/basic_state") => Ok(reply(&basic_state_typed(motherboard))),
            ("GET", "/api/basic_export") => Ok(ApiResponse {
                status: 200,
                content_type: "text/plain; charset=utf-8",
                body: basic_program::program(motherboard)
                    .map(|p| p.to_string())
                    .unwrap_or_default()
                    .into_bytes(),
                headers: Vec::new(),
            }),
            ("GET", "/api/basic_listing") => basic_listing(motherboard),
            ("POST", "/api/basic_step") => self.basic_step(request, motherboard),
            ("GET", "/api/basic_bp") => Ok(reply(&BasicBreakpoints {
                lines: self.basic_breakpoints.iter().copied().collect(),
            })),
            ("POST", "/api/basic_bp") => self.set_basic_breakpoints(request),
            ("POST", "/api/basic_runto") => self.basic_run_to(request),
            ("DELETE", "/api/counters") => self.delete_counters(request),
            _ => unreachable!("{} {}", request.method, request.path),
        };
        result.unwrap_or_else(ApiError::response)
    }

    // ------------------------------------------------------------ state

    fn emu(&self, motherboard: &Motherboard) -> EmuState {
        EmuState {
            fps: self.fps,
            frames: self.frames,
            ticks: (motherboard.t_state_counter().value() / T_STATES_PER_NOP) as u64,
            paused: self.paused,
            cpc_model: motherboard.model().name().to_string(),
            crtc_type: motherboard.crtc().crtc_type() as u8,
            autotyping: !self.typing.is_empty() || self.typed_pressed.is_some(),
            autotype_remaining: self.typing.len() as u64,
            ram_apply_seq: self.ram_seq,
            applied_cmd_seq: self.applied_cmd_seq,
        }
    }

    fn state(&self, motherboard: &Motherboard) -> Value {
        to_json(&self.state_typed(motherboard))
    }

    fn state_typed(&self, motherboard: &Motherboard) -> State {
        State {
            z80: z80_registers(motherboard),
            ga: ga_state(motherboard),
            crtc: self.crtc_state(motherboard),
            psg: psg_state(motherboard),
            fdc: fdc_state(motherboard),
            beam: self.beam.map(|beam| beam.state(false)),
            emu: self.emu(motherboard),
        }
    }

    fn crtc(&self, motherboard: &Motherboard) -> Value {
        to_json(&self.crtc_state(motherboard))
    }

    fn crtc_state(&self, motherboard: &Motherboard) -> Crtc {
        let crtc = motherboard.crtc();
        Crtc {
            crtc_type: crtc.crtc_type() as u8,
            regs: (0..14).map(|r| crtc.do_read_register(r)).collect(),
            selected_reg: crtc.selected_register(),
            // Extension: the counters (C0, C4, C9, VSYNC, R5 adjust) and
            // the address the CRTC outputs.
            hcc: crtc.hcc,
            vcc: crtc.vcc.value(),
            vlc: crtc.vlc.value(),
            vsc: crtc.vsc,
            vtac: crtc.vtac,
            ma: crtc.vma(),
            rasterline: self.beam.map(|beam| beam.line as u64),
        }
    }

    // ------------------------------------------------------------- beam

    fn beam_position(&self, request: &ApiRequest) -> ApiResult {
        let query: BeamQuery = query_as(request)?;
        let beam = self.beam.ok_or_else(|| {
            ApiError::new("no beam position: the emulation is not driving a monitor")
        })?;
        Ok(reply(&BeamPosition {
            beam: beam.state(flag(query.crop, false)),
            nops_per_line: NOPS_PER_LINE as u64,
            nops_per_frame: (NOPS_PER_LINE * LINES_PER_FRAME) as u64,
        }))
    }

    fn list_raster_breakpoints(&self) -> Vec<api::RasterBreakpoint> {
        self.raster_breakpoints
            .iter()
            .enumerate()
            .map(|(i, b)| b.typed(i))
            .collect()
    }

    /// `POST /api/raster_bp`: AMSpiriT's `x`/`y` (beam ticks and lines,
    /// `crop=1` for picture coordinates), or the position in NOPs: `nop`
    /// (0-63) in `line` (0-311), `frame_nop` (0-19967); `nop` alone stops
    /// in every line, `line` alone at its start.
    fn set_raster_breakpoint(&mut self, request: &ApiRequest) -> ApiResult {
        self.forget_beam();
        let set: SetRasterBreakpoint = query_as(request)?;
        let param = |name: &str, value: Option<Int>, max: usize| -> Result<Option<usize>, ApiError> {
            Ok(ranged(name, value, 0, max as i64)?.map(|v| v as usize))
        };
        let enable = flag(set.enable, true);
        let add = flag(set.add, false);
        let crop = flag(set.crop, false);
        let x = param("x", set.x, 0xffff)?;
        let y = param("y", set.y, 0xffff)?;
        let nop = param("nop", set.nop, NOPS_PER_LINE - 1)?;
        let line = param("line", set.line, 0xffff)?;
        let frame_nop = param("frame_nop", set.frame_nop, NOPS_PER_LINE * LINES_PER_FRAME - 1)?;
        let name = set.name.filter(|n| !n.is_empty());

        let breakpoint = if let Some(frame_nop) = frame_nop {
            Some(RasterBreakpoint {
                line: Some(frame_nop / NOPS_PER_LINE),
                x: frame_nop % NOPS_PER_LINE * BEAM_TICKS_PER_NOP,
                name,
            })
        } else if nop.is_some() || line.is_some() {
            Some(RasterBreakpoint {
                line,
                x: nop.unwrap_or(0) * BEAM_TICKS_PER_NOP,
                name,
            })
        } else if x.is_some() || y.is_some() {
            let (dx, dy) = match (crop, self.beam) {
                (true, Some(beam)) => (beam.crop_x, beam.crop_y as isize),
                (true, None) => {
                    return Err(ApiError::field(
                        "crop",
                        "no beam position to crop against yet",
                    ))
                }
                (false, _) => (0, 0),
            };
            let x = (x.unwrap_or(0) as isize + dx).max(0) as usize;
            Some(RasterBreakpoint {
                line: y.map(|y| (y as isize + dy).max(0) as usize),
                x,
                name,
            })
        } else {
            None
        };
        if !add || !enable {
            self.raster_breakpoints.clear();
        }
        match (enable, breakpoint) {
            (true, Some(breakpoint)) => self.raster_breakpoints.push(breakpoint),
            (true, None) if !add => {}
            (true, None) => {
                return Err(ApiError::new(
                    "add=1 needs a position: x/y, line/nop or frame_nop",
                ))
            }
            (false, _) => {}
        }
        Ok(reply(&RasterBreakpointsSet {
            ok: true,
            cmd_seq: self.command(),
            breakpoints: self.list_raster_breakpoints(),
        }))
    }

    // ------------------------------------------------------------ BASIC

    fn run_basic_to(&mut self, target: BasicTarget) -> ApiResponse {
        self.basic_target = Some(target);
        self.paused = false;
        reply(&CommandAck::new(self.command()))
    }

    fn set_basic_breakpoints(&mut self, request: &ApiRequest) -> ApiResult {
        let mut lines = std::collections::BTreeSet::new();
        for entry in request
            .text()
            .split(',')
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            let line = entry.parse::<u16>().map_err(|_| {
                ApiError::new(format!("{entry}: expected a BASIC line number (0-65535)"))
            })?;
            lines.insert(line);
        }
        self.basic_breakpoints = lines;
        Ok(reply(&BasicBreakpointsSet {
            ok: true,
            lines: self.basic_breakpoints.iter().copied().collect(),
            cmd_seq: self.command(),
        }))
    }

    /// `POST /api/basic_step`.
    fn basic_step(&mut self, request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
        let query: BasicStep = query_as(request)?;
        let target = match query.mode.unwrap_or(BasicStepMode::Statement) {
            BasicStepMode::Line => BasicTarget::LineAfter(BasicState::read(motherboard).line),
            BasicStepMode::Statement => BasicTarget::Statement,
        };
        Ok(self.run_basic_to(target))
    }

    /// `POST /api/basic_runto`.
    fn basic_run_to(&mut self, request: &ApiRequest) -> ApiResult {
        let query: BasicRunTo = query_as(request)?;
        match (
            ranged("line", query.line, 0, 0xffff)?,
            ranged("addr", query.addr, 0, 0xffff)?,
        ) {
            (Some(line), _) => Ok(self.run_basic_to(BasicTarget::Line(line as u16))),
            (None, Some(addr)) => Ok(self.run_basic_to(BasicTarget::Address(addr as u16))),
            (None, None) => Err(ApiError::new("missing ?line= or ?addr=")),
        }
    }

    /// `POST /api/basic[?reset=1][&run=1]`: after a reset, the program is
    /// loaded once the firmware booted.
    fn basic_load_request(
        &mut self,
        request: &ApiRequest,
        motherboard: &mut Motherboard,
    ) -> ApiResult {
        let query: BasicLoad = query_as(request)?;
        let (reset, run) = (flag(query.reset, false), flag(query.run, false));
        let source = request.text();
        // Checked now, to answer 400 rather than fail later.
        cpclib::basic::BasicProgram::parse(&source)
            .map_err(|e| ApiError::new(format!("BASIC inject failed: {e:?}")))?;
        if reset {
            motherboard.restart();
            self.events.publish("reset", &ResetEvent { hard: true });
            self.pending_basic = Some(PendingBasic {
                source,
                run,
                at_t_states: motherboard.t_state_counter().value() + BASIC_BOOT_T_STATES,
            });
        } else if motherboard.t_state_counter().value() < BASIC_BOOT_T_STATES {
            // Just powered on, the firmware still booting: BASIC would clear
            // the program as it starts. Loaded once it is up, like after a
            // reset (a debugger that starts the emulator injects at once).
            self.pending_basic = Some(PendingBasic {
                source,
                run,
                at_t_states: BASIC_BOOT_T_STATES,
            });
        } else {
            self.load_basic(&source, run, motherboard)
                .map_err(|e| ApiError::new(format!("BASIC inject failed: {e}")))?;
        }
        Ok(reply(&CommandAck::new(self.command())))
    }

    fn load_basic(
        &mut self,
        source: &str,
        run: bool,
        motherboard: &mut Motherboard,
    ) -> Result<usize, String> {
        let size = basic_program::load(motherboard, source)?;
        self.ram_seq += 1;
        if run {
            let layout = motherboard.keyboard_layout();
            self.typing
                .extend("RUN\n".chars().filter_map(|c| layout.keys_of_char(c)));
        }
        Ok(size)
    }

    // --------------------------------------------------------- counters

    fn list_counters(&self) -> Vec<Counter> {
        self.counters
            .iter()
            .map(|(name, counter)| Counter {
                name: name.clone(),
                nops: counter.t_states / T_STATES_PER_NOP as u64,
                running: counter.running,
            })
            .collect()
    }

    /// `POST /api/counters {"name", "action": create|reset|start|stop}`:
    /// a counter counts the NOPs the emulation runs while it is running
    /// (from its creation unless created with `"running": false`).
    fn counter_command(&mut self, request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
        let command: CounterCommand = body_as(request)?;
        let name = command
            .name
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ApiError::field("name", "missing field \"name\": the counter's name"))?;
        let running = command.running.unwrap_or(true);
        // Count from the instruction about to run.
        self.count_nops(motherboard);
        match command.action.unwrap_or(api::CounterAction::Create) {
            api::CounterAction::Create => {
                if self.counters.contains_key(&name) {
                    return Err(ApiError::field(
                        "name",
                        format!("counter \"{name}\" exists (reset or delete it)"),
                    ));
                }
                self.counters.insert(
                    name,
                    NopCounter {
                        t_states: 0,
                        running,
                    },
                );
            }
            action => {
                let counter = self
                    .counters
                    .get_mut(&name)
                    .ok_or_else(|| ApiError::field("name", format!("no counter \"{name}\"")))?;
                match action {
                    api::CounterAction::Reset => counter.t_states = 0,
                    api::CounterAction::Start => counter.running = true,
                    _ => counter.running = false,
                }
            }
        }
        Ok(reply(&CountersChanged {
            ok: true,
            counters: self.list_counters(),
        }))
    }

    fn delete_counters(&mut self, request: &ApiRequest) -> ApiResult {
        refuse_empty(request, "name", "the counter to delete (none: all)")?;
        let query: RemoveCounter = query_as(request)?;
        match query.name {
            None => self.counters.clear(),
            Some(name) => {
                if self.counters.remove(&name).is_none() {
                    return Err(ApiError::field("name", format!("no counter \"{name}\"")));
                }
            }
        }
        Ok(reply(&CountersChanged {
            ok: true,
            counters: self.list_counters(),
        }))
    }

    // --------------------------------------------------------- commands

    /// `POST /api/z80`.
    fn set_z80(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let set: SetZ80 = body_as(request)?;
        let ack = self.apply_z80(&set, motherboard)?;
        Ok(reply(&ack))
    }

    /// Set the registers `set` names; all are checked before any changes.
    fn apply_z80(&mut self, set: &SetZ80, motherboard: &mut Motherboard) -> Result<CommandAck, ApiError> {
        let check = |name: &str, value: Option<Int>, max: i64| ranged(name, value, 0, max);
        let words = [
            ("PC", set.PC, Reg16::Pc),
            ("SP", set.SP, Reg16::Sp),
            ("IX", set.IX, Reg16::Ix),
            ("IY", set.IY, Reg16::Iy),
        ];
        // the pairs, set by their halves
        let pairs = [
            ("AF", set.AF, Reg8::A, Reg8::F),
            ("BC", set.BC, Reg8::B, Reg8::C),
            ("DE", set.DE, Reg8::D, Reg8::E),
            ("HL", set.HL, Reg8::H, Reg8::L),
            ("AF2", set.AF2, Reg8::A2, Reg8::F2),
            ("BC2", set.BC2, Reg8::B2, Reg8::C2),
            ("DE2", set.DE2, Reg8::D2, Reg8::E2),
            ("HL2", set.HL2, Reg8::H2, Reg8::L2),
        ];
        let bytes = [
            ("A", set.A, Reg8::A),
            ("F", set.F, Reg8::F),
            ("B", set.B, Reg8::B),
            ("C", set.C, Reg8::C),
            ("D", set.D, Reg8::D),
            ("E", set.E, Reg8::E),
            ("H", set.H, Reg8::H),
            ("L", set.L, Reg8::L),
            ("A2", set.A2, Reg8::A2),
            ("F2", set.F2, Reg8::F2),
            ("B2", set.B2, Reg8::B2),
            ("C2", set.C2, Reg8::C2),
            ("D2", set.D2, Reg8::D2),
            ("E2", set.E2, Reg8::E2),
            ("H2", set.H2, Reg8::H2),
            ("L2", set.L2, Reg8::L2),
            ("I", set.I, Reg8::I),
            ("R", set.R, Reg8::R),
        ];
        // Validate everything before changing anything.
        let mut word_values = Vec::new();
        for (name, value, reg) in words {
            if let Some(v) = check(name, value, 0xffff)? {
                word_values.push((reg, v as u16));
            }
        }
        let mut byte_values = Vec::new();
        for (name, value, high, low) in pairs {
            if let Some(v) = check(name, value, 0xffff)? {
                byte_values.push((high, (v >> 8) as u8));
                byte_values.push((low, v as u8));
            }
        }
        for (name, value, reg) in bytes {
            if let Some(v) = check(name, value, 0xff)? {
                byte_values.push((reg, v as u8));
            }
        }
        let iff1 = check("IFF1", set.IFF1, 1)?;
        let iff2 = check("IFF2", set.IFF2, 1)?;
        let im = check("IM", set.IM, 2)?;
        if word_values.is_empty()
            && byte_values.is_empty()
            && iff1.is_none()
            && iff2.is_none()
            && im.is_none()
        {
            return Err(ApiError::new(
                "no recognised field: expected Z80 register names (PC, A, IFF1, IM...)",
            ));
        }

        let cpu = motherboard.cpu_mut();
        for (reg, v) in word_values {
            cpu.reg16_mut(reg).set(v);
        }
        for (reg, v) in byte_values {
            cpu.reg8_mut(reg).set(v);
        }
        if iff1.is_some() || iff2.is_some() {
            let (current1, current2) = (cpu.iff1(), cpu.iff2());
            cpu.set_iffs(
                iff1.map_or(current1, |v| v == 1),
                iff2.map_or(current2, |v| v == 1),
            );
        }
        if let Some(im) = im {
            cpu.set_im(
                [
                    InterruptMode::Mode0,
                    InterruptMode::Mode1,
                    InterruptMode::Mode2,
                ][im as usize],
            );
        }
        Ok(CommandAck::new(self.command()))
    }

    /// `POST /api/step`.
    fn step_request(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let query: StepQuery = query_as(request)?;
        let ack = self.step_by(query.mode.unwrap_or_default(), motherboard)?;
        Ok(reply(&ack))
    }

    /// Run as far as `mode` says, then pause.
    fn step_by(&mut self, mode: StepMode, motherboard: &mut Motherboard) -> Result<CommandAck, ApiError> {
        let command = match mode {
            StepMode::In => "debug-step-in",
            StepMode::Over => "debug-step-over",
            StepMode::Out => "debug-step-out",
            StepMode::Frame => "debug-step-frame",
        };
        self.run_command(command, None, motherboard)?;
        Ok(CommandAck::new(self.cmd_seq))
    }

    /// `GET /api/ping`.
    fn ping(&self, motherboard: &Motherboard) -> Ping {
        Ping {
            ok: true,
            emu: self.emu(motherboard),
            frontend: "bemu".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            features: FEATURES.iter().map(|f| (*f).to_owned()).collect(),
        }
    }

    fn write_ram(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let write: WriteRam = body_as(request)?;
        if write.at.is_some() {
            return self.write_at(&write, motherboard);
        }
        if write.paging.is_some() {
            return Err(ApiError::field("paging", "a paging is for a CPU address: give it in \"at\""));
        }
        let addr = ranged("addr", write.addr, 0, 0xffff)?;
        let bank_count = motherboard.physical_bank_count() as i64;
        let bank = ranged("bank", write.bank, 0, bank_count - 1)?.unwrap_or(0) as usize;
        let data = match &write.data {
            None => Vec::new(),
            Some(text) => parse_hex(text)
                .ok_or_else(|| ApiError::field("data", "field \"data\" must be a hex string"))?,
        };
        let exec = write.exec.unwrap_or(false);
        let entry = ranged("entry", write.entry, 0, 0xffff)?;
        if data.is_empty() && !exec {
            return Err(ApiError::new(
                "nothing to do: no \"data\" to write and no \"exec\"",
            ));
        }
        let addr = match (addr, data.is_empty()) {
            (Some(addr), _) => addr as usize,
            (None, true) => 0,
            (None, false) => {
                return Err(ApiError::field(
                    "addr",
                    "missing field \"addr\": where to write \"data\"",
                ))
            }
        };
        for (i, &byte) in data.iter().enumerate() {
            // An offset past 0x3FFF carries into the next banks.
            let linear = bank * 0x4000 + addr + i;
            if !motherboard.write_bank_byte(
                (linear / 0x4000) as u16,
                (linear % 0x4000) as u16,
                byte,
            ) {
                return Err(ApiError::field(
                    "data",
                    format!(
                        "the write runs past the last RAM bank (B{:02X})",
                        bank_count - 1
                    ),
                ));
            }
        }
        if exec {
            let target = entry.unwrap_or(addr as i64) as u16;
            motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(target);
        }
        self.ram_seq += 1;
        Ok(reply(&MemoryAck {
            ok: true,
            seq: self.ram_seq,
            cmd_seq: self.command(),
        }))
    }

    /// `POST /api/ram` with `at`: a CPU address (the RAM a write there reaches, through
    /// `paging`) or an offset in a RAM bank; a ROM, a cartridge page, the registers refused.
    fn write_at(&mut self, write: &WriteRam, motherboard: &mut Motherboard) -> ApiResult {
        if write.addr.is_some() || write.bank.is_some() {
            return Err(ApiError::field("at", "\"at\" replaces addr and bank: give one or the other"));
        }
        let at = write.at.expect("write_at is for an \"at\"");
        let data = match &write.data {
            None => Vec::new(),
            Some(text) => parse_hex(text)
                .ok_or_else(|| ApiError::field("data", "field \"data\" must be a hex string"))?,
        };
        let exec = write.exec.unwrap_or(false);
        let entry = ranged("entry", write.entry, 0, 0xffff)?;
        if data.is_empty() && !exec {
            return Err(ApiError::new("nothing to do: no \"data\" to write and no \"exec\""));
        }
        // where each byte goes: a RAM bank and an offset in it
        let places: Vec<(u16, u16)> = match at {
            Location::Cpu(address) => {
                let through = crate::paging::Paging::from_api(&write.paging.unwrap_or_default(), motherboard);
                (0..data.len())
                    .map(|i| through.write_location(motherboard, address.wrapping_add(i as u16)))
                    .collect()
            }
            Location::In { space: Space::Ram(bank), offset } => {
                if write.paging.is_some() {
                    return Err(ApiError::field("paging", "a paging is for a CPU address, not a RAM bank"));
                }
                let start = u32::from(bank) * 0x4000 + offset;
                (0..data.len() as u32)
                    .map(|i| (((start + i) / 0x4000) as u16, ((start + i) % 0x4000) as u16))
                    .collect()
            }
            Location::In { space, .. } => {
                return Err(ApiError::field(
                    "at",
                    format!("{} is not written: a ROM, a cartridge page or the ASIC's registers", space.name()),
                ))
            }
        };
        for (&(bank, offset), &byte) in places.iter().zip(&data) {
            if !motherboard.write_bank_byte(bank, offset, byte) {
                return Err(ApiError::field(
                    "data",
                    format!(
                        "the write runs past the last RAM bank (B{:02X})",
                        motherboard.physical_bank_count() - 1
                    ),
                ));
            }
        }
        if exec {
            let target = match (entry, at) {
                (Some(entry), _) => entry as u16,
                (None, Location::Cpu(address)) => address,
                (None, _) => {
                    return Err(ApiError::field("entry", "exec at a RAM bank's offset needs the \"entry\" PC"))
                }
            };
            motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(target);
        }
        self.ram_seq += 1;
        Ok(reply(&MemoryAck {
            ok: true,
            seq: self.ram_seq,
            cmd_seq: self.command(),
        }))
    }

    fn exec(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let set: SetPc = body_as(request)?;
        let addr = ranged("addr", set.addr, 0, 0xffff)?
            .ok_or_else(|| ApiError::field("addr", "missing field \"addr\": the new PC"))?;
        self.run_command("debug-set-pc", Some(Arg::Number(addr)), motherboard)?;
        self.ram_seq += 1;
        Ok(reply(&MemoryAck {
            ok: true,
            seq: self.ram_seq,
            cmd_seq: self.command(),
        }))
    }

    fn config(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let set: SetConfig = body_as(request)?;
        let soft = set.do_soft_reset.unwrap_or(false);
        let hard = set.do_hard_reset.unwrap_or(false);
        let history_size = ranged("history_size", set.history_size, 1, 100_000)?;
        if set.paused.is_none() && !soft && !hard && history_size.is_none() {
            return Err(ApiError::new(
                "no recognised field: supported are paused, do_soft_reset, do_hard_reset, history_size"
            ));
        }
        // The same commands as the menus.
        if let Some(size) = history_size {
            // Resizing empties the history, as in AMSpiriT.
            self.run_command("history-set-size", Some(Arg::Number(size)), motherboard)?;
        }
        if soft || hard {
            // Soft: the reset button (RAM kept). Hard: power off and on.
            let reset = if hard {
                "machine-reset-hard"
            } else {
                "machine-reset-soft"
            };
            self.run_command(reset, None, motherboard)?;
        }
        if let Some(paused) = set.paused {
            let id = if paused { "debug-pause" } else { "debug-run" };
            self.run_command(id, None, motherboard)?;
        }
        Ok(reply(&CommandAck::new(self.command())))
    }

    /// `GET /api/config`.
    fn get_config(&self, motherboard: &Motherboard) -> Config {
        Config {
            crtc_type: motherboard.crtc().crtc_type() as u8,
            paused: self.paused,
            history_size: self.history_size as u64,
        }
    }

    /// `POST /api/z80_bp`: CPU addresses are resolved against the live
    /// mapping, `Bnn:hhhh` (hex 16K bank, hex offset carrying into the next
    /// banks) and the deprecated `Cnn:hhhh` (decimal bank) name a RAM byte.
    fn set_breakpoints(&mut self, request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
        let query: ExecBreakpointsQuery = query_as(request)?;
        let (breakpoints, resolved, warnings) = self.parse_exec_breakpoints(request, motherboard)?;
        if flag(query.add, false) {
            self.exec_breakpoints.extend(breakpoints);
        } else {
            self.exec_breakpoints = breakpoints;
        }
        Ok(reply(&ExecBreakpointsSet {
            ok: true,
            cmd_seq: self.command(),
            resolved,
            warnings: (!warnings.is_empty()).then_some(warnings),
        }))
    }

    /// `DELETE /api/z80_bp`: the breakpoints listed in the body (as `POST` takes them) removed,
    /// the others kept; an empty body removes them all.
    fn remove_exec_breakpoints(&mut self, request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
        let (breakpoints, resolved, warnings) = self.parse_exec_breakpoints(request, motherboard)?;
        if request.text().trim().is_empty() {
            self.exec_breakpoints.clear();
        } else {
            self.exec_breakpoints.retain(|location| !breakpoints.contains(location));
        }
        Ok(reply(&ExecBreakpointsRemoved {
            ok: true,
            cmd_seq: self.command(),
            removed: resolved,
            warnings: (!warnings.is_empty()).then_some(warnings),
        }))
    }

    /// `POST /api/run_to`: run to `addr` (a CPU address or a symbol), or to the beam at `line`
    /// and `nop`, once, then pause (a z80_bp event, reason run_to or run_to_beam).
    fn run_to_request(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let run_to: RunTo = body_as(request)?;
        if let Some(AddressOrSymbol(text)) = run_to.addr {
            self.run_command("debug-run-to-address", Some(Arg::Text(text)), motherboard)?;
            return Ok(reply(&CommandAck::new(self.cmd_seq)));
        }
        let line = ranged("line", run_to.line, 0, 0xffff)?;
        let nop = ranged("nop", run_to.nop, 0, NOPS_PER_LINE as i64 - 1)?;
        match line {
            Some(line) => {
                self.run_to_beam(line as usize, nop.unwrap_or(0) as usize);
                Ok(reply(&CommandAck::new(self.cmd_seq)))
            }
            None => Err(ApiError::new("expected addr, or line (and nop)")),
        }
    }

    /// The breakpoint list of `/api/z80_bp`'s body, resolved.
    #[allow(clippy::type_complexity)]
    fn parse_exec_breakpoints(
        &self,
        request: &ApiRequest,
        motherboard: &Motherboard,
    ) -> Result<(HashSet<PhysicalAddress>, Vec<ResolvedBreakpoint>, Vec<String>), ApiError> {
        let mut breakpoints = HashSet::new();
        let mut resolved = Vec::new();
        let mut warnings = Vec::new();
        let text = request.text();
        for entry in text.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let location = match entry.split_once(':') {
                Some((bank, offset)) if bank.starts_with(['B', 'b', 'C', 'c']) => {
                    let decimal = bank.starts_with(['C', 'c']);
                    if decimal {
                        warnings.push(format!(
                            "{entry}: the Cnn: form is deprecated, use Bnn: (hex bank)"
                        ));
                    }
                    let bank = u32::from_str_radix(&bank[1..], if decimal { 10 } else { 16 }).ok();
                    let offset = u32::from_str_radix(offset.trim_start_matches("0x"), 16).ok();
                    let (Some(bank), Some(offset)) = (bank, offset) else {
                        return Err(ApiError::new(format!(
                            "{entry}: expected Bnn:hhhh (hex bank, hex offset)"
                        )));
                    };
                    let linear = bank as u64 * 0x4000 + offset as u64;
                    let bank = linear / 0x4000;
                    if bank >= motherboard.physical_bank_count() as u64 {
                        return Err(ApiError::new(format!(
                            "{entry}: no such RAM bank (B00-B{:02X} exist)",
                            motherboard.physical_bank_count() - 1
                        )));
                    }
                    PhysicalAddress::Ram {
                        bank: bank as u16,
                        offset: (linear % 0x4000) as u16,
                    }
                }
                _ => {
                    let address = parse_int(entry)
                        .filter(|v| (0..=0xffff).contains(v))
                        .map(|v| v as u16)
                        .or_else(|| self.symbols.resolve(entry))
                        .ok_or_else(|| {
                            ApiError::new(format!(
                                "{entry}: expected a CPU address (decimal or 0x hex, 0-65535), a symbol (label, label+n) or Bnn:hhhh"
                            ))
                        })?;
                    motherboard.physical_address(address)
                }
            };
            resolved.push(ResolvedBreakpoint {
                entry: entry.to_owned(),
                location: location_name(location),
            });
            breakpoints.insert(location);
        }
        Ok((breakpoints, resolved, warnings))
    }

    /// `GET /api/breakpoints` (extension).
    fn list_breakpoints(&self) -> Breakpoints {
        let mut exec: Vec<String> = self
            .exec_breakpoints
            .iter()
            .map(|l| location_name(*l))
            .collect();
        exec.sort();
        Breakpoints {
            z80_bp: exec,
            advanced: self
                .advanced
                .iter()
                .enumerate()
                .map(|(index, b)| breakpoint_typed(index, b))
                .collect(),
        }
    }

    /// `POST /api/breakpoints` (extension): one advanced breakpoint, as a
    /// JSON object (`kind` exec|mem|io, `access` r|w|rw, `run` stop|watch,
    /// `addr`, `mask`, `size`, `value`, `valmask`, `name`) or as the text of
    /// a REMU `acebreak` entry in `remu`.
    fn add_breakpoint(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let new: NewBreakpoint = body_as(request)?;
        let brk = match &new.remu {
            Some(text) => {
                AdvancedBreakpoint::parse_remu(text).map_err(|e| ApiError::field("remu", e))?
            }
            None => breakpoint_of(&new)?,
        };
        self.advanced.push(brk);
        motherboard.watch_bus(self.watches_bus());
        Ok(reply(&BreakpointAdded {
            ok: true,
            index: self.advanced.len() as u64 - 1,
            cmd_seq: self.command(),
        }))
    }

    /// `DELETE /api/breakpoints[?index=N]` (extension): one, or all.
    fn remove_breakpoints(
        &mut self,
        request: &ApiRequest,
        motherboard: &mut Motherboard,
    ) -> ApiResult {
        refuse_empty(request, "index", "the breakpoint to remove (none: all)")?;
        let query: RemoveBreakpoint = query_as(request)?;
        match query.index {
            Some(index) => {
                let index = index
                    .within(0, self.advanced.len() as i64 - 1)
                    .ok_or_else(|| {
                        ApiError::field(
                            "index",
                            "parameter \"index\" must name an existing breakpoint",
                        )
                    })?;
                self.advanced.remove(index as usize);
            }
            None => self.advanced.clear(),
        }
        motherboard.watch_bus(self.watches_bus());
        Ok(reply(&CommandAck::new(self.command())))
    }

    /// Breakpoints read from elsewhere (e.g. a snapshot's chunks).
    pub fn add_advanced_breakpoint(
        &mut self,
        brk: AdvancedBreakpoint,
        motherboard: &mut Motherboard,
    ) {
        self.advanced.push(brk);
        motherboard.watch_bus(self.watches_bus());
    }

    /// Names for addresses (a snapshot's labels, symbol files), added to those known.
    pub fn add_symbols(&mut self, symbols: &crate::symbols::SymbolTable) {
        self.symbols.extend(symbols);
    }

    /// `GET /api/symbols` (extension): `[{name, address}]`, by address.
    fn list_symbols(&self) -> Vec<Symbol> {
        self.symbols
            .iter()
            .map(|(name, address)| Symbol {
                name: name.to_string(),
                address,
            })
            .collect()
    }

    /// Whether an execution breakpoint is on the CPU address `address` (as mapped now).
    pub fn has_exec_breakpoint(&self, address: u16, motherboard: &Motherboard) -> bool {
        self.exec_breakpoints
            .contains(&motherboard.physical_address(address))
    }

    /// Remove the execution breakpoint on the CPU address `address` (as mapped now): whether
    /// there was one.
    pub fn remove_exec_breakpoint(&mut self, address: u16, motherboard: &Motherboard) -> bool {
        self.exec_breakpoints
            .remove(&motherboard.physical_address(address))
    }

    /// The execution breakpoints, as CPU addresses where they are mapped now (the others,
    /// in a bank or a ROM not paged in, are left out).
    pub fn exec_breakpoints_mapped(&self, motherboard: &Motherboard) -> Vec<u16> {
        let offset = |location: &PhysicalAddress| match *location {
            PhysicalAddress::Ram { offset, .. }
            | PhysicalAddress::LowerRom { offset }
            | PhysicalAddress::UpperRom { offset, .. } => offset,
        };
        let mut found: Vec<u16> = self
            .exec_breakpoints
            .iter()
            .flat_map(|location| {
                // where in the CPU's 64K the location is paged now: one of the 4 regions
                (0..4u16)
                    .map(move |region| region * 0x4000 + offset(location))
                    .filter(|&address| motherboard.physical_address(address) == *location)
            })
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// How many execution breakpoints there are, mapped or not.
    pub fn exec_breakpoint_count(&self) -> usize {
        self.exec_breakpoints.len()
    }

    /// An execution breakpoint on a CPU address, resolved now.
    pub fn add_exec_breakpoint(&mut self, address: u16, motherboard: &Motherboard) {
        self.exec_breakpoints
            .insert(motherboard.physical_address(address));
    }

    /// `GET /api/history[?n=N]`: oldest first.
    fn history_list(&self, request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
        let query: HistoryQuery = query_as(request)?;
        let n = ranged("n", query.n, 1, 100_000)?.map_or(self.history.len(), |n| n as usize);
        let hex = |bytes: [u8; 4]| bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
        let entries: Vec<HistoryEntry> = self
            .history
            .iter()
            .skip(self.history.len().saturating_sub(n))
            .map(|&(pc, bytes)| HistoryEntry {
                pc,
                hex: hex(bytes),
                current_hex: hex(instruction_bytes(motherboard, pc)),
                label: self.symbols.describe(pc),
            })
            .collect();
        Ok(reply(&entries))
    }

    /// `GET /api/screenshot[?crop=][&live=][&full=]`, as AMSpiriT:
    /// - `crop=1` (default) the visible area (768x272), `crop=0` the whole
    ///   frame (1024x350, sync and borders included, the beam's coordinates);
    /// - `live=1` (default while paused) the frame being drawn, `live=0` the
    ///   last complete one;
    /// - `full=1` (default) the frame alone (black where the beam has not
    ///   been), `full=0` the frame being drawn over the previous one.
    ///
    /// `crop=0` and `live=1` need the monitor (the emulator's loop, not a
    /// bare `handle`).
    fn screenshot(&self, request: &ApiRequest, monitor: Option<&Monitor>) -> ApiResult {
        let query: ScreenshotQuery = query_as(request)?;
        let crop = flag(query.crop, true);
        let live = flag(query.live, self.paused);
        let alone = flag(query.full, true);
        let no_frame = || {
            Ok(ApiResponse::json(
                503,
                json!({ "error": "no frame available" }),
            ))
        };

        let settled = if crop {
            self.last_frame.as_ref()
        } else {
            monitor.and_then(Monitor::full_frame)
        };
        let drawing = match (live, monitor) {
            (true, Some(monitor)) if crop => Some(monitor.image_in_progress()),
            (true, Some(monitor)) => monitor.full_frame_in_progress(),
            _ => None,
        };
        if !crop && monitor.and_then(Monitor::full_frame_in_progress).is_none() {
            return Ok(ApiResponse::json(
                503,
                json!({ "error": "whole frames are not kept here (crop=0 needs the emulator's monitor)" }),
            ));
        }
        // Where the beam is in the returned picture: what is drawn so far.
        let (origin_x, origin_y) = match (crop, self.beam) {
            (true, Some(beam)) => (beam.crop_x, beam.crop_y as isize),
            _ => (0, 0),
        };
        let image = match drawing {
            Some(drawing) if alone || settled.is_none() => drawing.clone(),
            Some(drawing) => {
                let beam = self.beam.unwrap_or_default();
                composite(
                    drawing,
                    settled.unwrap(),
                    beam.line as isize - origin_y,
                    beam.x as isize - origin_x,
                )
            }
            None => match settled {
                Some(settled) => settled.clone(),
                None => return no_frame(),
            },
        };

        let mut png = std::io::Cursor::new(Vec::new());
        let (width, height) = image.dimensions();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut png, image::ImageOutputFormat::Png)
            .map_err(|e| ApiError::new(format!("PNG encoding failed: {e}")))?;
        let mut headers = vec![
            ("X-Crop-W", width.to_string()),
            ("X-Crop-H", height.to_string()),
            ("X-Crop-X", origin_x.to_string()),
            ("X-Crop-Y", origin_y.to_string()),
        ];
        if let Some(beam) = self.beam {
            headers.push(("X-Beam-X", beam.x.to_string()));
            headers.push(("X-Beam-Y", beam.line.to_string()));
        }
        Ok(ApiResponse {
            status: 200,
            content_type: "image/png",
            body: png.into_inner(),
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        })
    }

    /// `POST /api/media?name=&drive=&crtc=`: the raw file as the body. A
    /// snapshot or a disc image is recognised by its content, a binary by
    /// its name (`.bin`/`.amsdos`): with an AMSDOS header it loads at the
    /// header's address, headerless it needs `name@LOAD[@ENTRY].bin` (hex).
    /// The CPU jumps only to an `@ENTRY` given in the name; the reply gives
    /// the header's entry otherwise. A snapshot's breakpoints and symbols are set.
    /// `crtc` is accepted for compatibility: the CRTC is always kept.
    fn media(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let bytes = &request.body;
        if bytes.is_empty() {
            return Err(ApiError::new(
                "empty body: expected the raw media file bytes",
            ));
        }
        let query: MediaQuery = query_as(request)?;
        let name = query.name.as_deref().unwrap_or("");
        let drive = match ranged("drive", query.drive, 0, 1)? {
            None | Some(0) => crate::fdc::FdcDrive::A,
            _ => crate::fdc::FdcDrive::B,
        };
        let lower_name = name.to_ascii_lowercase();
        let mut loaded = MediaLoaded {
            ok: true,
            ..MediaLoaded::default()
        };
        if bytes.starts_with(b"MV - SNA") {
            let breakpoints =
                crate::snapshot::load_sna_bytes(motherboard, bytes).map_err(ApiError::new)?;
            for brk in &breakpoints.exec {
                self.add_exec_breakpoint(brk.address, motherboard);
            }
            for brk in &breakpoints.advanced {
                self.add_advanced_breakpoint(brk.clone(), motherboard);
            }
            self.add_symbols(&breakpoints.symbols);
            loaded.media = "sna".to_owned();
            loaded.breakpoints = Some(breakpoints.len() as u64);
            loaded.symbols = Some(breakpoints.symbols.len() as u64);
            if !breakpoints.ignored.is_empty() {
                loaded.warnings = Some(breakpoints.ignored.iter().map(ToString::to_string).collect());
            }
        } else if bytes.starts_with(b"MV - CPC")
            || bytes.starts_with(b"EXTENDED CPC DSK")
            || bytes.starts_with(b"HXCPICFE")
            || bytes.starts_with(b"HXCHFEV3")
            || (crate::disc_image::is_archive(bytes)
                && [".zip", ".7z", ".gz", ".tgz"].iter().any(|ext| {
                    crate::disc_image::split_disc_spec(name)
                        .0
                        .to_ascii_lowercase()
                        .ends_with(ext)
                }))
        {
            // a DSK, an HFE, or a zip archive holding one (`name` may be `games.zip#side b.dsk`)
            let (file, entry) = crate::disc_image::split_disc_spec(name);
            let image =
                crate::disc_image::load_disc_bytes(bytes, file, entry).map_err(ApiError::new)?;
            motherboard.insert_disc(drive, Some(image));
            loaded.media = "dsk".to_owned();
        } else if bytes.starts_with(b"ZXTape!") {
            let tape = crate::tape::Tape::from_cdt(bytes).map_err(ApiError::new)?;
            if !tape.notes.is_empty() {
                loaded.warnings = Some(tape.notes.iter().map(ToString::to_string).collect());
            }
            motherboard.insert_tape(Some(tape));
            loaded.media = "cdt".to_owned();
        } else if lower_name.ends_with(".bin") || lower_name.ends_with(".amsdos") {
            let binary = parse_binary(name, bytes).map_err(|e| ApiError::field("name", e))?;
            motherboard.write_buffer(binary.load, binary.data);
            if let Some(entry) = binary.jump {
                motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(entry);
            }
            loaded.media = "bin".to_owned();
            loaded.load = Some(binary.load);
            loaded.len = Some(binary.data.len() as u64);
            loaded.entry = binary.entry;
        } else {
            return Err(ApiError::new(
                "unsupported media: a .sna or .dsk image (recognised by its content), or a .bin/.amsdos named by ?name="
            ));
        }
        loaded.cmd_seq = self.command();
        Ok(reply(&loaded))
    }

    fn keytype(
        &mut self,
        request: &ApiRequest,
        layout: crate::keyboard::KeyboardLayout,
    ) -> ApiResult {
        let typed: KeyType = body_as(request)?;
        let text = typed.text.ok_or_else(|| {
            ApiError::field("text", "missing field \"text\": the characters to type")
        })?;
        for c in text.chars() {
            let c = if c == '\r' { '\n' } else { c };
            let keys = layout
                .keys_of_char(c)
                .ok_or_else(|| ApiError::field("text", format!("cannot type {c:?}")))?;
            self.typing.push_back(keys);
        }
        Ok(reply(&CommandAck::new(self.command())))
    }

    /// Press/release the typed keys that are due.
    fn type_due_keys(&mut self, motherboard: &mut Motherboard) {
        if self.typing.is_empty() && self.typed_pressed.is_none() {
            return;
        }
        // Keys typed for a program not loaded yet (`RUN`) wait for it.
        if self.pending_basic.is_some() && self.typed_pressed.is_none() {
            return;
        }
        let now = motherboard.t_state_counter().value();
        if now < self.typing_next_at {
            return;
        }
        if let Some(keys) = self.typed_pressed.take() {
            for key in keys {
                motherboard.keyboard_mut().set_pressed(key, false);
            }
        } else if let Some(keys) = self.typing.pop_front() {
            for &key in &keys {
                motherboard.keyboard_mut().set_pressed(key, true);
            }
            self.typed_pressed = Some(keys);
        }
        self.typing_next_at = now
            + if self.typed_pressed.is_some() {
                self.key_press_t_states
            } else {
                self.key_gap_t_states
            };
    }
}

/// AMSpiriT's `beam` object: the beam in the video buffer (1024 ticks by
/// 350 lines) and the visible area, plus the position in NOPs.
fn beam_json(beam: Beam) -> Value {
    to_json(&beam.state(false))
}



fn basic_state(motherboard: &Motherboard) -> Value {
    to_json(&basic_state_typed(motherboard))
}

fn basic_state_typed(motherboard: &Motherboard) -> api::BasicState {
    let state = BasicState::read(motherboard);
    let rom = basic_program::BasicRom::of(motherboard.model());
    api::BasicState {
        basic_ver: rom.version,
        prog_start: state.prog_start,
        txttop: state.txttop,
        vartop: state.vartop,
        arrend: state.arrend,
        prog_size: state.txttop.wrapping_sub(state.prog_start),
        var_size: state.vartop.wrapping_sub(state.txttop),
        chain_heads_addr: rom.variable_chain_heads,
        cur_linenum: state.line.unwrap_or(0xffff),
        stmt_addr: state.stmt_addr,
    }
}

fn basic_listing(motherboard: &Motherboard) -> ApiResult {
    let lines = basic_program::listing(motherboard)
        .map_err(|e| ApiError::new(format!("cannot decode the program: {e}")))?;
    Ok(reply(&BasicListing {
        lines: lines
            .iter()
            .map(|line| BasicLine {
                addr: line.addr,
                num: line.number,
                stmts: line
                    .statements
                    .iter()
                    .map(|s| BasicStatement {
                        addr: s.addr,
                        end: s.end,
                        colon: s.colon,
                        text: s.text.clone(),
                        vars: s.vars.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }))
}

/// The visible picture (what `GET /api/screenshot` returns).
const SCREENSHOT_SIZE: (usize, usize) = (768, 272);

/// The frame being drawn over the previous one: `drawing` where the beam
/// has been (the rows before `row`, and `row` up to `column`), `previous`
/// elsewhere.
fn composite(
    drawing: &image::RgbImage,
    previous: &image::RgbImage,
    row: isize,
    column: isize,
) -> image::RgbImage {
    let mut out = previous.clone();
    if out.dimensions() != drawing.dimensions() {
        return drawing.clone();
    }
    for (x, y, pixel) in drawing.enumerate_pixels() {
        let (x, y) = (x as isize, y as isize);
        if y < row || (y == row && x < column) {
            out.put_pixel(x as u32, y as u32, *pixel);
        }
    }
    out
}

/// `0xRRGGBB` of a GA colour.
fn rgb(levels: crate::ga::RgbLevels) -> u32 {
    let to_u8 = |v: f32| (v * 255.0 / 100.0).round().clamp(0.0, 255.0) as u32;
    to_u8(levels.r()) << 16 | to_u8(levels.g()) << 8 | to_u8(levels.b())
}

fn ga(motherboard: &Motherboard) -> Value {
    to_json(&ga_state(motherboard))
}

fn ga_state(motherboard: &Motherboard) -> GateArray {
    use crate::ga::VideoMode;
    let ga = motherboard.ga();
    let palette = ga.palette();
    let mmr = motherboard.mmr();
    GateArray {
        mode: match ga.active_video_mode() {
            VideoMode::Mode0 => 0,
            VideoMode::Mode1 => 1,
            VideoMode::Mode2 => 2,
            VideoMode::Mode3 => 3,
        },
        pen: ga.selected_pen(),
        // The hardware colour numbers (0x40 | n is the INKR write).
        border_idx: palette[16].hardware_number(),
        border_rgb: rgb(palette[16].into()),
        ink_idx: palette[..16].iter().map(|ink| ink.hardware_number()).collect(),
        ink_rgb: palette[..16].iter().map(|&ink| rgb(ink.into())).collect(),
        rmr: ga.rmr(),
        ram_mode: mmr & 7,
        ram_page: (mmr >> 3) & 7,
        // Extension: firmware ink numbers, ROM enables, interrupt counter.
        ink_firmware: palette.iter().map(|ink| ink.number()).collect(),
        lower_rom: ga.lower_rom_pagging().is_enabled(),
        upper_rom: ga.upper_rom_pagging().is_enabled(),
        r52: ga.r52(),
    }
}

fn psg(motherboard: &Motherboard) -> Value {
    to_json(&psg_state(motherboard))
}

fn psg_state(motherboard: &Motherboard) -> Psg {
    crate::state::psg(motherboard)
}

fn fdc(motherboard: &Motherboard) -> Value {
    to_json(&fdc_state(motherboard))
}

fn fdc_state(motherboard: &Motherboard) -> Fdc {
    crate::state::fdc(motherboard)
}

/// `GET /api/asic`: a CPC+'s ASIC (`null` on a CPC).
fn asic(motherboard: &Motherboard) -> Value {
    to_json(&crate::state::asic(motherboard))
}

/// `POST /api/drive?drive=0|1[&kind=3|3.5][&flip=1]`: make a drive a 3" or a 3.5" one, turn the
/// disc of a 3" drive over. Replies with the FDC state.
impl DebugServer {
    /// `POST /api/drive`, through the commands of File > Drive A / B.
    fn drive_command(
        &mut self,
        request: &ApiRequest,
        motherboard: &mut Motherboard,
    ) -> Result<ApiResponse, ApiError> {
        use crate::fdc::DriveKind;
        let command: DriveCommand = query_as(request)?;
        let drive = match ranged("drive", command.drive, 0, 1)? {
            None | Some(0) => "drive-a",
            _ => "drive-b",
        };
        if let Some(kind) = command.kind {
            let kind: DriveKind = kind
                .parse()
                .map_err(|e: String| ApiError::field("kind", e))?;
            let id = match kind {
                DriveKind::ThreeInch => format!("{drive}-original"),
                DriveKind::ThreeAndHalfInch => format!("{drive}-pc"),
                DriveKind::Gotek => format!("{drive}-gotek"),
            };
            self.run_command(&id, None, motherboard)?;
        }
        if flag(command.flip, false)
            && self
                .run_command(&format!("{drive}-flip-disc"), None, motherboard)
                .is_err()
        {
            return Err(ApiError::field(
                "flip",
                "only the disc of a 3\" drive with a disc in it can be turned over",
            ));
        }
        Ok(reply(&fdc_state(motherboard)))
    }

    /// Run the command `id` of the menus (see [`crate::commands`]): the API and the menus do
    /// the same thing the same way.
    fn run_command(
        &mut self,
        id: &str,
        arg: Option<Arg>,
        motherboard: &mut Motherboard,
    ) -> Result<(), ApiError> {
        let command = match arg {
            Some(arg) => Command::with_arg(id, arg),
            None => Command::new(id),
        }
        .map_err(|e| ApiError::new(e.to_string()))?;
        let mut target = ServerTarget {
            server: self,
            motherboard,
        };
        match command.execute(&mut target) {
            Ok(Outcome::Done(_)) => Ok(()),
            Ok(Outcome::NeedsArg(prompt)) => {
                Err(ApiError::new(format!("{}: missing", prompt.title)))
            }
            Err(error) => Err(ApiError::new(error.to_string())),
        }
    }
}

/// The commands' receiver for the API: the machine and the debugger, no window.
struct ServerTarget<'a> {
    server: &'a mut DebugServer,
    motherboard: &'a mut Motherboard,
}

impl CommandTarget for ServerTarget<'_> {
    fn motherboard(&self) -> &Motherboard {
        self.motherboard
    }
    fn motherboard_mut(&mut self) -> &mut Motherboard {
        self.motherboard
    }
    fn debugger(&self) -> Option<&DebugServer> {
        Some(self.server)
    }
    fn debugger_mut(&mut self) -> Option<&mut DebugServer> {
        Some(self.server)
    }
    fn debugger_and_motherboard(&mut self) -> Option<(&mut DebugServer, &mut Motherboard)> {
        Some((self.server, self.motherboard))
    }
}

/// `GET /api/memmap`: each 16K region of the CPU's view, with the ROM
/// read there if any and the absolute RAM bank (written there in any case).
fn memmap(motherboard: &Motherboard) -> Value {
    let paging = crate::paging::Paging::from_api(&bemu_api::Paging::default(), motherboard);
    to_json(&memmap_state(motherboard, &paging))
}

/// `GET /api/memmap[?paging=]`.
fn memmap_request(request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
    let query: MemMapQuery = query_as(request)?;
    let paging = crate::paging::Paging::from_api(&query.paging.unwrap_or_default(), motherboard);
    Ok(reply(&memmap_state(motherboard, &paging)))
}

/// The map of `paging`: what each region reads (the ROM, or which RAM bank), the bank written,
/// and the registers' values that make it.
fn memmap_state(motherboard: &Motherboard, paging: &crate::paging::Paging) -> MemMap {
    use crate::paging::Source;
    let regions = (0..4u16)
        .map(|region| {
            let base = region * 0x4000;
            let source = paging.source(motherboard, region);
            let mut value = MemRegion {
                base,
                name: format!("{base:04X}"),
                rom: true,
                rom_bank: None,
                ram_bank: None,
                ext: None,
                write_bank: paging.write_location(motherboard, base).0,
                source: Some(paging.space(motherboard, region).name()),
            };
            match source {
                // AMSpiriT's numbers: 255 the lower ROM, else the upper ROM's
                Source::LowerRom => value.rom_bank = Some(255),
                Source::UpperRom(slot) => value.rom_bank = Some(u16::from(slot)),
                Source::Cartridge(_) if region == 3 => value.rom_bank = Some(u16::from(paging.upper_number)),
                Source::Cartridge(_) => value.rom_bank = Some(255),
                Source::Registers => value.rom = false,
                Source::Ram(bank) => {
                    value.rom = false;
                    value.ram_bank = Some(bank);
                    value.ext = Some(bank >= 4);
                }
            }
            value
        })
        .collect();
    let (mmr, rmr) = if paging.follow {
        (motherboard.mmr(), motherboard.ga().rmr())
    } else {
        // the values that would page it: the RAM's, and RMR's ROM bits (set: disabled)
        let mmr = 0xc0 | ((paging.page & 7) << 3) | paging.ram_config;
        let rmr = (motherboard.ga().rmr() & !0x0c)
            | if paging.lower_rom { 0 } else { 0x04 }
            | if paging.upper_rom { 0 } else { 0x08 };
        (mmr, rmr)
    };
    MemMap {
        regions,
        rmr,
        ram_mode: mmr & 7,
        ram_page: (mmr >> 3) & 7,
        mmr,
        ram_banks: motherboard.physical_bank_count() as u32,
    }
}

fn z80(motherboard: &Motherboard) -> Value {
    to_json(&z80_registers(motherboard))
}

fn z80_registers(motherboard: &Motherboard) -> Z80Registers {
    crate::state::z80(motherboard)
}

/// A binary to load: where, what, and where to start.
struct Binary<'b> {
    load: u16,
    data: &'b [u8],
    /// The program's entry (header or name).
    entry: Option<u16>,
    /// The entry to jump to: only one given in the name.
    jump: Option<u16>,
}

/// An AMSDOS header (128 bytes, checksum of bytes 0-66 at 67-68) gives the
/// load address (21-22), the length (24-25) and the entry (26-27); a
/// headerless file needs `name@LOAD[@ENTRY].ext`, hex addresses.
fn parse_binary<'b>(name: &str, bytes: &'b [u8]) -> Result<Binary<'b>, String> {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let mut parts = stem.split('@').skip(1);
    let hex = |text: &str| {
        u16::from_str_radix(text, 16)
            .map_err(|_| format!("{text}: expected a hex address in the name"))
    };
    let named_load = parts.next().map(hex).transpose()?;
    let named_entry = parts.next().map(hex).transpose()?;

    let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let has_header = bytes.len() >= 128 && {
        let sum: u16 = bytes[..67].iter().map(|&b| b as u16).sum();
        sum == word(67)
    };
    if has_header {
        let length = (word(24) as usize).min(bytes.len() - 128);
        return Ok(Binary {
            load: named_load.unwrap_or(word(21)),
            data: &bytes[128..128 + length],
            entry: named_entry.or(Some(word(26))),
            jump: named_entry,
        });
    }
    let load = named_load
        .ok_or("no AMSDOS header: give the load address in the name, e.g. game@4000.bin")?;
    Ok(Binary {
        load,
        data: bytes,
        entry: named_entry,
        jump: named_entry,
    })
}

/// Decode `%XX` and `+` in a query value.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the HTTP server is the desktop's
/// A query string's pairs, decoded: what [`ApiRequest::query`] holds.
pub fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if let Some(byte) = text
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok()) =>
            {
                out.push(byte);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What a breakpoint condition reads: the machine, and the access that hit.
struct MachineContext<'m> {
    motherboard: &'m Motherboard,
    access: Option<BusAccess>,
    /// For `LineCount`/`LineCycle`: the beam, when a monitor drives it.
    beam: Option<Beam>,
}

impl ConditionContext for MachineContext<'_> {
    fn variable(&self, name: &str) -> Option<i64> {
        let cpu = self.motherboard.cpu();
        let r8 = |reg: Reg8| Some(cpu.reg8(reg).get() as i64);
        let r16 = |reg: Reg16| Some(cpu.reg16(reg).get() as i64);
        match name {
            "A" => r8(Reg8::A),
            "F" => r8(Reg8::F),
            "B" => r8(Reg8::B),
            "C" => r8(Reg8::C),
            "D" => r8(Reg8::D),
            "E" => r8(Reg8::E),
            "H" => r8(Reg8::H),
            "L" => r8(Reg8::L),
            "I" => r8(Reg8::I),
            "R" => r8(Reg8::R),
            "IXH" => r8(Reg8::Ixh),
            "IXL" => r8(Reg8::Ixl),
            "IYH" => r8(Reg8::Iyh),
            "IYL" => r8(Reg8::Iyl),
            "AF" => r16(Reg16::Af),
            "BC" => r16(Reg16::Bc),
            "DE" => r16(Reg16::De),
            "HL" => r16(Reg16::Hl),
            "IX" => r16(Reg16::Ix),
            "IY" => r16(Reg16::Iy),
            "SP" => r16(Reg16::Sp),
            "PC" => r16(Reg16::Pc),
            "A'" => r8(Reg8::A2),
            "F'" => r8(Reg8::F2),
            "B'" => r8(Reg8::B2),
            "C'" => r8(Reg8::C2),
            "D'" => r8(Reg8::D2),
            "E'" => r8(Reg8::E2),
            "H'" => r8(Reg8::H2),
            "L'" => r8(Reg8::L2),
            "AF'" => r16(Reg16::Af2),
            "BC'" => r16(Reg16::Bc2),
            "DE'" => r16(Reg16::De2),
            "HL'" => r16(Reg16::Hl2),
            "IFF1" => Some(cpu.iff1() as i64),
            "IFF2" => Some(cpu.iff2() as i64),
            "IM" => Some(match cpu.im() {
                InterruptMode::Mode0 => 0,
                InterruptMode::Mode1 => 1,
                InterruptMode::Mode2 => 2,
            }),
            "ADDRESS" => self.access.map(|a| a.address as i64),
            "VALUE" => self.access.map(|a| a.value as i64),
            _ => self.ace_variable(name),
        }
    }

    fn peek(&self, address: u16) -> u8 {
        self.motherboard.read_byte(address)
    }
}

impl MachineContext<'_> {
    /// ACE's variables beyond the registers: flags, CRTC, beam, memory.
    fn ace_variable(&self, name: &str) -> Option<i64> {
        let motherboard = self.motherboard;
        let flags = motherboard.cpu().reg8(Reg8::F).get();
        let flag = |bit: u8, set: bool| Some(((flags >> bit & 1 == 1) == set) as i64);
        let crtc = motherboard.crtc();
        let mmr = motherboard.mmr();
        let ga = motherboard.ga();
        if let Some(number) = name
            .strip_prefix("CRTCR")
            .and_then(|n| n.parse::<u8>().ok())
        {
            return Some(crtc.register_value(number) as i64);
        }
        match name {
            "FLAGZ" => flag(6, true),
            "FLAGNZ" => flag(6, false),
            "FLAGC" => flag(0, true),
            "FLAGNC" => flag(0, false),
            "FLAGM" => flag(7, true),
            "FLAGP" => flag(7, false),
            "FLAGPE" => flag(2, true),
            "FLAGPO" => flag(2, false),
            "C0" => Some(crtc.hcc as i64),
            "C3H" => Some(crtc.hsc as i64),
            "C3V" => Some(crtc.vsc as i64),
            "C4" => Some(crtc.vcc.value() as i64),
            "C5" => Some(crtc.vtac as i64),
            "C9" => Some(crtc.vlc.value() as i64),
            "CRTCSEL" => Some(crtc.selected_register() as i64),
            "LINECOUNT" => self.beam.map(|beam| beam.line as i64),
            "LINECYCLE" => self.beam.map(|beam| beam.nop() as i64),
            "MEMPAGE" => Some((mmr >> 3 & 7) as i64),
            "MEMBANKMODE" => Some((mmr & 7) as i64),
            "LOWERROMON" => Some(ga.lower_rom_pagging().is_enabled() as i64),
            "UPPERROMON" => Some(ga.upper_rom_pagging().is_enabled() as i64),
            "UPPERROMNUM" => Some(motherboard.upper_rom_number() as i64),
            // A Plus's: no ASIC here, the lower ROM is the firmware's at 0.
            "LOWERROMNUM" | "LOWERROMADR" | "ASICPAGEON" => Some(0),
            "ASICPAGEOFF" => Some(1),
            _ => None,
        }
    }
}

/// The 4 bytes at `pc`, as the CPU sees them now.
fn instruction_bytes(motherboard: &Motherboard, pc: u16) -> [u8; 4] {
    std::array::from_fn(|i| motherboard.read_byte(pc.wrapping_add(i as u16)))
}

fn hex16(value: u16) -> String {
    format!("0x{value:04X}")
}

/// The registers part of the `frame`/`z80_bp`/`watch` events.
fn registers_event(motherboard: &Motherboard) -> EventRegisters {
    let cpu = motherboard.cpu();
    let r8 = |reg: Reg8| cpu.reg8(reg).get();
    let r16 = |reg: Reg16| hex16(cpu.reg16(reg).get());
    EventRegisters {
        pc: r16(Reg16::Pc),
        sp: r16(Reg16::Sp),
        a: r8(Reg8::A),
        f: r8(Reg8::F),
        b: r8(Reg8::B),
        c: r8(Reg8::C),
        d: r8(Reg8::D),
        e: r8(Reg8::E),
        h: r8(Reg8::H),
        l: r8(Reg8::L),
        ix: r16(Reg16::Ix),
        iy: r16(Reg16::Iy),
    }
}

/// What a stop event says beyond the machine's state.
struct StopInfo {
    /// See [`StopEvent::kind`].
    kind: &'static str,
    index: Option<u64>,
    name: Option<String>,
    hits: Option<u64>,
    access: Option<EventAccess>,
}

impl StopInfo {
    fn of(kind: &'static str) -> Self {
        StopInfo {
            kind,
            index: None,
            name: None,
            hits: None,
            access: None,
        }
    }
}

/// What a `z80_bp` event's `kind` means, in one word: `breakpoint` (an
/// execution, memory or I/O breakpoint), `raster`, `step`, `step_over`
/// (its run to the instruction after), `step_out`, `run_to`, `run_to_beam`.
fn stop_reason(kind: &str) -> StopReason {
    match kind {
        "raster" => StopReason::Raster,
        "step" => StopReason::Step,
        "step_out" => StopReason::StepOut,
        "run_to" => StopReason::RunTo,
        "run_to_beam" => StopReason::RunToBeam,
        _ => StopReason::Breakpoint,
    }
}

fn kind_name(kind: BreakpointKind) -> &'static str {
    match kind {
        BreakpointKind::Exec => "exec",
        BreakpointKind::Memory => "mem",
        BreakpointKind::Io => "io",
    }
}

/// `B04:0200`, `LOWER:0038`, `UPPER07:0006`.
fn location_name(location: PhysicalAddress) -> String {
    match location {
        PhysicalAddress::Ram { bank, offset } => format!("B{bank:02X}:{offset:04X}"),
        PhysicalAddress::LowerRom { offset } => format!("LOWER:{offset:04X}"),
        PhysicalAddress::UpperRom { rom, offset } => format!("UPPER{rom:02X}:{offset:04X}"),
    }
}

fn breakpoint_typed(index: usize, brk: &AdvancedBreakpoint) -> api::AdvancedBreakpoint {
    api::AdvancedBreakpoint {
        index: index as u64,
        kind: match brk.kind {
            BreakpointKind::Exec => api::BreakpointKind::Exec,
            BreakpointKind::Memory => api::BreakpointKind::Mem,
            BreakpointKind::Io => api::BreakpointKind::Io,
        },
        access: match brk.access {
            BreakpointAccess::Read => api::BreakpointAccess::Read,
            BreakpointAccess::Write => api::BreakpointAccess::Write,
            BreakpointAccess::ReadWrite => api::BreakpointAccess::ReadWrite,
        },
        run: match brk.run {
            RunMode::Stop => api::BreakpointRun::Stop,
            RunMode::Watch => api::BreakpointRun::Watch,
            RunMode::Count => api::BreakpointRun::Count,
        },
        addr: brk.address,
        mask: brk.mask,
        size: brk.size,
        value: brk.value,
        valmask: brk.value_mask,
        name: brk.name.clone(),
        condition: brk.condition.as_ref().map(|c| c.source().to_owned()),
        counters: brk
            .counters
            .iter()
            .map(|(action, name)| api::CounterLink {
                action: match action {
                    CounterAction::Reset => api::CounterAction::Reset,
                    CounterAction::Start => api::CounterAction::Start,
                    CounterAction::Stop => api::CounterAction::Stop,
                },
                name: name.clone(),
            })
            .collect(),
        after: brk.after,
        hits: brk.hits,
    }
}

fn breakpoint_of(new: &NewBreakpoint) -> Result<AdvancedBreakpoint, ApiError> {
    let mut brk = AdvancedBreakpoint::exec(0);
    brk.kind = match new.kind.unwrap_or(api::BreakpointKind::Exec) {
        api::BreakpointKind::Exec => BreakpointKind::Exec,
        api::BreakpointKind::Mem => BreakpointKind::Memory,
        api::BreakpointKind::Io => BreakpointKind::Io,
    };
    brk.access = match new.access.unwrap_or(api::BreakpointAccess::ReadWrite) {
        api::BreakpointAccess::Read => BreakpointAccess::Read,
        api::BreakpointAccess::Write => BreakpointAccess::Write,
        api::BreakpointAccess::ReadWrite => BreakpointAccess::ReadWrite,
    };
    brk.run = match new.run.unwrap_or(api::BreakpointRun::Stop) {
        api::BreakpointRun::Stop => RunMode::Stop,
        api::BreakpointRun::Watch => RunMode::Watch,
        api::BreakpointRun::Count => RunMode::Count,
    };
    brk.address = ranged("addr", new.addr, 0, 0xffff)?
        .ok_or_else(|| ApiError::field("addr", "missing field \"addr\""))? as u16;
    brk.mask = ranged("mask", new.mask, 0, 0xffff)?.unwrap_or(0xffff) as u16;
    brk.size = ranged("size", new.size, 1, 0xffff)?.unwrap_or(1) as u16;
    brk.value = ranged("value", new.value, 0, 0xff)?.unwrap_or(0) as u8;
    brk.value_mask = ranged("valmask", new.valmask, 0, 0xff)?.unwrap_or(0) as u8;
    brk.name = new.name.clone();
    brk.condition = match new.condition.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(text) => Some(Condition::parse(text).map_err(|e| ApiError::field("condition", e))?),
    };
    // "start"/"stop"/"reset": a counter name or a list of them.
    for (action, names) in [
        (CounterAction::Reset, &new.reset),
        (CounterAction::Start, &new.start),
        (CounterAction::Stop, &new.stop),
    ] {
        let names = names.as_ref().map(|n| n.0.clone()).unwrap_or_default();
        if names.iter().any(String::is_empty) {
            return Err(ApiError::field(action.name(), "a counter name cannot be empty"));
        }
        brk.counters
            .extend(names.into_iter().map(|name| (action, name)));
    }
    brk.after = ranged("after", new.after, 0, u32::MAX as i64)?.unwrap_or(0) as u64;
    if brk.run == RunMode::Count && brk.counters.is_empty() {
        return Err(ApiError::field(
            "run",
            "run \"count\" needs counters to act on: start, stop or reset",
        ));
    }
    Ok(brk)
}

fn read_ram(request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
    let query: ReadRam = query_as(request)?;
    let len = ranged("len", query.len, 1, 0x10000)?.unwrap_or(256);
    if let Some(at) = query.at {
        if query.addr.is_some() || query.bank.is_some() || query.view.is_some() {
            return Err(ApiError::field("at", "\"at\" replaces addr, bank and view: give one or the other"));
        }
        let (addr, hex, paging) = match at {
            Location::Cpu(address) => {
                let paging = query.paging.unwrap_or_default();
                let through = crate::paging::Paging::from_api(&paging, motherboard);
                let hex = (0..len)
                    .map(|i| format!("{:02x}", through.read(motherboard, address.wrapping_add(i as u16))))
                    .collect();
                (u32::from(address), hex, (!paging.is_cpu()).then_some(paging))
            }
            Location::In { space, offset } => {
                if query.paging.is_some() {
                    return Err(ApiError::field("paging", "a paging is for a CPU address, not a space"));
                }
                let mut hex = String::with_capacity(2 * len as usize);
                for i in 0..len as u32 {
                    let byte = crate::paging::read_space(motherboard, space, offset + i).ok_or_else(|| {
                        ApiError::field("at", format!("no {} here (or it ends before {} bytes)", space.name(), len))
                    })?;
                    hex.push_str(&format!("{byte:02x}"));
                }
                (offset, hex, None)
            }
        };
        return Ok(reply(&Ram {
            addr,
            len: len as u32,
            hex,
            view: None,
            at: Some(at),
            paging,
        }));
    }
    let addr = ranged("addr", query.addr, 0, 0xffff)?.unwrap_or(0);
    let bank = ranged("bank", query.bank, 0, motherboard.physical_bank_count() as i64 - 1)?.unwrap_or(0);
    let view = query.view.unwrap_or(RamView::Raw);
    let paging = query.paging.filter(|paging| !paging.is_cpu());
    if paging.is_some() && view == RamView::Raw {
        return Err(ApiError::field("paging", "a paging is for the CPU's addresses: view=cpu, or at"));
    }
    let through = crate::paging::Paging::from_api(&paging.unwrap_or_default(), motherboard);
    // Past the last bank, the read wraps to the first one.
    let total = motherboard.physical_bank_count() as i64 * 0x4000;
    let byte_at = |i: i64| -> u8 {
        match view {
            RamView::Cpu => through.read(motherboard, (addr + i) as u16),
            RamView::Raw => {
                let linear = (bank * 0x4000 + addr + i) % total;
                motherboard
                    .read_bank_byte((linear / 0x4000) as u16, (linear % 0x4000) as u16)
                    .unwrap_or(0xff)
            }
        }
    };
    let hex: String = (0..len).map(|i| format!("{:02x}", byte_at(i))).collect();
    Ok(reply(&Ram {
        addr: addr as u32,
        len: len as u32,
        hex,
        view: (view == RamView::Cpu).then_some(RamView::Cpu),
        at: None,
        paging,
    }))
}

/// `GET /api/doc[?endpoint=][&format=json|openapi|html]`.
fn doc(request: &ApiRequest) -> ApiResult {
    let query: api::DocQuery = query_as(request)?;
    match (query.format.unwrap_or(DocFormat::Json), query.endpoint) {
        (DocFormat::Json, None) => Ok(ApiResponse::ok(api_doc::index())),
        (DocFormat::Json, Some(name)) => match api_doc::endpoint(&name) {
            Some(value) => Ok(ApiResponse::ok(value)),
            None => Err(ApiError::field(
                "endpoint",
                format!("no endpoint \"{name}\" (GET /api/doc lists them)"),
            )),
        },
        (DocFormat::Openapi, _) => Ok(ApiResponse::ok(api_doc::openapi())),
        (DocFormat::Html, _) => Ok(ApiResponse {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: api_doc::html().into_bytes(),
            headers: Vec::new(),
        }),
    }
}

// ------------------------------------------------------------- bodies

fn parse_body(body: &str) -> Result<Value, ApiError> {
    if body.trim().is_empty() {
        return Err(ApiError::new("empty body: expected a JSON object"));
    }
    match serde_json::from_str::<Value>(body) {
        Ok(value @ Value::Object(_)) => Ok(value),
        Ok(_) => Err(ApiError::new("expected a JSON object")),
        Err(e) => Err(ApiError::new(format!("expected a JSON object: {e}"))),
    }
}

/// A reply, as JSON.
fn to_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("the API's replies are JSON")
}

/// A reply: 200, `value` as JSON.
fn reply<T: serde::Serialize>(value: &T) -> ApiResponse {
    ApiResponse::ok(to_json(value))
}

/// A query flag, `default` when absent.
fn flag(value: Option<Flag>, default: bool) -> bool {
    value.map_or(default, |f| f.0)
}

/// An integer of a request, checked to be within `min..=max`.
fn ranged(name: &str, value: Option<Int>, min: i64, max: i64) -> Result<Option<i64>, ApiError> {
    value
        .map(|v| {
            v.within(min, max).ok_or_else(|| {
                ApiError::field(name, format!("\"{name}\" must be between {min} and {max}"))
            })
        })
        .transpose()
}

/// The query as `T` (see [`bemu_api`]): each parameter a string, [`Int`] reads numbers from them.
fn query_as<T: serde::de::DeserializeOwned>(request: &ApiRequest) -> Result<T, ApiError> {
    // An empty value (`?mode=`) is no value.
    let map: serde_json::Map<String, Value> = request
        .query
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    serde_path_to_error::deserialize(Value::Object(map)).map_err(typed_error)
}

/// `name` given empty (`?name=`): refused where no value means "all" and removes everything,
/// rather than taken for no value.
fn refuse_empty(request: &ApiRequest, name: &str, what: &str) -> Result<(), ApiError> {
    if request.query.iter().any(|(k, v)| k == name && v.is_empty()) {
        return Err(ApiError::field(
            name,
            format!("\"{name}\" is empty: give {what}, or leave it out"),
        ));
    }
    Ok(())
}

/// The JSON body as `T` (see [`bemu_api`]), its fields lifted from any depth first (the
/// AMSpiriT convention: `{"a":1}` and `{"x":{"a":1}}` both carry `a`).
fn body_as<T: serde::de::DeserializeOwned>(request: &ApiRequest) -> Result<T, ApiError> {
    let body = parse_body(&request.text())?;
    serde_path_to_error::deserialize(lift_fields(&body)).map_err(typed_error)
}

fn typed_error(error: serde_path_to_error::Error<serde_json::Error>) -> ApiError {
    let path = error.path().to_string();
    let message = error.into_inner().to_string();
    if path == "." {
        ApiError::new(message)
    } else {
        ApiError::field(&path, format!("field \"{path}\": {message}"))
    }
}

/// Every field of `value` at the top level, wherever it is nested: for each name, the one at
/// the top, else the first found depth first.
fn lift_fields(value: &Value) -> Value {
    fn walk(value: &Value, out: &mut serde_json::Map<String, Value>) {
        match value {
            Value::Object(map) => {
                for (key, v) in map {
                    out.entry(key.clone()).or_insert_with(|| v.clone());
                }
                for v in map.values() {
                    walk(v, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let mut out = serde_json::Map::new();
    walk(value, &mut out);
    Value::Object(out)
}

fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// Hex bytes; spaces and ':' are ignored.
fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<char> = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(&pair.iter().collect::<String>(), 16).ok())
        .collect()
}

// --------------------------------------------------------------- HTTP

/// The HTTP thread: parse, hand to the emulation thread, write the answer.

#[cfg(test)]
mod test {
    use super::*;
    use crate::motherboard::MotherboardBuilder;

    fn machine() -> Motherboard {
        MotherboardBuilder::default()
            .disable_lower_rom()
            .disable_upper_rom()
            .build()
    }

    fn call(
        server: &mut DebugServer,
        motherboard: &mut Motherboard,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, Value) {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let query = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap();
                (k.to_owned(), v.to_owned())
            })
            .collect();
        let request = ApiRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            query,
            body: body.as_bytes().to_vec(),
        };
        let response = server.handle(&request, motherboard);
        (
            response.status,
            serde_json::from_slice(&response.body).unwrap(),
        )
    }

    fn server() -> DebugServer {
        DebugServer::with_requests(mpsc::channel().1)
    }

    #[test]
    fn memmap_and_ga_follow_the_mmr_and_the_inks() {
        let (mut server, mut mb) = (server(), machine());
        // LD BC,0x7FC4: OUT (C),C (page 0 bank 0 at 0x4000);
        // LD C,0: OUT (C),C (pen 0); LD C,0x54: OUT (C),C (hardware 20).
        mb.write_buffer(
            0x4000,
            &[
                0x01, 0xc4, 0x7f, 0xed, 0x49, 0x0e, 0x00, 0xed, 0x49, 0x0e, 0x54, 0xed, 0x49,
            ],
        );
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        // The code runs from 0x4000 in bank 1: copy it into the bank about
        // to be mapped there.
        for i in 0..13 {
            let byte = mb.read_bank_byte(1, i).unwrap();
            mb.write_bank_byte(4, i, byte);
        }
        run(&mut server, &mut mb, 6);
        let (status, memmap) = call(&mut server, &mut mb, "GET", "/api/memmap", "");
        assert_eq!(status, 200);
        assert_eq!(memmap["ram_mode"], 4);
        assert_eq!(memmap["regions"][1]["ram_bank"], 4);
        assert_eq!(memmap["regions"][1]["ext"], true);
        assert_eq!(memmap["regions"][1]["rom"], false);
        assert_eq!(memmap["regions"][3]["ram_bank"], 3);
        let (_, ga) = call(&mut server, &mut mb, "GET", "/api/ga", "");
        assert_eq!(ga["ink_idx"][0], 20, "{ga}");
        assert_eq!(ga["ink_firmware"][0], 0);
        assert_eq!(ga["pen"], 0);
        for path in ["/api/crtc", "/api/psg", "/api/fdc"] {
            assert_eq!(call(&mut server, &mut mb, "GET", path, "").0, 200, "{path}");
        }
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/state", "");
        assert_eq!(state["fdc"]["msr"], 128);
    }

    #[test]
    fn memmap_names_the_roms() {
        let (mut server, mut mb) = (server(), MotherboardBuilder::default().build());
        let (_, memmap) = call(&mut server, &mut mb, "GET", "/api/memmap", "");
        assert_eq!(memmap["regions"][0]["rom"], true);
        assert_eq!(memmap["regions"][0]["rom_bank"], 255);
        assert!(
            memmap["regions"][0].get("ram_bank").is_none(),
            "only for RAM, as in AMSpiriT"
        );
        assert_eq!(
            memmap["regions"][0]["write_bank"], 0,
            "writes go to the RAM below"
        );
        assert_eq!(memmap["regions"][1]["name"], "4000");
        assert_eq!(memmap["regions"][3]["rom"], true);
        assert_eq!(memmap["regions"][3]["rom_bank"], 0);
    }

    fn beam(x: usize, line: usize) -> Beam {
        Beam {
            x,
            line,
            crop_x: 126,
            crop_y: 33,
        }
    }

    #[test]
    fn raster_breakpoints_fire_when_the_beam_passes() {
        let at = |line, nop| RasterBreakpoint {
            line,
            x: nop * BEAM_TICKS_PER_NOP,
            name: None,
        };
        // Line 100, NOP 10.
        assert!(at(Some(100), 10).passed(beam(150, 100), beam(170, 100)));
        assert!(at(Some(100), 10).passed(beam(150, 100), beam(160, 100)));
        assert!(
            !at(Some(100), 10).passed(beam(160, 100), beam(200, 100)),
            "already passed"
        );
        assert!(!at(Some(100), 10).passed(beam(150, 99), beam(170, 99)));
        // The start of a line, reached through the end of the previous.
        assert!(at(Some(100), 0).passed(beam(1000, 99), beam(30, 100)));
        // Through a new frame.
        assert!(at(Some(0), 1).passed(beam(1000, 311), beam(40, 0)));
        assert!(
            at(Some(311), 63).passed(beam(1000, 311), beam(40, 0)),
            "the end of the frame"
        );
        assert!(!at(Some(100), 0).passed(beam(1000, 311), beam(40, 0)));
        // Every line.
        assert!(at(None, 10).passed(beam(150, 7), beam(170, 7)));
        assert!(at(None, 10).passed(beam(150, 8), beam(170, 8)));
        assert!(at(None, 0).passed(beam(1010, 8), beam(10, 9)));
        assert!(!at(None, 10).passed(beam(170, 8), beam(200, 8)));
    }

    #[test]
    fn raster_bp_positions_in_pixels_or_nops() {
        let (mut server, mut mb) = (server(), machine());
        let (status, set) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/raster_bp?x=200&y=50",
            "",
        );
        assert_eq!(status, 200);
        assert_eq!(set["breakpoints"][0]["frame_nop"], 50 * 64 + 12);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/raster_bp?frame_nop=19967&add=1",
            "",
        );
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/raster_bp?nop=5&add=1",
            "",
        );
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/raster_bp", "");
        assert_eq!(list["breakpoints"][1]["line"], 311);
        assert_eq!(list["breakpoints"][1]["nop"], 63);
        assert!(list["breakpoints"][2].get("line").is_none(), "{list}");
        // Picture coordinates need the beam.
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "POST",
                "/api/raster_bp?x=0&y=0&crop=1",
                ""
            )
            .0,
            400
        );
        server.set_beam(beam(0, 0));
        let (_, set) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/raster_bp?x=2&y=0&crop=1",
            "",
        );
        assert_eq!(set["breakpoints"].as_array().unwrap().len(), 1, "replaces");
        assert_eq!(set["breakpoints"][0]["x"], 128);
        assert_eq!(set["breakpoints"][0]["line"], 33);
        assert_eq!(
            call(&mut server, &mut mb, "POST", "/api/raster_bp?nop=64", "").0,
            400
        );
        let (_, cleared) = call(&mut server, &mut mb, "POST", "/api/raster_bp?enable=0", "");
        assert_eq!(cleared["breakpoints"], json!([]));
        let (_, position) = call(&mut server, &mut mb, "GET", "/api/beam?crop=1", "");
        assert_eq!(
            (position["x"].clone(), position["y"].clone()),
            (json!(-126), json!(-33))
        );
        assert_eq!(position["nops_per_frame"], 19968);
    }

    #[test]
    fn a_raster_breakpoint_stops_the_running_machine_on_its_line() {
        use crate::monitor::Monitor;
        let (mut server, mut mb) = (server(), MotherboardBuilder::default().build());
        let mut monitor = Monitor::default();
        let events = server.events.subscribe(None).unwrap();
        let mut step = |server: &mut DebugServer, mb: &mut Motherboard| {
            mb.execute_next_opcode_with_video(|(rgb, sync)| monitor.receive(rgb, sync));
            let (x, line) = monitor.beam();
            let (crop_x, crop_y) = monitor.crop_origin();
            server.set_beam(Beam {
                x,
                line,
                crop_x,
                crop_y,
            });
            server.after_instruction(mb);
        };
        // Let the firmware set the CRTC up and the monitor lock.
        for _ in 0..4 * 19968 / 3 {
            step(&mut server, &mut mb);
        }
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/raster_bp?line=200&nop=20&name=split",
            "",
        );
        for _ in 0..19968 {
            step(&mut server, &mut mb);
            if server.paused() {
                break;
            }
        }
        assert!(server.paused());
        let (_, position) = call(&mut server, &mut mb, "GET", "/api/beam", "");
        assert_eq!(position["line"], 200, "{position}");
        let nop = position["nop"].as_u64().unwrap();
        assert!((20..20 + 7).contains(&nop), "{position}");
        let stop = events_of(&events)
            .into_iter()
            .find(|e| e.starts_with("event: z80_bp"))
            .unwrap();
        assert!(
            stop.contains(r#""kind":"raster""#) && stop.contains(r#""line":200"#),
            "{stop}"
        );
        // Which one, by its index and name, and why.
        assert!(
            stop.contains(r#""index":0"#)
                && stop.contains(r#""name":"split""#)
                && stop.contains(r#""reason":"raster""#),
            "{stop}"
        );
    }

    #[test]
    fn counters_count_nops_while_running() {
        let (mut server, mut mb) = (server(), machine());
        // NOP; NOP; LD A,1 (2 NOPs); JR -5 (3 NOPs).
        mb.write_buffer(0x4000, &[0x00, 0x00, 0x3e, 0x01, 0x18, 0xfa]);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        server.after_instruction(&mut mb);
        let (status, _) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/counters",
            r#"{"name":"loop"}"#,
        );
        assert_eq!(status, 200);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/counters",
            r#"{"name":"idle","running":false}"#,
        );
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "POST",
                "/api/counters",
                r#"{"name":"loop"}"#
            )
            .0,
            400,
            "exists"
        );
        run(&mut server, &mut mb, 4);
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(
            list["counters"][0],
            json!({ "name": "idle", "nops": 0, "running": false })
        );
        assert_eq!(list["counters"][1]["nops"], 7, "{list}");
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/counters",
            r#"{"name":"loop","action":"stop"}"#,
        );
        run(&mut server, &mut mb, 4);
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(list["counters"][1]["nops"], 7);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/counters",
            r#"{"name":"loop","action":"reset"}"#,
        );
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/counters",
            r#"{"name":"loop","action":"start"}"#,
        );
        run(&mut server, &mut mb, 2);
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(list["counters"][1]["nops"], 2, "{list}");
        // In the stop events.
        let events = server.events.subscribe(None).unwrap();
        call(&mut server, &mut mb, "POST", "/api/z80_bp", "0x4000");
        run(&mut server, &mut mb, 4);
        let stop = events_of(&events)
            .into_iter()
            .find(|e| e.starts_with("event: z80_bp"))
            .unwrap();
        assert!(stop.contains(r#""counters":{"idle":0,"loop":"#), "{stop}");
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "DELETE",
                "/api/counters?name=idle",
                ""
            )
            .0,
            200
        );
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "DELETE",
                "/api/counters?name=idle",
                ""
            )
            .0,
            400
        );
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "POST",
                "/api/counters",
                r#"{"name":"loop","action":"x"}"#
            )
            .0,
            400
        );
        let (_, list) = call(&mut server, &mut mb, "DELETE", "/api/counters", "");
        assert_eq!(list["counters"], json!([]));
    }

    /// A 6128 booted to BASIC's prompt.
    fn booted() -> Motherboard {
        booted_model(crate::motherboard::CpcModel::Cpc6128)
    }

    fn booted_model(model: crate::motherboard::CpcModel) -> Motherboard {
        let mut builder = MotherboardBuilder::default();
        builder.set_model(model);
        let mut mb = builder.build();
        while mb.t_state_counter().value() < BASIC_BOOT_T_STATES {
            mb.execute_next_opcode_no_video();
        }
        mb
    }

    /// ACE's condition variables, read from a booted 6128.
    #[test]
    fn ace_condition_variables_read_the_machine() {
        let mut mb = booted();
        mb.cpu_mut().reg8_mut(Reg8::F).set(0b0100_0001); // Z and C
        let beam = Beam {
            x: 10 * BEAM_TICKS_PER_NOP + 3,
            line: 100,
            crop_x: 0,
            crop_y: 0,
        };
        let context = MachineContext {
            motherboard: &mb,
            access: None,
            beam: Some(beam),
        };
        let holds = |source: &str| Condition::parse(source).unwrap().holds(&context);
        assert!(holds("FlagZ AND FlagC AND NOT FlagNZ AND FlagNC = 0"));
        assert!(holds("FlagP = 1 AND FlagM = 0 AND FlagPO = 1"));
        // The firmware's CRTC set-up.
        assert!(holds(
            "CrtcR0 = 63 AND CrtcR1 = 40 AND CrtcR2 = 46 AND CrtcR6 = 25 AND CrtcR7 = 30"
        ));
        assert!(holds("C0 <= CrtcR0 AND C4 <= CrtcR4 AND C9 <= CrtcR9"));
        assert!(holds("LineCount = 100 AND LineCycle = 10"));
        assert!(holds("MemBankMode = 0 AND MemPage = 0"));
        // BASIC running: the upper ROM (BASIC, ROM 0) paged in at times; the
        // number is read, whatever it is now.
        assert!(holds(&format!("UpperROMNum = {}", mb.upper_rom_number())));
        assert!(holds(
            "ASICPageOn = 0 AND ASICPageOff = 1 AND LowerROMNum = 0 AND LowerROMAdr = 0"
        ));
        // No monitor: no beam, the raster variables are 0.
        let headless = MachineContext {
            motherboard: &mb,
            access: None,
            beam: None,
        };
        assert!(!Condition::parse("LineCount OR LineCycle")
            .unwrap()
            .holds(&headless));
    }

    #[test]
    fn basic_programs_load_list_and_stop_on_lines_and_statements() {
        // (a CPC+ starts in its cartridge's menu, not in BASIC)
        for model in crate::motherboard::CpcModel::ALL
            .into_iter()
            .filter(|model| !model.is_plus())
        {
            basic_program_on(model);
        }
    }

    fn basic_program_on(model: crate::motherboard::CpcModel) {
        let (mut server, mut mb) = (server(), booted_model(model));
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/ping", "").1["emu"]["cpc_model"],
            model.name()
        );
        let events = server
            .events
            .subscribe(Some(vec!["basic_bp".to_owned()]))
            .unwrap();
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(state["prog_start"], 368, "{state}");
        assert_eq!(state["basic_ver"], model.basic_version());
        assert_eq!(state["prog_size"], 2, "an empty program: {state}");
        assert_eq!(state["cur_linenum"], 0xffff);

        let source = "10 A=1:B=2\n20 A=A+1:GOTO 20\n";
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "POST",
                "/api/basic?run=1",
                "10 GOTO GOTO"
            )
            .0,
            400
        );
        let (status, _) = call(&mut server, &mut mb, "POST", "/api/basic?run=1", source);
        assert_eq!(status, 200);
        let (_, listing) = call(&mut server, &mut mb, "GET", "/api/basic_listing", "");
        let line10 = &listing["lines"][0];
        assert_eq!(
            (line10["addr"].clone(), line10["num"].clone()),
            (json!(368), json!(10)),
            "{listing}"
        );
        assert_eq!(line10["stmts"][0]["addr"], 371);
        assert_eq!(line10["stmts"][0]["text"], "A=1");
        assert_eq!(line10["stmts"][0]["vars"], json!(["A"]));
        assert_eq!(line10["stmts"][1]["colon"], true);
        assert_eq!(line10["stmts"][0]["end"], line10["stmts"][1]["addr"]);
        let line20 = listing["lines"][1].clone();
        assert_eq!(line20["num"], 20);
        assert_eq!(line20["stmts"][1]["text"], "GOTO 20", "{listing}");
        let goto = line20["stmts"][1]["addr"].as_u64().unwrap();

        // Typed RUN reaches line 20.
        call(&mut server, &mut mb, "POST", "/api/basic_bp", "20");
        run(&mut server, &mut mb, 3_000_000);
        assert!(
            server.paused(),
            "{:?}",
            call(&mut server, &mut mb, "GET", "/api/basic_state", "")
        );
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(state["cur_linenum"], 20);
        assert_eq!(state["stmt_addr"], line20["stmts"][0]["addr"]);
        let hits = events_of(&events);
        assert!(
            hits[0].contains(r#""line":20"#) && hits[0].contains(r#""reason":"breakpoint""#),
            "{hits:?}"
        );

        // Statement by statement, then line by line (the same line again).
        call(&mut server, &mut mb, "POST", "/api/basic_bp", "");
        call(&mut server, &mut mb, "POST", "/api/basic_step", "");
        run(&mut server, &mut mb, 100_000);
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(state["stmt_addr"], goto, "{state}");
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/basic_step?mode=line",
            "",
        );
        run(&mut server, &mut mb, 100_000);
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(
            (state["cur_linenum"].clone(), state["stmt_addr"].clone()),
            (json!(20), line20["stmts"][0]["addr"].clone())
        );
        call(
            &mut server,
            &mut mb,
            "POST",
            &format!("/api/basic_runto?addr={goto}"),
            "",
        );
        run(&mut server, &mut mb, 100_000);
        assert!(server.paused());
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/basic_state", "").1["stmt_addr"],
            goto
        );
        assert!(events_of(&events)
            .iter()
            .any(|e| e.contains(r#""reason":"runto""#)));

        // The variables are where the state says.
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert!(state["var_size"].as_u64().unwrap() > 0, "{state}");
        let response = server.handle(
            &ApiRequest {
                method: "GET".into(),
                path: "/api/basic_export".into(),
                query: vec![],
                body: vec![],
            },
            &mut mb,
        );
        let text = String::from_utf8(response.body).unwrap();
        assert!(text.contains("GOTO 20"), "{text}");
    }

    #[test]
    fn basic_sent_while_the_firmware_boots_waits_for_it() {
        // Sent the moment the machine is powered on, then RUN typed: the
        // program is loaded once BASIC is up, and RUN after it.
        let (mut server, mut mb) = (server(), MotherboardBuilder::default().build());
        call(&mut server, &mut mb, "POST", "/api/basic", "10 GOTO 10\n");
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/keytype",
            r#"{"text":"RUN\r"}"#,
        );
        call(&mut server, &mut mb, "POST", "/api/basic_bp", "10");
        run(&mut server, &mut mb, 4_000_000);
        assert!(server.paused(), "line 10 reached");
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(
            (state["prog_start"].clone(), state["cur_linenum"].clone()),
            (json!(368), json!(10)),
            "{state}"
        );
    }

    #[test]
    fn basic_after_a_reset_loads_once_booted() {
        let (mut server, mut mb) = (server(), booted());
        let events = server
            .events
            .subscribe(Some(vec!["reset".to_owned()]))
            .unwrap();
        let (status, _) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/basic?reset=1&run=1",
            "10 DIM X(10)\n20 GOTO 20\n",
        );
        assert_eq!(status, 200);
        assert_eq!(events_of(&events).len(), 1);
        call(&mut server, &mut mb, "POST", "/api/basic_bp", "20");
        run(&mut server, &mut mb, 6_000_000);
        assert!(server.paused());
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(state["cur_linenum"], 20, "{state}");
        let (txttop, vartop, arrend) = (
            state["txttop"].as_u64().unwrap(),
            state["vartop"].as_u64().unwrap(),
            state["arrend"].as_u64().unwrap(),
        );
        assert!(txttop > 368 + 2 && vartop >= txttop, "{state}");
        assert!(arrend >= vartop + 11 * 5, "the array of 11 reals: {state}");
    }

    #[test]
    fn breakpoints_drive_the_counters_and_count_their_hits() {
        let (mut server, mut mb) = (server(), machine());
        // LD B,10; loop: NOP; NOP; DJNZ loop; JR $
        mb.write_buffer(0x4000, &[0x06, 0x0a, 0x00, 0x00, 0x10, 0xfc, 0x18, 0xfe]);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        let add = |server: &mut DebugServer, mb: &mut Motherboard, body: &str| {
            call(server, mb, "POST", "/api/breakpoints", body)
        };
        assert_eq!(
            add(
                &mut server,
                &mut mb,
                r#"{"addr":"0x4002","run":"count","reset":"loop","start":"loop","after":0}"#
            )
            .0,
            200
        );
        // Only reset at the first pass: from the second on, keep counting.
        call(&mut server, &mut mb, "DELETE", "/api/breakpoints", "");
        add(
            &mut server,
            &mut mb,
            r#"{"addr":"0x4000","run":"count","reset":"loop"}"#,
        );
        add(
            &mut server,
            &mut mb,
            r#"{"addr":"0x4002","run":"count","start":"loop"}"#,
        );
        add(
            &mut server,
            &mut mb,
            r#"{"addr":"0x4006","stop":"loop","name":"done"}"#,
        );
        let events = server.events.subscribe(None).unwrap();
        run(&mut server, &mut mb, 100);
        assert!(server.paused());
        assert_eq!(mb.pc(), 0x4006);
        let (_, counters) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(
            counters["counters"][0],
            json!({ "name": "loop", "nops": 9 * 6 + 5, "running": false }),
            "{counters}"
        );
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/breakpoints", "");
        assert_eq!(list["advanced"][1]["hits"], 10);
        assert_eq!(
            list["advanced"][1]["counters"],
            json!([{ "action": "start", "name": "loop" }])
        );
        let all = events_of(&events);
        assert!(
            !all.iter().any(|e| e.starts_with("event: watch")),
            "count mode is silent"
        );
        let stop = all.iter().find(|e| e.starts_with("event: z80_bp")).unwrap();
        assert!(
            stop.contains(r#""counters":{"loop":59}"#) && stop.contains(r#""hits":1"#),
            "{stop}"
        );

        // A pass count: stop at the 5th hit inside the loop.
        call(&mut server, &mut mb, "DELETE", "/api/breakpoints", "");
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        add(&mut server, &mut mb, r#"{"addr":"0x4003","after":4}"#);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/config",
            r#"{"paused":false}"#,
        );
        run(&mut server, &mut mb, 100);
        assert_eq!(
            (mb.pc(), mb.cpu().reg8(crate::z80::Reg8::B).value()),
            (0x4003, 6)
        );
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/breakpoints", "").1["advanced"][0]["hits"],
            5
        );

        assert_eq!(
            add(&mut server, &mut mb, r#"{"addr":1,"run":"count"}"#).0,
            400,
            "count without counters"
        );
        assert_eq!(add(&mut server, &mut mb, r#"{"addr":1,"start":5}"#).0, 400);
    }

    /// cpclib-basic's tokenisation (what `POST /api/basic` loads) against
    /// the ROM's own, the line typed on a booted 6128: a program loaded
    /// through the debug server is the one the CPC would have stored.
    #[test]
    fn cpclib_tokenises_as_the_rom() {
        for line in [
            "10 ON ERROR GOTO 30",
            "20 ERROR 5",
            "30 GOTO 30",
            "40 A=1:PRINT A",
            "50 DATA 1,2,300",
            "60 X=RND*100",
        ] {
            let (mut server, mut mb) = (self::server(), booted());
            call(
                &mut server,
                &mut mb,
                "POST",
                "/api/keytype",
                &format!(r#"{{"text":"{line}\r"}}"#),
            );
            run(&mut server, &mut mb, 1_500_000);
            let ours = cpclib::basic::BasicProgram::parse(format!("{line}\n"))
                .unwrap()
                .as_bytes();
            let length =
                u16::from_le_bytes([mb.read_video_ram_byte(0x170), mb.read_video_ram_byte(0x171)])
                    as usize;
            let typed: Vec<u8> = (0x170..0x170 + length as u16 + 2)
                .map(|a| mb.read_video_ram_byte(a))
                .collect();
            assert_eq!(ours, typed, "{line}");
        }
    }

    #[test]
    fn basic_flags_are_where_each_rom_keeps_them() {
        use crate::motherboard::CpcModel;
        for model in [CpcModel::Cpc464, CpcModel::Cpc6128] {
            let rom = basic_program::BasicRom::of(model);
            let (mut server, mut mb) = (server(), booted_model(model));
            let flag = |mb: &Motherboard, i: usize| mb.read_video_ram_byte(rom.flags[i]);
            // ERR: the last error's number.
            call(
                &mut server,
                &mut mb,
                "POST",
                "/api/basic?run=1",
                "10 ERROR 5\n",
            );
            run(&mut server, &mut mb, 3_000_000);
            assert_eq!(flag(&mb, 1), 5, "{}: ERR", model.name());
            // AUTO keeps its next line and step after its flag: typed on a
            // fresh machine, both 10.
            let (mut auto_server, mut auto_mb) = (self::server(), booted_model(model));
            call(
                &mut auto_server,
                &mut auto_mb,
                "POST",
                "/api/keytype",
                r#"{"text":"AUTO\r"}"#,
            );
            run(&mut auto_server, &mut auto_mb, 4_000_000);
            let auto = rom.flags[0];
            let fields = [auto + 1, auto + 3].map(|a| auto_mb.read_video_ram_byte(a));
            assert_eq!(
                fields,
                [10, 10],
                "{}: AUTO line and step after {auto:#06x}",
                model.name()
            );
        }
    }

    #[test]
    fn soft_resets_keep_the_ram_hard_ones_clear_it() {
        let (mut server, mut mb) = (server(), MotherboardBuilder::default().build());
        mb.write_bank_byte(1, 0x10, 0x42);
        mb.write_bank_byte(5, 0x10, 0x43);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/config",
            r#"{"do_soft_reset":true}"#,
        );
        assert_eq!(
            (
                mb.pc(),
                mb.read_bank_byte(1, 0x10),
                mb.read_bank_byte(5, 0x10)
            ),
            (0, Some(0x42), Some(0x43))
        );
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/config",
            r#"{"do_hard_reset":true}"#,
        );
        assert_eq!(
            (mb.read_bank_byte(1, 0x10), mb.read_bank_byte(5, 0x10)),
            (Some(0), Some(0))
        );
        mb.execute_next_opcode_no_video();
        assert_ne!(mb.pc(), 0, "runs after a hard reset");
    }

    #[test]
    fn ping_identifies_the_frontend() {
        let (mut server, mut mb) = (server(), machine());
        let (status, body) = call(&mut server, &mut mb, "GET", "/api/ping", "");
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(body["frontend"], "bemu");
        assert_eq!(body["emu"]["paused"], false);
    }

    #[test]
    fn unknown_paths_and_methods() {
        let (mut server, mut mb) = (server(), machine());
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/nothing", "").0, 404);
        let request = ApiRequest {
            method: "PUT".into(),
            path: "/api/ram".into(),
            query: vec![],
            body: Vec::new(),
        };
        let response = server.handle(&request, &mut mb);
        assert_eq!(response.status, 405);
        assert_eq!(response.header("allow"), Some("GET, POST"));
    }

    #[test]
    fn registers_round_trip_and_partial_update() {
        let (mut server, mut mb) = (server(), machine());
        let (status, body) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/z80",
            r#"{"PC":"0x4000","A":62,"IM":2}"#,
        );
        assert_eq!((status, body["ok"].clone()), (200, json!(true)));
        let (_, z80) = call(&mut server, &mut mb, "GET", "/api/z80", "");
        assert_eq!(
            (z80["PC"].clone(), z80["A"].clone(), z80["IM"].clone()),
            (json!(0x4000), json!(62), json!(2))
        );
        let (status, error) = call(&mut server, &mut mb, "POST", "/api/z80", r#"{"A":256}"#);
        assert_eq!((status, error["field"].clone()), (400, json!("A")));
        assert_eq!(
            call(&mut server, &mut mb, "POST", "/api/z80", r#"{}"#).0,
            400
        );
    }

    #[test]
    fn ram_write_then_read_and_nested_keys() {
        let (mut server, mut mb) = (server(), machine());
        let (status, body) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/ram",
            r#"{"write":{"addr":4096,"data":"3e 01:c9"}}"#,
        );
        assert_eq!((status, body["seq"].clone()), (200, json!(1)));
        let (_, read) = call(
            &mut server,
            &mut mb,
            "GET",
            "/api/ram?addr=0x1000&len=3",
            "",
        );
        assert_eq!(read["hex"], "3e01c9");
        let (_, cpu) = call(
            &mut server,
            &mut mb,
            "GET",
            "/api/ram?addr=4096&len=3&view=cpu",
            "",
        );
        assert_eq!(
            (cpu["hex"].clone(), cpu["view"].clone()),
            (json!("3e01c9"), json!("cpu"))
        );
        let (status, error) = call(&mut server, &mut mb, "GET", "/api/ram?len=0", "");
        assert_eq!((status, error["field"].clone()), (400, json!("len")));
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/ram", "RUN").0, 400);
    }

    #[test]
    fn step_runs_one_instruction_then_pauses() {
        let (mut server, mut mb) = (server(), machine());
        mb.write_buffer(0x4000, &[0x3e, 0x42, 0x00]); // LD A,0x42; NOP
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/config",
            r#"{"paused":true}"#,
        );
        server.poll(&mut mb);
        let (_, step) = call(&mut server, &mut mb, "POST", "/api/step", "");
        assert!(!server.paused());
        mb.execute_next_opcode_no_video();
        server.after_instruction(&mut mb);
        assert!(server.paused());
        let (_, ping) = call(&mut server, &mut mb, "GET", "/api/ping", "");
        assert_eq!(ping["emu"]["applied_cmd_seq"], step["cmd_seq"]);
        assert_eq!(mb.cpu().reg8(Reg8::A).get(), 0x42);
        assert_eq!(mb.pc(), 0x4002);
    }

    #[test]
    fn breakpoints_pause_on_the_address() {
        let (mut server, mut mb) = (server(), machine());
        mb.write_buffer(0x4000, &[0x00, 0x00, 0x00, 0x18, 0xfb]); // NOP x3; JR 0x4000
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        assert_eq!(
            call(&mut server, &mut mb, "POST", "/api/z80_bp", "0x4002, 16387").0,
            200
        );
        for _ in 0..2 {
            mb.execute_next_opcode_no_video();
            server.after_instruction(&mut mb);
        }
        assert!(server.paused());
        assert_eq!(server.take_stop(), Some(0x4002));
        let bank_count = mb.physical_bank_count();
        let missing = format!("B{bank_count:02X}:0000");
        assert_eq!(
            call(&mut server, &mut mb, "POST", "/api/z80_bp", &missing).0,
            400
        );
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/z80_bp", "").0, 200);
        assert!(server.exec_breakpoints.is_empty());
    }

    /// Run up to `n` instructions, reporting each to the server, stopping
    /// when it pauses - as the emulation loop does.
    fn run(server: &mut DebugServer, mb: &mut Motherboard, n: usize) {
        for _ in 0..n {
            mb.execute_next_opcode_no_video();
            server.after_instruction(mb);
            if server.paused() {
                break;
            }
        }
    }

    fn events_of(queue: &Arc<(Mutex<SseQueue>, Condvar)>) -> Vec<String> {
        queue.0.lock().unwrap().events.drain(..).collect()
    }

    /// `PC` at 0x4000, paused.
    fn paused_at_4000(server: &mut DebugServer, mb: &mut Motherboard) {
        call(server, mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        call(server, mb, "POST", "/api/config", r#"{"paused":true}"#);
        server.poll(mb);
    }

    #[test]
    fn a_step_is_told_with_its_reason_and_command() {
        let (mut server, mut mb) = (server(), machine());
        mb.write_buffer(0x4000, &[0x00, 0x00]); // NOP; NOP
        paused_at_4000(&mut server, &mut mb);
        let events = server.events.subscribe(None).unwrap();
        let (_, step) = call(&mut server, &mut mb, "POST", "/api/step", "");
        run(&mut server, &mut mb, 1);
        let stops: Vec<String> = events_of(&events)
            .into_iter()
            .filter(|e| e.starts_with("event: z80_bp"))
            .collect();
        assert_eq!(stops.len(), 1, "{stops:?}");
        assert!(stops[0].contains(r#""reason":"step""#), "{}", stops[0]);
        assert!(
            stops[0].contains(&format!(r#""cmd_seq":{}"#, step["cmd_seq"])),
            "{} {step}",
            stops[0]
        );
    }

    #[test]
    fn step_over_runs_a_call_whole_and_step_out_leaves_the_routine() {
        let (mut server, mut mb) = (server(), machine());
        // 0x4000: CALL 0x4010; NOP. 0x4010: NOP; NOP; RET
        mb.write_buffer(0x4000, &[0xcd, 0x10, 0x40, 0x00]);
        mb.write_buffer(0x4010, &[0x00, 0x00, 0xc9]);
        mb.cpu_mut().reg16_mut(Reg16::Sp).set(0xbf00);
        paused_at_4000(&mut server, &mut mb);

        let (status, _) = call(&mut server, &mut mb, "POST", "/api/step?mode=over", "");
        assert_eq!(status, 200);
        run(&mut server, &mut mb, 100);
        assert!(server.paused());
        assert_eq!(mb.pc(), 0x4003, "after the CALL");

        // Into the routine, then out of it.
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        server.poll(&mut mb);
        call(&mut server, &mut mb, "POST", "/api/step", "");
        run(&mut server, &mut mb, 1);
        assert_eq!(mb.pc(), 0x4010);
        call(&mut server, &mut mb, "POST", "/api/step?mode=out", "");
        run(&mut server, &mut mb, 100);
        assert!(server.paused());
        assert_eq!(mb.pc(), 0x4003, "back after the CALL");

        let (status, refused) = call(&mut server, &mut mb, "POST", "/api/step?mode=sideways", "");
        assert_eq!(status, 400, "{refused}");
    }

    #[test]
    fn run_to_stops_once_at_the_address() {
        let (mut server, mut mb) = (server(), machine());
        mb.write_buffer(0x4000, &[0x00, 0x00, 0x00, 0x18, 0xfb]); // NOP x3; JR 0x4000
        paused_at_4000(&mut server, &mut mb);
        let events = server.events.subscribe(None).unwrap();
        let (status, _) = call(&mut server, &mut mb, "POST", "/api/run_to", r#"{"addr":"0x4002"}"#);
        assert_eq!(status, 200);
        run(&mut server, &mut mb, 100);
        assert!(server.paused());
        assert_eq!(mb.pc(), 0x4002);
        assert!(events_of(&events)
            .iter()
            .any(|e| e.starts_with("event: z80_bp") && e.contains(r#""reason":"run_to""#)));
        // Once: resumed, it does not stop there again.
        call(&mut server, &mut mb, "POST", "/api/config", r#"{"paused":false}"#);
        server.poll(&mut mb);
        run(&mut server, &mut mb, 20);
        assert!(!server.paused());

        let (status, _) = call(&mut server, &mut mb, "POST", "/api/run_to", r#"{"nothing":1}"#);
        assert_eq!(status, 400);
    }

    /// The hex `/api/ram` answers for `query`.
    fn ram_hex(server: &mut DebugServer, mb: &mut Motherboard, query: &str) -> (u16, Value) {
        call(server, mb, "GET", &format!("/api/ram?{query}"), "")
    }

    #[test]
    fn memory_is_read_in_a_ram_bank_a_rom_or_through_a_paging() {
        let mut server = server();
        let mut mb = Motherboard::default();
        mb.write_bank_byte(4, 0x10, 0xaa);
        mb.write_bank_byte(1, 0x10, 0x55);
        mb.write_bank_byte(5, 0x3fff, 0x11);
        mb.write_bank_byte(6, 0, 0x22);
        // a RAM bank, whatever is paged; running on into the next one
        let (_, read) = ram_hex(&mut server, &mut mb, "at=B04:0010&len=1");
        assert_eq!(read["hex"], "aa", "{read}");
        assert_eq!(read["at"], "B04:0010");
        assert_eq!(ram_hex(&mut server, &mut mb, "at=P0B1:3fff&len=2").1["hex"], "1122");
        // the CPU's paging (C0: bank 1 at &4000), or another (C4: bank 4 there)
        assert_eq!(ram_hex(&mut server, &mut mb, "at=0x4010&len=1").1["hex"], "55");
        let (_, through) = ram_hex(&mut server, &mut mb, "at=0x4010&len=1&paging=C4");
        assert_eq!(through["hex"], "aa", "{through}");
        assert_eq!(through["paging"], "C4");
        // the older view=cpu takes a paging too
        assert_eq!(ram_hex(&mut server, &mut mb, "addr=0x4010&len=1&view=cpu&paging=C4").1["hex"], "aa");
        // the ROMs: the lower one at &0000, an upper one (an empty slot reads as BASIC's)
        let lower = mb.read_lower_rom_byte(0);
        assert_eq!(ram_hex(&mut server, &mut mb, "at=LOWER:0000&len=1").1["hex"], format!("{lower:02x}"));
        let basic = mb.upper_rom(0).unwrap()[0x10];
        assert_eq!(ram_hex(&mut server, &mut mb, "at=UPPER00:0010&len=1").1["hex"], format!("{basic:02x}"));
        assert_eq!(ram_hex(&mut server, &mut mb, "at=0xc010&len=1&paging=U0").1["hex"], format!("{basic:02x}"));
        // the RAM under the lower ROM: ROM off, or what a write reaches
        mb.write_bank_byte(0, 0, 0x42);
        assert_eq!(ram_hex(&mut server, &mut mb, "at=0&len=1&paging=-L").1["hex"], "42");
        assert_eq!(ram_hex(&mut server, &mut mb, "at=0&len=1&paging=W").1["hex"], "42");
        // refused: what is not there, a paging for a space, at with the older parameters
        for query in [
            "at=CART00:0000",
            "at=ASIC:0000",
            "at=B40:0000",
            "at=B04:0000&paging=C4",
            "at=0x4000&bank=1",
            "addr=0x4000&paging=C4",
            "at=0x4000&paging=C9",
        ] {
            let (status, refused) = ram_hex(&mut server, &mut mb, query);
            assert_eq!(status, 400, "{query}: {refused}");
        }
    }

    #[test]
    fn memory_is_written_where_a_write_goes() {
        let (mut server, mut mb) = (server(), machine());
        let (status, body) = call(&mut server, &mut mb, "POST", "/api/ram", r#"{"at":"B04:0010","data":"AB CD"}"#);
        assert_eq!(status, 200, "{body}");
        assert_eq!(mb.read_bank_byte(4, 0x10), Some(0xab));
        assert_eq!(mb.read_bank_byte(4, 0x11), Some(0xcd));
        // a CPU address through a paging: C4 puts bank 4 at &4000
        call(&mut server, &mut mb, "POST", "/api/ram", r#"{"at":"0x4020","paging":"C4","data":"77"}"#);
        assert_eq!(mb.read_bank_byte(4, 0x20), Some(0x77));
        assert_eq!(mb.read_bank_byte(1, 0x20), Some(0));
        // and jumps there
        call(&mut server, &mut mb, "POST", "/api/ram", r#"{"at":"0x4020","exec":true}"#);
        assert_eq!(mb.pc(), 0x4020);
        // a ROM is not written; exec at a bank's offset needs its PC
        let (status, refused) =
            call(&mut server, &mut mb, "POST", "/api/ram", r#"{"at":"UPPER07:0000","data":"00"}"#);
        assert_eq!(status, 400, "{refused}");
        assert_eq!(refused["field"], "at");
        let (status, _) = call(&mut server, &mut mb, "POST", "/api/ram", r#"{"at":"B04:0000","exec":true}"#);
        assert_eq!(status, 400);
        let (status, _) = call(&mut server, &mut mb, "POST", "/api/ram", r#"{"at":"B04:0000","addr":0,"data":"00"}"#);
        assert_eq!(status, 400);
    }

    #[test]
    fn the_memory_map_of_another_paging() {
        let (mut server, mut mb) = (server(), Motherboard::default());
        let (_, now) = call(&mut server, &mut mb, "GET", "/api/memmap", "");
        assert_eq!(now["regions"][0]["source"], "LOWER", "{now}");
        assert_eq!(now["regions"][1]["source"], "B01");
        let (_, other) = call(&mut server, &mut mb, "GET", "/api/memmap?paging=C4,-L", "");
        assert_eq!(other["regions"][0]["source"], "B00", "{other}");
        assert_eq!(other["regions"][1]["source"], "B04");
        assert_eq!(other["regions"][1]["ram_bank"], 4);
        assert_eq!(other["mmr"], 0xc4);
        assert_eq!(other["rmr"].as_u64().unwrap() & 4, 4, "the lower ROM's bit set: disabled");
    }

    #[test]
    fn a_cpc_plus_cartridge_page_and_its_registers_are_read() {
        let mut server = server();
        let mut mb = MotherboardBuilder::default()
            .set_model(crate::motherboard::CpcModel::Cpc6128Plus)
            .build();
        let page1 = mb.read_cartridge_byte(1, 0x20).unwrap();
        assert_eq!(ram_hex(&mut server, &mut mb, "at=CART01:0020&len=1").1["hex"], format!("{page1:02x}"));
        // the lower ROM is the cartridge page RMR2 selects (0 after the reset)
        let page0 = mb.read_cartridge_byte(0, 0x10).unwrap();
        assert_eq!(ram_hex(&mut server, &mut mb, "at=LOWER:0010&len=1").1["hex"], format!("{page0:02x}"));
        // a paging with page 1 as the lower ROM
        assert_eq!(ram_hex(&mut server, &mut mb, "at=0x20&len=1&paging=LP1").1["hex"], format!("{page1:02x}"));
        let (status, _) = ram_hex(&mut server, &mut mb, "at=ASIC:0000&len=16");
        assert_eq!(status, 200);
        let (status, _) = ram_hex(&mut server, &mut mb, "at=CART40:0000&len=1");
        assert_eq!(status, 400, "past the cartridge's pages");
    }

    #[test]
    fn an_empty_name_or_index_does_not_delete_everything() {
        let (mut server, mut mb) = (server(), machine());
        call(&mut server, &mut mb, "POST", "/api/counters", r#"{"name":"loop"}"#);
        call(&mut server, &mut mb, "POST", "/api/breakpoints", r#"{"addr":"0x4000"}"#);
        let (status, refused) = call(&mut server, &mut mb, "DELETE", "/api/counters?name=", "");
        assert_eq!(status, 400, "{refused}");
        assert_eq!(refused["field"], "name");
        let (status, _) = call(&mut server, &mut mb, "DELETE", "/api/breakpoints?index=", "");
        assert_eq!(status, 400);
        let (_, counters) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(counters["counters"].as_array().unwrap().len(), 1, "{counters}");
        let (_, breakpoints) = call(&mut server, &mut mb, "GET", "/api/breakpoints", "");
        assert_eq!(breakpoints["advanced"].as_array().unwrap().len(), 1, "{breakpoints}");
        // Left out, it is all of them.
        let (status, _) = call(&mut server, &mut mb, "DELETE", "/api/counters", "");
        assert_eq!(status, 200);
    }

    #[test]
    fn exec_breakpoints_are_added_and_removed_one_by_one() {
        let (mut server, mut mb) = (server(), machine());
        let list = |server: &mut DebugServer, mb: &mut Motherboard| {
            call(server, mb, "GET", "/api/breakpoints", "").1["z80_bp"].clone()
        };
        call(&mut server, &mut mb, "POST", "/api/z80_bp", "0x4000");
        call(&mut server, &mut mb, "POST", "/api/z80_bp?add=1", "0x4010, 0x4020");
        assert_eq!(list(&mut server, &mut mb).as_array().unwrap().len(), 3);
        let (status, _) = call(&mut server, &mut mb, "DELETE", "/api/z80_bp", "0x4010");
        assert_eq!(status, 200);
        assert_eq!(list(&mut server, &mut mb).as_array().unwrap().len(), 2);
        // Replacing still replaces.
        call(&mut server, &mut mb, "POST", "/api/z80_bp", "0x4030");
        assert_eq!(list(&mut server, &mut mb).as_array().unwrap().len(), 1);
        call(&mut server, &mut mb, "DELETE", "/api/z80_bp", "");
        assert_eq!(list(&mut server, &mut mb).as_array().unwrap().len(), 0);
    }

    #[test]
    fn register_pairs_are_set_by_their_halves() {
        let (mut server, mut mb) = (server(), machine());
        let (status, body) = call(&mut server, &mut mb, "POST", "/api/z80", r#"{"HL":"0x1234","BC2":48879}"#);
        assert_eq!(status, 200, "{body}");
        assert_eq!(mb.cpu().reg8(Reg8::H).get(), 0x12);
        assert_eq!(mb.cpu().reg8(Reg8::L).get(), 0x34);
        assert_eq!(mb.cpu().reg8(Reg8::B2).get(), 0xbe);
        assert_eq!(mb.cpu().reg8(Reg8::C2).get(), 0xef);
        let (status, _) = call(&mut server, &mut mb, "POST", "/api/z80", r#"{"DE":65536}"#);
        assert_eq!(status, 400, "too wide");
    }

    #[test]
    fn cpu_addresses_resolve_to_what_is_mapped() {
        let mut server = server();
        let mut mb = MotherboardBuilder::default().disable_upper_rom().build(); // lower ROM on
        let (_, body) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/z80_bp",
            "0x38,0x4002,B04:4001",
        );
        let names: Vec<_> = body["resolved"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["location"].clone())
            .collect();
        // B04 + 0x4001 carries into B05.
        assert_eq!(
            names,
            vec![json!("LOWER:0038"), json!("B01:0002"), json!("B05:0001")]
        );
        let (_, deprecated) = call(&mut server, &mut mb, "POST", "/api/z80_bp", "C4:0200");
        assert_eq!(deprecated["resolved"][0]["location"], "B04:0200");
        assert!(deprecated["warnings"].is_array());
    }

    #[test]
    fn a_physical_breakpoint_fires_only_on_its_bank() {
        let (mut server, mut mb) = (server(), machine());
        mb.write_buffer(0x4000, &[0x00, 0x00, 0x00, 0x18, 0xfb]); // NOPs; JR 0x4000
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        // 0x4002 is B01:0002 here: a breakpoint on B04:0002 must not fire.
        call(&mut server, &mut mb, "POST", "/api/z80_bp", "B04:0002");
        run(&mut server, &mut mb, 10);
        assert!(!server.paused());
        call(&mut server, &mut mb, "POST", "/api/z80_bp", "B01:0002");
        run(&mut server, &mut mb, 10);
        assert!(server.paused());
        assert_eq!(mb.pc(), 0x4002);
    }

    #[test]
    fn ram_banks_beyond_the_base_64k() {
        let (mut server, mut mb) = (server(), machine());
        assert!(mb.physical_bank_count() >= 8, "a 6128 has 64K of extra RAM");
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/ram",
            r#"{"bank":4,"addr":0,"data":"a5"}"#,
        );
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "GET",
                "/api/ram?bank=4&addr=0&len=1",
                ""
            )
            .1["hex"],
            "a5"
        );
        assert_ne!(
            call(&mut server, &mut mb, "GET", "/api/ram?addr=0&len=1", "").1["hex"],
            "a5"
        );
        // addr past 0x3FFF carries into the next bank.
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "GET",
                "/api/ram?bank=3&addr=0x4000&len=1",
                ""
            )
            .1["hex"],
            "a5"
        );
        let beyond = format!("/api/ram?bank={}", mb.physical_bank_count());
        assert_eq!(call(&mut server, &mut mb, "GET", &beyond, "").0, 400);
    }

    #[test]
    fn memory_write_breakpoints_stop_or_watch() {
        let (mut server, mut mb) = (server(), machine());
        let events = server.events.subscribe(None).unwrap();
        // LD A,0x81; LD (0xC005),A; NOP; JR -6
        mb.write_buffer(0x4000, &[0x3e, 0x81, 0x32, 0x05, 0xc0, 0x00, 0x18, 0xf8]);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        let (status, added) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/breakpoints",
            r#"{"kind":"mem","access":"w","run":"watch","addr":"0xC000","size":16,"name":"screen"}"#,
        );
        assert_eq!((status, added["index"].clone()), (200, json!(0)));
        run(&mut server, &mut mb, 2);
        assert!(!server.paused(), "watch only reports");
        let watch: Vec<_> = events_of(&events)
            .into_iter()
            .filter(|e| e.starts_with("event: watch"))
            .collect();
        assert_eq!(watch.len(), 1);
        assert!(
            watch[0].contains(r#""addr":"0xC005""#) && watch[0].contains(r#""name":"screen""#),
            "{}",
            watch[0]
        );

        // Now stop, written as rasm/basm store it in REMU.
        call(&mut server, &mut mb, "DELETE", "/api/breakpoints", "");
        let (status, _) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/breakpoints",
            r#"{"remu":"MEM,W,STOP,addr=49157,value=129,valmask=255"}"#,
        );
        assert_eq!(status, 200);
        run(&mut server, &mut mb, 4);
        assert!(server.paused());
        assert_eq!(mb.pc(), 0x4005, "stops after the writing instruction");
        let stops = events_of(&events);
        assert!(
            stops
                .iter()
                .any(|e| e.starts_with("event: z80_bp") && e.contains(r#""kind":"mem""#)),
            "{stops:?}"
        );
        assert!(
            stops
                .iter()
                .any(|e| e.starts_with("event: pause") && e.contains("true")),
            "{stops:?}"
        );
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/breakpoints", "");
        assert_eq!(list["advanced"][0]["addr"], 0xc005);
        assert_eq!(
            call(
                &mut server,
                &mut mb,
                "POST",
                "/api/breakpoints",
                r#"{"addr":1,"condition":"A=="}"#
            )
            .0,
            400
        );
    }

    #[test]
    fn conditions_select_the_hits() {
        let (mut server, mut mb) = (server(), machine());
        // INC A; LD (0xC000),A; JR -5 - writes 1, 2, 3...
        mb.write_buffer(0x4000, &[0x3c, 0x32, 0x00, 0xc0, 0x18, 0xfa]);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        let (status, _) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/breakpoints",
            r#"{"kind":"mem","access":"w","addr":"0xC000","condition":"value == 3 && A = 3 && peek(0xC000) = 3"}"#,
        );
        assert_eq!(status, 200);
        run(&mut server, &mut mb, 100);
        assert!(server.paused());
        assert_eq!(mb.read_byte(0xc000), 3);
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/breakpoints", "");
        assert_eq!(
            list["advanced"][0]["condition"],
            "value == 3 && A = 3 && peek(0xC000) = 3"
        );
    }

    #[test]
    fn reset_and_resume_events() {
        let (mut server, mut mb) = (server(), machine());
        let events = server
            .events
            .subscribe(Some(vec!["reset".to_owned()]))
            .unwrap();
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/config",
            r#"{"do_hard_reset":true,"paused":true}"#,
        );
        server.poll(&mut mb);
        // Filtered on "reset": the pause event is not delivered.
        assert_eq!(
            events_of(&events),
            vec!["event: reset\ndata: {\"hard\":true}\n\n".to_owned()]
        );
    }

    #[test]
    fn history_keeps_the_bytes_as_fetched() {
        let (mut server, mut mb) = (server(), machine());
        // LD A,7; XOR A; LD (0x4001),A - the last one zeroes the operand of the first.
        mb.write_buffer(0x4000, &[0x3e, 0x07, 0xaf, 0x32, 0x01, 0x40, 0x00]);
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/exec",
            r#"{"addr":16384}"#,
        );
        server.after_instruction(&mut mb); // what a loop does once before running
        run(&mut server, &mut mb, 3);
        let (_, history) = call(&mut server, &mut mb, "GET", "/api/history", "");
        let history = history.as_array().unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(
            (history[0]["pc"].clone(), history[0]["hex"].clone()),
            (json!(0x4000), json!("3E07AF32"))
        );
        assert_eq!(history[0]["current_hex"], "3E00AF32");
        assert_eq!(history[2]["pc"], 0x4003);
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/history?n=1", "")
                .1
                .as_array()
                .unwrap()
                .len(),
            1
        );
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/config",
            r#"{"history_size":1}"#,
        );
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/history", "")
                .1
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn screenshots_whole_live_and_composite() {
        use crate::monitor::{FULL_FRAME_HEIGHT, FULL_FRAME_WIDTH};
        let (mut server, mut mb) = (server(), MotherboardBuilder::default().build());
        let mut monitor = Monitor::default();
        let shot = |server: &mut DebugServer,
                    mb: &mut Motherboard,
                    monitor: Option<&Monitor>,
                    query: &str| {
            let query = query
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (k, v) = p.split_once('=').unwrap();
                    (k.to_owned(), v.to_owned())
                })
                .collect();
            let request = ApiRequest {
                method: "GET".into(),
                path: "/api/screenshot".into(),
                query,
                body: Vec::new(),
            };
            let response = server.handle_with(&request, mb, monitor);
            let picture = (response.status == 200)
                .then(|| image::load_from_memory(&response.body).unwrap().to_rgb8());
            (
                response.status,
                picture,
                response.header("X-Crop-X").map(str::to_owned),
            )
        };
        assert_eq!(
            shot(&mut server, &mut mb, Some(&monitor), "crop=0").0,
            503,
            "whole frames not kept"
        );
        monitor.keep_full_frames();
        // Three frames, the firmware's border drawn, then half of one.
        let mut frames = 0;
        let step = |server: &mut DebugServer, mb: &mut Motherboard, monitor: &mut Monitor| {
            let mut done = None;
            mb.execute_next_opcode_with_video(|(rgb, sync)| {
                monitor.receive(rgb, sync);
                if done.is_none() {
                    done = monitor.take_frame();
                }
            });
            let (x, line) = monitor.beam();
            let (crop_x, crop_y) = monitor.crop_origin();
            server.set_beam(Beam {
                x,
                line,
                crop_x,
                crop_y,
            });
            if let Some(frame) = done {
                server.set_frame(&frame);
                return true;
            }
            false
        };
        while frames < 3 {
            if step(&mut server, &mut mb, &mut monitor) {
                frames += 1;
            }
        }
        while monitor.beam().1 < 170 {
            step(&mut server, &mut mb, &mut monitor);
        }

        let (status, whole, origin) = shot(&mut server, &mut mb, Some(&monitor), "crop=0&live=0");
        let whole = whole.unwrap();
        assert_eq!(
            (status, whole.dimensions()),
            (200, (FULL_FRAME_WIDTH, FULL_FRAME_HEIGHT))
        );
        assert_eq!(origin.as_deref(), Some("0"));
        let visible = shot(&mut server, &mut mb, Some(&monitor), "").1.unwrap();
        assert_eq!(visible.dimensions(), (768, 272));
        // The visible area is inside the whole frame, where the crop says.
        let (cx, cy) = monitor.crop_origin();
        assert_eq!(
            *whole.get_pixel(cx as u32 + 10, cy as u32 + 100),
            *visible.get_pixel(10, 100)
        );

        // Live: the half drawn, black below the beam; over the previous frame
        // with full=0.
        let below = 250;
        let live = shot(&mut server, &mut mb, Some(&monitor), "crop=0&live=1")
            .1
            .unwrap();
        assert_eq!(
            *live.get_pixel(500, below),
            image::Rgb([0, 0, 0]),
            "not drawn yet"
        );
        let composite = shot(&mut server, &mut mb, Some(&monitor), "crop=0&live=1&full=0")
            .1
            .unwrap();
        assert_eq!(
            *composite.get_pixel(500, below),
            *whole.get_pixel(500, below),
            "the previous frame"
        );
        assert_eq!(
            *composite.get_pixel(500, 100),
            *live.get_pixel(500, 100),
            "what is drawn"
        );
        assert_ne!(
            *whole.get_pixel(500, below),
            image::Rgb([0, 0, 0]),
            "a border is drawn there"
        );

        // Without the monitor: the last frame, and no whole frame.
        assert_eq!(shot(&mut server, &mut mb, None, "crop=0").0, 503);
        assert_eq!(shot(&mut server, &mut mb, None, "live=1").0, 200);
        assert_eq!(shot(&mut server, &mut mb, None, "crop=2").0, 400);
    }

    #[test]
    fn screenshot_is_a_png_of_the_last_frame() {
        let (mut server, mut mb) = (server(), machine());
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/screenshot", "").0,
            503
        );
        server.set_frame(&image::RgbImage::from_pixel(
            768,
            272,
            image::Rgb([0, 0, 128]),
        ));
        let request = ApiRequest {
            method: "GET".into(),
            path: "/api/screenshot".into(),
            query: vec![],
            body: Vec::new(),
        };
        let response = server.handle(&request, &mut mb);
        assert_eq!((response.status, response.content_type), (200, "image/png"));
        assert_eq!(response.header("X-Crop-W"), Some("768"));
        let png = image::load_from_memory(&response.body).unwrap().to_rgb8();
        assert_eq!(*png.get_pixel(10, 10), image::Rgb([0, 0, 128]));
    }

    fn upload(
        server: &mut DebugServer,
        mb: &mut Motherboard,
        query: &str,
        body: &[u8],
    ) -> (u16, Value) {
        let query = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap();
                (k.to_owned(), v.to_owned())
            })
            .collect();
        let request = ApiRequest {
            method: "POST".into(),
            path: "/api/media".into(),
            query,
            body: body.to_vec(),
        };
        let response = server.handle(&request, mb);
        (
            response.status,
            serde_json::from_slice(&response.body).unwrap(),
        )
    }

    #[test]
    fn media_binaries_with_and_without_header() {
        let (mut server, mut mb) = (server(), machine());
        let (status, reply) = upload(
            &mut server,
            &mut mb,
            "name=game@4000@4001.bin",
            &[0xc9, 0x00],
        );
        assert_eq!(
            (status, reply["load"].clone(), reply["len"].clone()),
            (200, json!(0x4000), json!(2))
        );
        assert_eq!((mb.read_byte(0x4000), mb.pc()), (0xc9, 0x4001));

        let mut amsdos = vec![0u8; 128];
        amsdos[21..23].copy_from_slice(&0x8000u16.to_le_bytes());
        amsdos[24..26].copy_from_slice(&3u16.to_le_bytes());
        amsdos[26..28].copy_from_slice(&0x8001u16.to_le_bytes());
        let sum: u16 = amsdos[..67].iter().map(|&b| b as u16).sum();
        amsdos[67..69].copy_from_slice(&sum.to_le_bytes());
        amsdos.extend_from_slice(&[1, 2, 3, 4]);
        let (status, reply) = upload(&mut server, &mut mb, "name=prog.bin", &amsdos);
        assert_eq!((status, reply["entry"].clone()), (200, json!(0x8001)));
        assert_eq!(
            (
                mb.read_byte(0x8000),
                mb.read_byte(0x8002),
                mb.read_byte(0x8003)
            ),
            (1, 3, 0)
        );
        assert_eq!(mb.pc(), 0x4001, "no @ENTRY in the name: no jump");

        assert_eq!(upload(&mut server, &mut mb, "name=raw.bin", &[0]).0, 400);
        assert_eq!(upload(&mut server, &mut mb, "name=x.cdt", &[0]).0, 400);
        let cdt = crate::tape::cdt_of_binary("x", 0x4000, 0x4000, &[0xc9], 2000);
        assert_eq!(
            upload(&mut server, &mut mb, "name=x.cdt", &cdt).1["media"],
            json!("cdt")
        );
        assert!(mb.tape_deck().is_some_and(|deck| deck.playing()));
        assert_eq!(upload(&mut server, &mut mb, "", &[]).0, 400);
    }

    #[test]
    fn media_snapshot_sets_its_breakpoints() {
        use cpclib::sna::*;
        let (mut server, mut mb) = (server(), machine());
        let mut snapshot = Snapshot::default();
        snapshot.set_value(SnapshotFlag::Z80_PC, 0x1234).unwrap();
        let mut remu = RemuChunk::empty();
        remu.add_entry(&RemuEntry::new_breakpoint(0x4000, 0));
        snapshot.add_chunk(remu);
        let mut bytes = Vec::new();
        snapshot.write_all(&mut bytes, SnapshotVersion::V3).unwrap();
        let (status, reply) = upload(&mut server, &mut mb, "name=save.sna", &bytes);
        assert_eq!(
            (status, reply["media"].clone(), reply["breakpoints"].clone()),
            (200, json!("sna"), json!(1))
        );
        assert_eq!(mb.pc(), 0x1234);
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/breakpoints", "").1["z80_bp"],
            json!(["B01:0000"])
        );
    }

    #[test]
    fn a_snapshot_names_the_breakpoints_and_the_history() {
        use cpclib::sna::*;
        let (mut server, mut mb) = (server(), machine());
        let mut snapshot = Snapshot::default();
        let mut remu = RemuChunk::empty();
        remu.add_entry(&RemuEntry::new_label("main".to_owned(), 0x4000, 0));
        snapshot.add_chunk(remu);
        let mut symb = AceSymbolChunk::empty();
        symb.add_symbol(AceSymbol::new(
            "loop",
            0x4002,
            AceMemMapType::Undefined,
            AceSymbolType::Absolute,
        ));
        snapshot.add_chunk(symb);
        let mut bytes = Vec::new();
        snapshot.write_all(&mut bytes, SnapshotVersion::V3).unwrap();
        let (status, reply) = upload(&mut server, &mut mb, "name=labels.sna", &bytes);
        assert_eq!((status, reply["symbols"].clone()), (200, json!(2)));
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/symbols", "").1,
            json!([{ "name": "main", "address": 0x4000 }, { "name": "loop", "address": 0x4002 }])
        );

        // nop : nop : loop: jr loop
        mb.write_buffer(0x4000, &[0x00, 0x00, 0x18, 0xfe]);
        mb.cpu_mut().set_pc(0x4000);
        let (status, reply) = call(&mut server, &mut mb, "POST", "/api/z80_bp", "loop, main+1");
        assert_eq!(status, 200, "{reply}");
        assert_eq!(
            call(&mut server, &mut mb, "GET", "/api/breakpoints", "").1["z80_bp"],
            json!(["B01:0001", "B01:0002"])
        );
        assert_eq!(
            call(&mut server, &mut mb, "POST", "/api/z80_bp", "nowhere").0,
            400
        );

        call(&mut server, &mut mb, "POST", "/api/z80_bp", "");
        server.after_instruction(&mut mb);
        run(&mut server, &mut mb, 3);
        let history = call(&mut server, &mut mb, "GET", "/api/history", "").1;
        let labels: Vec<Value> = history
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["label"].clone())
            .collect();
        assert_eq!(labels, vec![json!("main"), json!("main+1"), json!("loop")]);
    }

    #[test]
    fn a_saved_snapshot_goes_back_through_media() {
        let (mut server, mut mb) = (server(), machine());
        mb.cpu_mut().set_pc(0x4321);
        let request = ApiRequest {
            method: "GET".into(),
            path: "/api/snapshot".into(),
            query: vec![],
            body: Vec::new(),
        };
        let response = server.handle(&request, &mut mb);
        assert_eq!(
            (response.status, response.content_type),
            (200, "application/octet-stream")
        );
        let mut other = machine();
        let (status, reply) = upload(&mut server, &mut other, "name=saved.sna", &response.body);
        assert_eq!((status, reply["media"].clone()), (200, json!("sna")));
        assert_eq!(other.pc(), 0x4321);
    }

    #[test]
    fn media_disc_image_goes_to_the_drive() {
        let (mut server, mut mb) = (server(), machine());
        let dsk = std::fs::read("tests/fixtures/disctest_data_errors.dsk").unwrap();
        let (status, reply) = upload(&mut server, &mut mb, "name=a.dsk&drive=1", &dsk);
        assert_eq!((status, reply["media"].clone()), (200, json!("dsk")));
        assert!(mb.fdc().image(crate::fdc::FdcDrive::B).is_some());
        assert_eq!(upload(&mut server, &mut mb, "drive=2", &dsk).0, 400);
    }

    #[test]
    fn query_values_are_percent_decoded() {
        assert_eq!(percent_decode("game%40%34000.bin"), "game@4000.bin");
        assert_eq!(percent_decode("a+b%2"), "a b%2");
        assert_eq!(percent_decode("%é9"), "%é9");
    }

    #[test]
    fn quit_is_reported_to_the_loop() {
        let (mut server, mut mb) = (server(), machine());
        assert!(!server.quit_requested());
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/quit", "").0, 200);
        assert!(server.quit_requested());
    }

    #[test]
    fn keytype_reports_autotyping() {
        let (mut server, mut mb) = (server(), machine());
        assert_eq!(
            call(&mut server, &mut mb, "POST", "/api/keytype", r#"{}"#).0,
            400
        );
        call(
            &mut server,
            &mut mb,
            "POST",
            "/api/keytype",
            r#"{"text":"RUN\r"}"#,
        );
        let (_, ping) = call(&mut server, &mut mb, "GET", "/api/ping", "");
        assert_eq!(
            (
                ping["emu"]["autotyping"].clone(),
                ping["emu"]["autotype_remaining"].clone()
            ),
            (json!(true), json!(4))
        );
    }

    #[test]
    fn requests_reach_the_machine_through_poll() {
        let (sender, receiver) = mpsc::channel();
        let mut server = DebugServer::with_requests(receiver);
        let mut mb = machine();
        let (reply_sender, reply) = mpsc::channel();
        let request = ApiRequest {
            method: "GET".into(),
            path: "/api/state".into(),
            query: vec![],
            body: Vec::new(),
        };
        sender.send((request, reply_sender)).unwrap();
        server.poll(&mut mb);
        let state: Value = serde_json::from_slice(&reply.recv().unwrap().body).unwrap();
        assert_eq!(state["crtc"]["regs"].as_array().unwrap().len(), 14);
        assert_eq!(state["ga"]["ink_rgb"].as_array().unwrap().len(), 16);
    }
}
