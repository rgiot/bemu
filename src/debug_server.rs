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
//! Implemented so far: `ping`, `state`, `z80` (GET/POST), `ram` (GET/POST,
//! every 16K bank), `exec`, `step`, `config` (GET/POST: pause/resume,
//! resets), `z80_bp` (CPU addresses resolved to the RAM bank or ROM holding
//! them, `Bnn:hhhh` physical locations), `keytype`, `events` (Server-Sent
//! Events: `frame`, `z80_bp`, `pause`, `reset`, `basic_bp`), `doc`, the chips
//! (`memmap`, `crtc`, `ga`, `psg`, `fdc`), BASIC (`basic`, `basic_state`,
//! `basic_listing`, `basic_export`, `basic_bp`, `basic_step`, `basic_runto`,
//! see [`crate::basic_program`]) and the beam (`beam`, `raster_bp`, in
//! AMSpiriT's video buffer coordinates or in NOPs: 64 a line, 19968 a
//! frame).
//!
//! Extensions to the AMSpiriT API - listed in `/api/doc` with
//! `"extension":true` - add what debuggers of rasm/basm/ACE programs need:
//! `/api/breakpoints` for advanced breakpoints (memory/IO watchpoints,
//! ranges and masks, stop or watch, conditions, see
//! [`crate::breakpoints`]), the
//! `watch` event for watch hits, `kind`/`name`/`access`/`beam`/`counters`
//! in `z80_bp` events, and `/api/counters`: named counters of the NOPs
//! run. Replies identify this emulator with `"frontend":"bemu"`.

mod api_doc;

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::basic_program::{self, BasicState};
use crate::breakpoints::{AdvancedBreakpoint, BreakpointAccess, BreakpointKind, Condition, ConditionContext, CounterAction, RunMode};
use crate::csl_runner::keys_of_char;
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
    pub body: Vec<u8>
}

impl ApiRequest {
    /// The body as text (JSON, breakpoint lists).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// The answer to an [`ApiRequest`].
#[derive(Debug, PartialEq)]
pub struct ApiResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// Extra headers (`Allow` of a 405, screenshot geometry).
    pub headers: Vec<(String, String)>
}

impl ApiResponse {
    fn json(status: u16, value: Value) -> Self {
        Self { status, content_type: "application/json", body: value.to_string().into_bytes(), headers: Vec::new() }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn ok(value: Value) -> Self {
        Self::json(200, value)
    }
}

/// A request the endpoint cannot use: `400 {"error", "field"}`.
#[derive(Debug)]
struct ApiError {
    error: String,
    field: Option<String>
}

impl ApiError {
    fn new(error: impl Into<String>) -> Self {
        Self { error: error.into(), field: None }
    }

    fn field(field: &str, error: impl Into<String>) -> Self {
        Self { error: error.into(), field: Some(field.to_owned()) }
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
    extension: bool
}

const ROUTES: &[Route] = &[
    Route { path: "/api/ping", methods: &["GET"], summary: "connectivity probe, with the emu object", extension: false },
    Route { path: "/api/state", methods: &["GET"], summary: "z80, ga, crtc, psg and emu state", extension: false },
    Route { path: "/api/z80", methods: &["GET", "POST"], summary: "read / write the Z80 registers", extension: false },
    Route { path: "/api/ram", methods: &["GET", "POST"], summary: "read / write RAM (bank, view=raw|cpu), optional jump", extension: false },
    Route { path: "/api/exec", methods: &["POST"], summary: "set PC", extension: false },
    Route { path: "/api/step", methods: &["POST"], summary: "execute one instruction and pause", extension: false },
    Route { path: "/api/config", methods: &["GET", "POST"], summary: "pause/resume, soft/hard reset", extension: false },
    Route { path: "/api/z80_bp", methods: &["POST"], summary: "replace the PC breakpoints: CPU addresses or Bnn:hhhh, comma-separated", extension: false },
    Route { path: "/api/keytype", methods: &["POST"], summary: "type text on the CPC keyboard (\\r = RETURN)", extension: false },
    Route { path: "/api/events", methods: &["GET"], summary: "Server-Sent Events: frame, z80_bp, pause, reset, watch (?topics=)", extension: false },
    Route { path: "/api/breakpoints", methods: &["GET", "POST", "DELETE"], summary: "advanced breakpoints: list, add (JSON or REMU acebreak text), remove (?index=)", extension: true },
    Route { path: "/api/quit", methods: &["POST"], summary: "leave the emulator", extension: false },
    Route { path: "/api/history", methods: &["GET"], summary: "last executed instructions (?n=), bytes as fetched and as now", extension: false },
    Route { path: "/api/screenshot", methods: &["GET"], summary: "frame as PNG: crop=1 visible 768x272 | crop=0 whole 1024x350; live=1 the frame being drawn; full=0 over the previous one", extension: false },
    Route { path: "/api/snapshot", methods: &["GET"], summary: "the machine's state as a version 3 .sna file (save it, load it back with /api/media)", extension: true },
    Route { path: "/api/media", methods: &["POST"], summary: "load a .sna/.dsk (by content) or .bin/.amsdos (?name=, ?drive=)", extension: false },
    Route { path: "/api/memmap", methods: &["GET"], summary: "what is mapped in each 16K region (ROM, absolute RAM bank), RMR, MMR", extension: false },
    Route { path: "/api/crtc", methods: &["GET"], summary: "CRTC registers, selected register and counters", extension: false },
    Route { path: "/api/ga", methods: &["GET"], summary: "Gate Array: mode, pens, inks, RMR and banking", extension: false },
    Route { path: "/api/psg", methods: &["GET"], summary: "PSG registers, decoded", extension: false },
    Route { path: "/api/fdc", methods: &["GET"], summary: "FDC status registers, phase, motor and heads", extension: false },
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
    Address(u16)
}

/// A program to load once the firmware booted after a reset.
struct PendingBasic {
    source: String,
    run: bool,
    at_t_states: usize
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
    pub crop_y: usize
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

    fn json(&self) -> Value {
        json!({
            "x": self.x,
            "y": self.line,
            "nop": self.nop(),
            "line": self.line,
            "frame_nop": self.line * NOPS_PER_LINE + self.nop()
        })
    }
}

/// A beam breakpoint: a position in the frame, or (no line) a position in
/// every line.
#[derive(Clone, Debug)]
struct RasterBreakpoint {
    /// The line; `None`: every line.
    line: Option<usize>,
    /// In monitor ticks.
    x: usize
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
                }
                else {
                    // A new frame started.
                    target > before || target <= after
                }
            },
            None if after.line == before.line && after.x >= before.x => before.x < self.x && self.x <= after.x,
            None => self.x > before.x || self.x <= after.x
        }
    }

    fn json(&self, index: usize) -> Value {
        let mut value = json!({ "index": index, "x": self.x, "nop": self.x / BEAM_TICKS_PER_NOP });
        if let Some(line) = self.line {
            value["y"] = json!(line);
            value["line"] = json!(line);
            value["frame_nop"] = json!(line * NOPS_PER_LINE + self.x / BEAM_TICKS_PER_NOP);
        }
        value
    }
}

/// A named counter of emulated NOPs (T-states / 4), running or stopped.
#[derive(Clone, Debug)]
struct NopCounter {
    t_states: u64,
    running: bool
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
    pending_basic: Option<PendingBasic>
}

/// AMSpiriT's default depth of `GET /api/history`.
const DEFAULT_HISTORY_SIZE: usize = 20;

/// One SSE client: the events waiting to be written to it.
struct SseClient {
    topics: Option<Vec<String>>,
    queue: Arc<(Mutex<SseQueue>, Condvar)>
}

#[derive(Default)]
struct SseQueue {
    events: VecDeque<String>,
    closed: bool
}

/// The Server-Sent Events clients, shared by the HTTP thread (which adds
/// them) and the emulation thread (which publishes).
#[derive(Clone, Default)]
pub struct EventHub {
    clients: Arc<Mutex<Vec<SseClient>>>
}

/// AMSpiriT's limits: 8 clients, 32 queued events each (oldest dropped).
const MAX_SSE_CLIENTS: usize = 8;
const MAX_SSE_QUEUE: usize = 32;
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);
/// `frame` events are sent every 10 frames (~5Hz).
const FRAME_EVENT_EVERY: u64 = 10;

impl EventHub {
    fn has_clients(&self) -> bool {
        !self.clients.lock().unwrap().is_empty()
    }

    fn publish(&self, topic: &str, data: &Value) {
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|client| !client.queue.0.lock().unwrap().closed);
        let mut message = None;
        for client in clients.iter() {
            if client.topics.as_ref().is_some_and(|topics| !topics.iter().any(|t| t == topic)) {
                continue;
            }
            let message = message.get_or_insert_with(|| format!("event: {topic}\ndata: {data}\n\n"));
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
        clients.push(SseClient { topics, queue: Arc::clone(&queue) });
        Some(queue)
    }
}

impl DebugServer {
    /// Listen on `127.0.0.1:port` (loopback only).
    pub fn start(port: u16) -> Result<Self, String> {
        let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| format!("debug server on port {port}: {e}"))?;
        let (sender, requests) = mpsc::channel();
        let debug_server = Self::with_requests(requests);
        let events = debug_server.events.clone();
        std::thread::Builder::new()
            .name("debug-server".to_owned())
            .spawn(move || serve(server, sender, events))
            .map_err(|e| e.to_string())?;
        Ok(debug_server)
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
            fetched: None,
            last_frame: None,
            beam: None,
            previous_beam: None,
            raster_breakpoints: Vec::new(),
            counters: Default::default(),
            counted_t_states: 0,
            basic_breakpoints: Default::default(),
            basic_target: None,
            pending_basic: None
        }
    }

    /// Whether the emulation must stay stopped.
    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Whether `POST /api/quit` asked the emulator to leave.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Hand the last complete frame over, for `GET /api/screenshot`.
    pub fn set_frame(&mut self, frame: &image::RgbImage) {
        match &mut self.last_frame {
            Some(last) if last.dimensions() == frame.dimensions() => last.copy_from_slice(frame),
            last => *last = Some(frame.clone())
        }
    }

    /// Where the monitor's beam is: to call before
    /// [`Self::after_instruction`] for `/api/beam` and the raster
    /// breakpoints.
    pub fn set_beam(&mut self, beam: Beam) {
        self.previous_beam = self.beam.replace(beam);
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
            self.events.publish("pause", &json!({ "paused": self.paused }));
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
        if !self.exec_breakpoints.is_empty() && self.exec_breakpoints.contains(&motherboard.physical_address(pc)) {
            self.stop(motherboard, json!({ "kind": "exec" }));
        }
        for index in 0..self.advanced.len() {
            if self.advanced[index].hit_by_execution(pc) {
                self.hit(index, None, motherboard);
            }
        }
        if (!self.basic_breakpoints.is_empty() || self.basic_target.is_some()) && basic_program::at_statement(motherboard) {
            self.basic_statement(motherboard);
        }
        if self.pending_basic.as_ref().is_some_and(|p| motherboard.t_state_counter().value() >= p.at_t_states) {
            let pending = self.pending_basic.take().unwrap();
            self.load_basic(&pending.source, pending.run, motherboard);
        }
        if let (Some(before), Some(after)) = (self.previous_beam.take(), self.beam) {
            if self.raster_breakpoints.iter().any(|brk| brk.passed(before, after)) {
                self.stop(motherboard, json!({ "kind": "raster" }));
            }
        }
        if stepped {
            self.paused = true;
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
            if !condition.holds(&MachineContext { motherboard, access, beam: self.beam }) {
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
        let mut info = json!({ "kind": kind_name(brk.kind), "index": index, "hits": brk.hits });
        if let Some(name) = &brk.name {
            info["name"] = json!(name);
        }
        if let Some(access) = access {
            info["access"] = json!({
                "type": match access.kind {
                    BusAccessKind::MemoryRead | BusAccessKind::IoRead => "read",
                    BusAccessKind::MemoryWrite | BusAccessKind::IoWrite => "write"
                },
                "addr": hex16(access.address),
                "value": access.value
            });
        }
        match brk.run {
            RunMode::Stop => self.stop(motherboard, info),
            RunMode::Watch => {
                let mut event = registers_event(motherboard);
                if !self.counters.is_empty() {
                    event["counters"] = self.counters_json();
                }
                merge(&mut event, info);
                self.events.publish("watch", &event);
            },
            RunMode::Count => {}
        }
    }

    /// A breakpoint's counter action (the instruction just executed is
    /// already counted).
    fn act_on_counter(&mut self, action: CounterAction, name: &str) {
        let counter = match (action, self.counters.get_mut(name)) {
            (_, Some(counter)) => counter,
            (CounterAction::Stop, None) => return,
            (_, None) => self.counters.entry(name.to_owned()).or_insert(NopCounter { t_states: 0, running: false })
        };
        match action {
            CounterAction::Start => counter.running = true,
            CounterAction::Stop => counter.running = false,
            CounterAction::Reset => counter.t_states = 0
        }
    }

    fn stop(&mut self, motherboard: &Motherboard, info: Value) {
        self.paused = true;
        self.last_stop = Some(motherboard.pc());
        let mut event = registers_event(motherboard);
        event["ticks"] = json!(motherboard.t_state_counter().value() / T_STATES_PER_NOP);
        if let Some(beam) = self.beam {
            event["beam"] = beam.json();
        }
        if !self.counters.is_empty() {
            event["counters"] = self.counters_json();
        }
        merge(&mut event, info);
        self.events.publish("z80_bp", &event);
    }

    /// At the start of a BASIC statement: stop on a line breakpoint, at the
    /// end of a step, at a run-to target.
    fn basic_statement(&mut self, motherboard: &Motherboard) {
        let state = BasicState::read(motherboard);
        let Some(line) = state.line
        else {
            return; // direct mode
        };
        let reason = match self.basic_target {
            Some(BasicTarget::Statement) => Some("step"),
            Some(BasicTarget::LineAfter(from)) if from != Some(line) || state.first_statement => Some("step"),
            Some(BasicTarget::Line(target)) if target == line && state.first_statement => Some("runto"),
            Some(BasicTarget::Address(target)) if target == state.stmt_addr => Some("runto"),
            _ if state.first_statement && self.basic_breakpoints.contains(&line) => Some("breakpoint"),
            _ => None
        };
        if let Some(reason) = reason {
            self.basic_target = None;
            self.paused = true;
            self.last_stop = Some(motherboard.pc());
            let mut event = json!({ "line": line, "addr": hex16(state.stmt_addr), "reason": reason });
            if !self.counters.is_empty() {
                event["counters"] = self.counters_json();
            }
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

    fn counters_json(&self) -> Value {
        Value::Object(
            self.counters
                .iter()
                .map(|(name, counter)| (name.clone(), json!(counter.t_states / T_STATES_PER_NOP as u64)))
                .collect()
        )
    }

    /// To call once per completed video frame.
    pub fn frame_completed(&mut self, motherboard: &Motherboard) {
        self.frames += 1;
        if self.frames % FRAME_EVENT_EVERY == 0 && self.events.has_clients() {
            let mut event = registers_event(motherboard);
            merge(&mut event, json!({ "fps": self.fps, "frame": self.frames, "paused": self.paused }));
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

    /// Route `request`.
    pub fn handle(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResponse {
        self.handle_with(request, motherboard, None)
    }

    /// [`Self::handle`], with the monitor (see [`Self::poll_with`]).
    pub fn handle_with(
        &mut self,
        request: &ApiRequest,
        motherboard: &mut Motherboard,
        monitor: Option<&Monitor>
    ) -> ApiResponse {
        let Some(route) = ROUTES.iter().find(|route| route.path == request.path)
        else {
            return ApiResponse::json(404, json!({ "error": "not found", "path": request.path }));
        };
        if !route.methods.contains(&request.method.as_str()) {
            let mut response = ApiResponse::json(405, json!({ "error": "method not allowed", "allow": route.methods }));
            response.headers.push(("Allow".to_owned(), route.methods.join(", ")));
            return response;
        }
        let result = match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/ping") => Ok(ApiResponse::ok(json!({
                "ok": true,
                "emu": self.emu(motherboard),
                "frontend": "bemu",
                "version": env!("CARGO_PKG_VERSION")
            }))),
            ("GET", "/api/state") => Ok(ApiResponse::ok(self.state(motherboard))),
            ("GET", "/api/z80") => Ok(ApiResponse::ok(z80(motherboard))),
            ("POST", "/api/z80") => self.set_z80(request, motherboard),
            ("GET", "/api/ram") => read_ram(request, motherboard),
            ("POST", "/api/ram") => self.write_ram(request, motherboard),
            ("POST", "/api/exec") => self.exec(request, motherboard),
            ("POST", "/api/step") => {
                self.paused = false;
                self.stepping = true;
                Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
            },
            ("GET", "/api/config") => Ok(ApiResponse::ok(json!({
                "crtc_type": motherboard.crtc().crtc_type() as u8,
                "paused": self.paused,
                "history_size": self.history_size
            }))),
            ("POST", "/api/config") => self.config(request, motherboard),
            ("POST", "/api/z80_bp") => self.set_breakpoints(request, motherboard),
            ("GET", "/api/breakpoints") => Ok(ApiResponse::ok(self.list_breakpoints())),
            ("POST", "/api/breakpoints") => self.add_breakpoint(request, motherboard),
            ("DELETE", "/api/breakpoints") => self.remove_breakpoints(request, motherboard),
            // Answered by the HTTP thread (see `serve`): only reached
            // through `handle` directly.
            ("GET", "/api/events") => Err(ApiError::new("events are streamed by the HTTP server")),
            ("POST", "/api/keytype") => self.keytype(request),
            ("GET", "/api/doc") => doc(request),
            ("POST", "/api/quit") => {
                self.quit = true;
                Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
            },
            ("GET", "/api/history") => self.history_list(request, motherboard),
            ("GET", "/api/screenshot") => self.screenshot(request, monitor),
            ("POST", "/api/media") => self.media(request, motherboard),
            ("GET", "/api/snapshot") => crate::snapshot::save_sna_bytes(motherboard).map_err(ApiError::new).map(|body| ApiResponse {
                status: 200,
                content_type: "application/octet-stream",
                body,
                headers: vec![("Content-Disposition".into(), "attachment; filename=\"bemu.sna\"".into())]
            }),
            ("GET", "/api/memmap") => Ok(ApiResponse::ok(memmap(motherboard))),
            ("GET", "/api/crtc") => Ok(ApiResponse::ok(self.crtc(motherboard))),
            ("GET", "/api/ga") => Ok(ApiResponse::ok(ga(motherboard))),
            ("GET", "/api/psg") => Ok(ApiResponse::ok(psg(motherboard))),
            ("GET", "/api/fdc") => Ok(ApiResponse::ok(fdc(motherboard))),
            ("GET", "/api/beam") => self.beam_position(request),
            ("GET", "/api/raster_bp") => Ok(ApiResponse::ok(self.list_raster_breakpoints())),
            ("POST", "/api/raster_bp") => self.set_raster_breakpoint(request),
            ("GET", "/api/counters") => Ok(ApiResponse::ok(self.list_counters())),
            ("POST", "/api/counters") => self.counter_command(request, motherboard),
            ("POST", "/api/basic") => self.basic_load_request(request, motherboard),
            ("GET", "/api/basic_state") => Ok(ApiResponse::ok(basic_state(motherboard))),
            ("GET", "/api/basic_export") => Ok(ApiResponse {
                status: 200,
                content_type: "text/plain; charset=utf-8",
                body: basic_program::program(motherboard).map(|p| p.to_string()).unwrap_or_default().into_bytes(),
                headers: Vec::new()
            }),
            ("GET", "/api/basic_listing") => basic_listing(motherboard),
            ("POST", "/api/basic_step") => {
                let target = match request.param("mode") {
                    Some("line") => BasicTarget::LineAfter(BasicState::read(motherboard).line),
                    None | Some("") | Some("stmt") | Some("statement") => BasicTarget::Statement,
                    Some(other) => return ApiError::field("mode", format!("unknown mode \"{other}\": line or statement")).response()
                };
                Ok(self.run_basic_to(target))
            },
            ("GET", "/api/basic_bp") => Ok(ApiResponse::ok(json!({ "lines": self.basic_breakpoints }))),
            ("POST", "/api/basic_bp") => self.set_basic_breakpoints(request),
            ("POST", "/api/basic_runto") => {
                let number = |name: &str| request.param(name).map(|v| parse_int(v).filter(|v| (0..=0xffff).contains(v)));
                match (number("line"), number("addr")) {
                    (Some(Some(line)), _) => Ok(self.run_basic_to(BasicTarget::Line(line as u16))),
                    (None, Some(Some(addr))) => Ok(self.run_basic_to(BasicTarget::Address(addr as u16))),
                    (Some(None), _) => Err(ApiError::field("line", "line must be a number from 0 to 65535")),
                    (None, Some(None)) => Err(ApiError::field("addr", "addr must be a number from 0 to 65535")),
                    (None, None) => Err(ApiError::new("missing ?line= or ?addr="))
                }
            },
            ("DELETE", "/api/counters") => self.delete_counters(request),
            _ => unreachable!("{} {}", request.method, request.path)
        };
        result.unwrap_or_else(ApiError::response)
    }

    // ------------------------------------------------------------ state

    fn emu(&self, motherboard: &Motherboard) -> Value {
        json!({
            "fps": self.fps,
            "frames": self.frames,
            "ticks": motherboard.t_state_counter().value() / T_STATES_PER_NOP,
            "paused": self.paused,
            "cpc_model": motherboard.model().name(),
            "crtc_type": motherboard.crtc().crtc_type() as u8,
            "autotyping": !self.typing.is_empty() || self.typed_pressed.is_some(),
            "autotype_remaining": self.typing.len(),
            "ram_apply_seq": self.ram_seq,
            "applied_cmd_seq": self.applied_cmd_seq
        })
    }

    fn state(&self, motherboard: &Motherboard) -> Value {
        json!({
            "z80": z80(motherboard),
            "ga": ga(motherboard),
            "crtc": self.crtc(motherboard),
            "psg": psg(motherboard),
            "fdc": fdc(motherboard),
            "beam": self.beam.map(|beam| beam_json(beam)),
            "emu": self.emu(motherboard)
        })
    }

    fn crtc(&self, motherboard: &Motherboard) -> Value {
        let crtc = motherboard.crtc();
        let mut value = json!({
            "type": crtc.crtc_type() as u8,
            "regs": (0..14).map(|r| crtc.do_read_register(r)).collect::<Vec<_>>(),
            "selected_reg": crtc.selected_register(),
            // Extension: the counters (C0, C4, C9, VSYNC, R5 adjust) and
            // the address the CRTC outputs.
            "hcc": crtc.hcc,
            "vcc": crtc.vcc.value(),
            "vlc": crtc.vlc.value(),
            "vsc": crtc.vsc,
            "vtac": crtc.vtac,
            "ma": crtc.vma()
        });
        if let Some(beam) = self.beam {
            value["rasterline"] = json!(beam.line);
        }
        value
    }

    // ------------------------------------------------------------- beam

    fn beam_position(&self, request: &ApiRequest) -> ApiResult {
        let beam = self.beam.ok_or_else(|| ApiError::new("no beam position: the emulation is not driving a monitor"))?;
        let crop = request.param("crop").is_some_and(|v| v != "0");
        let mut value = beam.json();
        if crop {
            value["x"] = json!(beam.x as isize - beam.crop_x);
            value["y"] = json!(beam.line as isize - beam.crop_y as isize);
        }
        merge(&mut value, beam_crop(beam));
        merge(&mut value, json!({ "nops_per_line": NOPS_PER_LINE, "nops_per_frame": NOPS_PER_LINE * LINES_PER_FRAME }));
        Ok(ApiResponse::ok(value))
    }

    fn list_raster_breakpoints(&self) -> Value {
        json!({
            "breakpoints": self.raster_breakpoints.iter().enumerate().map(|(i, b)| b.json(i)).collect::<Vec<_>>()
        })
    }

    /// `POST /api/raster_bp`: AMSpiriT's `x`/`y` (beam ticks and lines,
    /// `crop=1` for picture coordinates), or the position in NOPs: `nop`
    /// (0-63) in `line` (0-311), `frame_nop` (0-19967); `nop` alone stops
    /// in every line, `line` alone at its start.
    fn set_raster_breakpoint(&mut self, request: &ApiRequest) -> ApiResult {
        let param = |name: &str, max: usize| -> Result<Option<usize>, ApiError> {
            match request.param(name) {
                None | Some("") => Ok(None),
                Some(text) => parse_int(text)
                    .filter(|&v| (0..=max as i64).contains(&v))
                    .map(|v| Some(v as usize))
                    .ok_or_else(|| ApiError::field(name, format!("{name} must be a number from 0 to {max}")))
            }
        };
        let enable = param("enable", 1)?.unwrap_or(1) == 1;
        let add = param("add", 1)?.unwrap_or(0) == 1;
        let crop = param("crop", 1)?.unwrap_or(0) == 1;
        let x = param("x", 0xffff)?;
        let y = param("y", 0xffff)?;
        let nop = param("nop", NOPS_PER_LINE - 1)?;
        let line = param("line", 0xffff)?;
        let frame_nop = param("frame_nop", NOPS_PER_LINE * LINES_PER_FRAME - 1)?;

        let breakpoint = if let Some(frame_nop) = frame_nop {
            Some(RasterBreakpoint { line: Some(frame_nop / NOPS_PER_LINE), x: frame_nop % NOPS_PER_LINE * BEAM_TICKS_PER_NOP })
        }
        else if nop.is_some() || line.is_some() {
            Some(RasterBreakpoint { line, x: nop.unwrap_or(0) * BEAM_TICKS_PER_NOP })
        }
        else if x.is_some() || y.is_some() {
            let (dx, dy) = match (crop, self.beam) {
                (true, Some(beam)) => (beam.crop_x, beam.crop_y as isize),
                (true, None) => return Err(ApiError::field("crop", "no beam position to crop against yet")),
                (false, _) => (0, 0)
            };
            let x = (x.unwrap_or(0) as isize + dx).max(0) as usize;
            Some(RasterBreakpoint { line: y.map(|y| (y as isize + dy).max(0) as usize), x })
        }
        else {
            None
        };
        if !add || !enable {
            self.raster_breakpoints.clear();
        }
        match (enable, breakpoint) {
            (true, Some(breakpoint)) => self.raster_breakpoints.push(breakpoint),
            (true, None) if !add => {},
            (true, None) => return Err(ApiError::new("add=1 needs a position: x/y, line/nop or frame_nop")),
            (false, _) => {}
        }
        let mut reply = self.list_raster_breakpoints();
        merge(&mut reply, json!({ "ok": true, "cmd_seq": self.command() }));
        Ok(ApiResponse::ok(reply))
    }

    // ------------------------------------------------------------ BASIC

    fn run_basic_to(&mut self, target: BasicTarget) -> ApiResponse {
        self.basic_target = Some(target);
        self.paused = false;
        ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() }))
    }

    fn set_basic_breakpoints(&mut self, request: &ApiRequest) -> ApiResult {
        let mut lines = std::collections::BTreeSet::new();
        for entry in request.text().split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let line = entry
                .parse::<u16>()
                .map_err(|_| ApiError::new(format!("{entry}: expected a BASIC line number (0-65535)")))?;
            lines.insert(line);
        }
        self.basic_breakpoints = lines;
        Ok(ApiResponse::ok(json!({ "ok": true, "lines": self.basic_breakpoints, "cmd_seq": self.command() })))
    }

    /// `POST /api/basic[?reset=1][&run=1]`: after a reset, the program is
    /// loaded once the firmware booted.
    fn basic_load_request(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let flag = |name: &str| request.param(name).is_some_and(|v| v != "0" && !v.is_empty());
        let source = request.text();
        // Checked now, to answer 400 rather than fail later.
        cpclib::basic::BasicProgram::parse(&source)
            .map_err(|e| ApiError::new(format!("BASIC inject failed: {e:?}")))?;
        if flag("reset") {
            motherboard.restart();
            self.events.publish("reset", &json!({ "hard": true }));
            self.pending_basic = Some(PendingBasic {
                source,
                run: flag("run"),
                at_t_states: motherboard.t_state_counter().value() + BASIC_BOOT_T_STATES
            });
        }
        else if motherboard.t_state_counter().value() < BASIC_BOOT_T_STATES {
            // Just powered on, the firmware still booting: BASIC would clear
            // the program as it starts. Loaded once it is up, like after a
            // reset (a debugger that starts the emulator injects at once).
            self.pending_basic = Some(PendingBasic { source, run: flag("run"), at_t_states: BASIC_BOOT_T_STATES });
        }
        else {
            self.load_basic(&source, flag("run"), motherboard)
                .map_err(|e| ApiError::new(format!("BASIC inject failed: {e}")))?;
        }
        Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
    }

    fn load_basic(&mut self, source: &str, run: bool, motherboard: &mut Motherboard) -> Result<usize, String> {
        let size = basic_program::load(motherboard, source)?;
        self.ram_seq += 1;
        if run {
            self.typing.extend("RUN\n".chars().filter_map(keys_of_char));
        }
        Ok(size)
    }

    // --------------------------------------------------------- counters

    fn list_counters(&self) -> Value {
        json!({
            "counters": self.counters.iter().map(|(name, counter)| json!({
                "name": name,
                "nops": counter.t_states / T_STATES_PER_NOP as u64,
                "running": counter.running
            })).collect::<Vec<_>>()
        })
    }

    /// `POST /api/counters {"name", "action": create|reset|start|stop}`:
    /// a counter counts the NOPs the emulation runs while it is running
    /// (from its creation unless created with `"running": false`).
    fn counter_command(&mut self, request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
        let body = parse_body(&request.text())?;
        let name = match find_key(&body, "name") {
            Some(Value::String(name)) if !name.is_empty() => name.clone(),
            _ => return Err(ApiError::field("name", "missing field \"name\": the counter's name"))
        };
        let action = match find_key(&body, "action") {
            None => "create",
            Some(Value::String(action)) => action.as_str(),
            Some(_) => return Err(ApiError::field("action", "field \"action\" must be a string"))
        };
        let running = match find_key(&body, "running") {
            None => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(ApiError::field("running", "field \"running\" must be a boolean"))
        };
        // Count from the instruction about to run.
        self.count_nops(motherboard);
        match action {
            "create" => {
                if self.counters.contains_key(&name) {
                    return Err(ApiError::field("name", format!("counter \"{name}\" exists (reset or delete it)")));
                }
                self.counters.insert(name, NopCounter { t_states: 0, running });
            },
            "reset" | "start" | "stop" => {
                let counter = self
                    .counters
                    .get_mut(&name)
                    .ok_or_else(|| ApiError::field("name", format!("no counter \"{name}\"")))?;
                match action {
                    "reset" => counter.t_states = 0,
                    "start" => counter.running = true,
                    _ => counter.running = false
                }
            },
            other => {
                return Err(ApiError::field("action", format!("unknown action \"{other}\": create, reset, start or stop")))
            }
        }
        let mut reply = self.list_counters();
        merge(&mut reply, json!({ "ok": true }));
        Ok(ApiResponse::ok(reply))
    }

    fn delete_counters(&mut self, request: &ApiRequest) -> ApiResult {
        match request.param("name") {
            None => self.counters.clear(),
            Some(name) => {
                if self.counters.remove(name).is_none() {
                    return Err(ApiError::field("name", format!("no counter \"{name}\"")));
                }
            }
        }
        let mut reply = self.list_counters();
        merge(&mut reply, json!({ "ok": true }));
        Ok(ApiResponse::ok(reply))
    }

    // --------------------------------------------------------- commands

    fn set_z80(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let body = parse_body(&request.text())?;
        let words = [("PC", Reg16::Pc), ("SP", Reg16::Sp), ("IX", Reg16::Ix), ("IY", Reg16::Iy)];
        let bytes = [
            ("A", Reg8::A), ("F", Reg8::F), ("B", Reg8::B), ("C", Reg8::C),
            ("D", Reg8::D), ("E", Reg8::E), ("H", Reg8::H), ("L", Reg8::L),
            ("A2", Reg8::A2), ("F2", Reg8::F2), ("B2", Reg8::B2), ("C2", Reg8::C2),
            ("D2", Reg8::D2), ("E2", Reg8::E2), ("H2", Reg8::H2), ("L2", Reg8::L2),
            ("I", Reg8::I), ("R", Reg8::R)
        ];
        // Validate everything before changing anything.
        let mut word_values = Vec::new();
        for (name, reg) in words {
            if let Some(v) = int_field(&body, name, 0, 0xffff)? {
                word_values.push((reg, v as u16));
            }
        }
        let mut byte_values = Vec::new();
        for (name, reg) in bytes {
            if let Some(v) = int_field(&body, name, 0, 0xff)? {
                byte_values.push((reg, v as u8));
            }
        }
        let iff1 = int_field(&body, "IFF1", 0, 1)?;
        let iff2 = int_field(&body, "IFF2", 0, 1)?;
        let im = int_field(&body, "IM", 0, 2)?;
        if word_values.is_empty() && byte_values.is_empty() && iff1.is_none() && iff2.is_none() && im.is_none() {
            return Err(ApiError::new("no recognised field: expected Z80 register names (PC, A, IFF1, IM...)"));
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
            cpu.set_iffs(iff1.map_or(current1, |v| v == 1), iff2.map_or(current2, |v| v == 1));
        }
        if let Some(im) = im {
            cpu.set_im([InterruptMode::Mode0, InterruptMode::Mode1, InterruptMode::Mode2][im as usize]);
        }
        Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
    }

    fn write_ram(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let body = parse_body(&request.text())?;
        let addr = int_field(&body, "addr", 0, 0xffff)?;
        let bank_count = motherboard.physical_bank_count() as i64;
        let bank = int_field(&body, "bank", 0, bank_count - 1)?.unwrap_or(0) as usize;
        let data = match find_key(&body, "data") {
            None => Vec::new(),
            Some(Value::String(text)) => parse_hex(text).ok_or_else(|| ApiError::field("data", "field \"data\" must be a hex string"))?,
            Some(_) => return Err(ApiError::field("data", "field \"data\" must be a hex string"))
        };
        let exec = match find_key(&body, "exec") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(ApiError::field("exec", "field \"exec\" must be a boolean"))
        };
        let entry = int_field(&body, "entry", 0, 0xffff)?;
        if data.is_empty() && !exec {
            return Err(ApiError::new("nothing to do: no \"data\" to write and no \"exec\""));
        }
        let addr = match (addr, data.is_empty()) {
            (Some(addr), _) => addr as usize,
            (None, true) => 0,
            (None, false) => return Err(ApiError::field("addr", "missing field \"addr\": where to write \"data\""))
        };
        for (i, &byte) in data.iter().enumerate() {
            // An offset past 0x3FFF carries into the next banks.
            let linear = bank * 0x4000 + addr + i;
            if !motherboard.write_bank_byte((linear / 0x4000) as u16, (linear % 0x4000) as u16, byte) {
                return Err(ApiError::field("data", format!("the write runs past the last RAM bank (B{:02X})", bank_count - 1)));
            }
        }
        if exec {
            let target = entry.unwrap_or(addr as i64) as u16;
            motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(target);
        }
        self.ram_seq += 1;
        Ok(ApiResponse::ok(json!({ "ok": true, "seq": self.ram_seq, "cmd_seq": self.command() })))
    }

    fn exec(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let body = parse_body(&request.text())?;
        let addr = int_field(&body, "addr", 0, 0xffff)?.ok_or_else(|| ApiError::field("addr", "missing field \"addr\": the new PC"))?;
        motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(addr as u16);
        self.ram_seq += 1;
        Ok(ApiResponse::ok(json!({ "ok": true, "seq": self.ram_seq, "cmd_seq": self.command() })))
    }

    fn config(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let body = parse_body(&request.text())?;
        let bool_field = |name: &str| -> Result<Option<bool>, ApiError> {
            match find_key(&body, name) {
                None => Ok(None),
                Some(Value::Bool(b)) => Ok(Some(*b)),
                Some(_) => Err(ApiError::field(name, format!("field \"{name}\" must be a boolean")))
            }
        };
        let paused = bool_field("paused")?;
        let soft = bool_field("do_soft_reset")?.unwrap_or(false);
        let hard = bool_field("do_hard_reset")?.unwrap_or(false);
        let history_size = int_field(&body, "history_size", 1, 100_000)?;
        if paused.is_none() && !soft && !hard && history_size.is_none() {
            return Err(ApiError::new(
                "no recognised field: supported are paused, do_soft_reset, do_hard_reset, history_size"
            ));
        }
        if let Some(size) = history_size {
            // Resizing empties the history, as in AMSpiriT.
            self.history_size = size as usize;
            self.history.clear();
        }
        if soft || hard {
            // Soft: the reset button (RAM kept). Hard: power off and on.
            if hard {
                motherboard.power_cycle();
            }
            else {
                motherboard.restart();
            }
            self.events.publish("reset", &json!({ "hard": hard }));
        }
        if let Some(paused) = paused {
            self.paused = paused;
        }
        Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
    }

    /// `POST /api/z80_bp`: CPU addresses are resolved against the live
    /// mapping, `Bnn:hhhh` (hex 16K bank, hex offset carrying into the next
    /// banks) and the deprecated `Cnn:hhhh` (decimal bank) name a RAM byte.
    fn set_breakpoints(&mut self, request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
        let mut breakpoints = HashSet::new();
        let mut resolved = Vec::new();
        let mut warnings = Vec::new();
        let text = request.text();
        for entry in text.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let location = match entry.split_once(':') {
                Some((bank, offset)) if bank.starts_with(['B', 'b', 'C', 'c']) => {
                    let decimal = bank.starts_with(['C', 'c']);
                    if decimal {
                        warnings.push(format!("{entry}: the Cnn: form is deprecated, use Bnn: (hex bank)"));
                    }
                    let bank = u32::from_str_radix(&bank[1..], if decimal { 10 } else { 16 }).ok();
                    let offset = u32::from_str_radix(offset.trim_start_matches("0x"), 16).ok();
                    let (Some(bank), Some(offset)) = (bank, offset)
                    else {
                        return Err(ApiError::new(format!("{entry}: expected Bnn:hhhh (hex bank, hex offset)")));
                    };
                    let linear = bank as u64 * 0x4000 + offset as u64;
                    let bank = linear / 0x4000;
                    if bank >= motherboard.physical_bank_count() as u64 {
                        return Err(ApiError::new(format!(
                            "{entry}: no such RAM bank (B00-B{:02X} exist)",
                            motherboard.physical_bank_count() - 1
                        )));
                    }
                    PhysicalAddress::Ram { bank: bank as u16, offset: (linear % 0x4000) as u16 }
                },
                _ => {
                    let address = parse_int(entry)
                        .filter(|v| (0..=0xffff).contains(v))
                        .ok_or_else(|| ApiError::new(format!("{entry}: expected a CPU address (decimal or 0x hex, 0-65535) or Bnn:hhhh")))?;
                    motherboard.physical_address(address as u16)
                }
            };
            resolved.push(json!({ "entry": entry, "location": location_name(location) }));
            breakpoints.insert(location);
        }
        self.exec_breakpoints = breakpoints;
        let mut body = json!({ "ok": true, "cmd_seq": self.command(), "resolved": resolved });
        if !warnings.is_empty() {
            body["warnings"] = json!(warnings);
        }
        Ok(ApiResponse::ok(body))
    }

    /// `GET /api/breakpoints` (extension).
    fn list_breakpoints(&self) -> Value {
        let mut exec: Vec<String> = self.exec_breakpoints.iter().map(|l| location_name(*l)).collect();
        exec.sort();
        json!({
            "z80_bp": exec,
            "advanced": self.advanced.iter().enumerate().map(|(index, b)| breakpoint_json(index, b)).collect::<Vec<_>>()
        })
    }

    /// `POST /api/breakpoints` (extension): one advanced breakpoint, as a
    /// JSON object (`kind` exec|mem|io, `access` r|w|rw, `run` stop|watch,
    /// `addr`, `mask`, `size`, `value`, `valmask`, `name`) or as the text of
    /// a REMU `acebreak` entry in `remu`.
    fn add_breakpoint(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let body = parse_body(&request.text())?;
        let brk = match find_key(&body, "remu") {
            Some(Value::String(text)) => AdvancedBreakpoint::parse_remu(text).map_err(|e| ApiError::field("remu", e))?,
            Some(_) => return Err(ApiError::field("remu", "field \"remu\" must be a string")),
            None => breakpoint_of_json(&body)?
        };
        self.advanced.push(brk);
        motherboard.watch_bus(self.watches_bus());
        Ok(ApiResponse::ok(json!({ "ok": true, "index": self.advanced.len() - 1, "cmd_seq": self.command() })))
    }

    /// `DELETE /api/breakpoints[?index=N]` (extension): one, or all.
    fn remove_breakpoints(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        match request.query.iter().find(|(k, _)| k == "index") {
            Some((_, index)) => {
                let index = parse_int(index)
                    .filter(|i| (0..self.advanced.len() as i64).contains(i))
                    .ok_or_else(|| ApiError::field("index", "parameter \"index\" must name an existing breakpoint"))?;
                self.advanced.remove(index as usize);
            },
            None => self.advanced.clear()
        }
        motherboard.watch_bus(self.watches_bus());
        Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
    }

    /// Breakpoints read from elsewhere (e.g. a snapshot's chunks).
    pub fn add_advanced_breakpoint(&mut self, brk: AdvancedBreakpoint, motherboard: &mut Motherboard) {
        self.advanced.push(brk);
        motherboard.watch_bus(self.watches_bus());
    }

    /// An execution breakpoint on a CPU address, resolved now.
    pub fn add_exec_breakpoint(&mut self, address: u16, motherboard: &Motherboard) {
        self.exec_breakpoints.insert(motherboard.physical_address(address));
    }

    /// `GET /api/history[?n=N]`: oldest first.
    fn history_list(&self, request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
        let n = match request.param("n") {
            None => self.history.len(),
            Some(text) => parse_int(text)
                .filter(|n| (1..=100_000).contains(n))
                .ok_or_else(|| ApiError::field("n", "parameter \"n\" must be between 1 and 100000"))? as usize
        };
        let hex = |bytes: [u8; 4]| bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
        let entries: Vec<Value> = self
            .history
            .iter()
            .skip(self.history.len().saturating_sub(n))
            .map(|&(pc, bytes)| {
                json!({ "pc": pc, "hex": hex(bytes), "current_hex": hex(instruction_bytes(motherboard, pc)) })
            })
            .collect();
        Ok(ApiResponse::ok(json!(entries)))
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
        let flag = |name: &str, default: bool| -> Result<bool, ApiError> {
            match request.param(name) {
                None | Some("") => Ok(default),
                Some("0") => Ok(false),
                Some("1") => Ok(true),
                Some(_) => Err(ApiError::field(name, format!("{name} must be 0 or 1")))
            }
        };
        let crop = flag("crop", true)?;
        let live = flag("live", self.paused)?;
        let alone = flag("full", true)?;
        let no_frame = || Ok(ApiResponse::json(503, json!({ "error": "no frame available" })));

        let settled = if crop { self.last_frame.as_ref() } else { monitor.and_then(Monitor::full_frame) };
        let drawing = match (live, monitor) {
            (true, Some(monitor)) if crop => Some(monitor.image_in_progress()),
            (true, Some(monitor)) => monitor.full_frame_in_progress(),
            _ => None
        };
        if !crop && monitor.and_then(Monitor::full_frame_in_progress).is_none() {
            return Ok(ApiResponse::json(
                503,
                json!({ "error": "whole frames are not kept here (crop=0 needs the emulator's monitor)" })
            ));
        }
        // Where the beam is in the returned picture: what is drawn so far.
        let (origin_x, origin_y) = match (crop, self.beam) {
            (true, Some(beam)) => (beam.crop_x, beam.crop_y as isize),
            _ => (0, 0)
        };
        let image = match drawing {
            Some(drawing) if alone || settled.is_none() => drawing.clone(),
            Some(drawing) => {
                let beam = self.beam.unwrap_or_default();
                composite(
                    drawing,
                    settled.unwrap(),
                    beam.line as isize - origin_y,
                    beam.x as isize - origin_x
                )
            },
            None => match settled {
                Some(settled) => settled.clone(),
                None => return no_frame()
            }
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
            ("X-Crop-Y", origin_y.to_string())
        ];
        if let Some(beam) = self.beam {
            headers.push(("X-Beam-X", beam.x.to_string()));
            headers.push(("X-Beam-Y", beam.line.to_string()));
        }
        Ok(ApiResponse {
            status: 200,
            content_type: "image/png",
            body: png.into_inner(),
            headers: headers.into_iter().map(|(name, value)| (name.to_owned(), value)).collect()
        })
    }

    /// `POST /api/media?name=&drive=&crtc=`: the raw file as the body. A
    /// snapshot or a disc image is recognised by its content, a binary by
    /// its name (`.bin`/`.amsdos`): with an AMSDOS header it loads at the
    /// header's address, headerless it needs `name@LOAD[@ENTRY].bin` (hex).
    /// The CPU jumps only to an `@ENTRY` given in the name; the reply gives
    /// the header's entry otherwise. A snapshot's breakpoints are set.
    /// `crtc` is accepted for compatibility: the CRTC is always kept.
    fn media(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let bytes = &request.body;
        if bytes.is_empty() {
            return Err(ApiError::new("empty body: expected the raw media file bytes"));
        }
        let name = request.param("name").unwrap_or("");
        let drive = match request.param("drive") {
            None | Some("0") => crate::fdc::FdcDrive::A,
            Some("1") => crate::fdc::FdcDrive::B,
            Some(_) => return Err(ApiError::field("drive", "parameter \"drive\" must be 0 or 1"))
        };
        if let Some(crtc) = request.param("crtc") {
            if !matches!(crtc, "keep" | "sna") {
                return Err(ApiError::field("crtc", "parameter \"crtc\" must be one of keep, sna"));
            }
        }
        let lower_name = name.to_ascii_lowercase();
        let mut reply = json!({ "ok": true });
        if bytes.starts_with(b"MV - SNA") {
            let breakpoints = crate::snapshot::load_sna_bytes(motherboard, bytes).map_err(ApiError::new)?;
            for brk in &breakpoints.exec {
                self.add_exec_breakpoint(brk.address, motherboard);
            }
            for brk in &breakpoints.advanced {
                self.add_advanced_breakpoint(brk.clone(), motherboard);
            }
            reply["media"] = json!("sna");
            reply["breakpoints"] = json!(breakpoints.len());
            if !breakpoints.ignored.is_empty() {
                reply["warnings"] = json!(breakpoints.ignored);
            }
        }
        else if bytes.starts_with(b"MV - CPC") || bytes.starts_with(b"EXTENDED CPC DSK") {
            let image = crate::disc_image::load_dsk(bytes).map_err(ApiError::new)?;
            motherboard.fdc_mut().insert_image(drive, Some(image));
            reply["media"] = json!("dsk");
        }
        else if lower_name.ends_with(".bin") || lower_name.ends_with(".amsdos") {
            let binary = parse_binary(name, bytes).map_err(|e| ApiError::field("name", e))?;
            motherboard.write_buffer(binary.load, binary.data);
            if let Some(entry) = binary.jump {
                motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(entry);
            }
            reply["media"] = json!("bin");
            reply["load"] = json!(binary.load);
            reply["len"] = json!(binary.data.len());
            if let Some(entry) = binary.entry {
                reply["entry"] = json!(entry);
            }
        }
        else {
            return Err(ApiError::new(
                "unsupported media: a .sna or .dsk image (recognised by its content), or a .bin/.amsdos named by ?name="
            ));
        }
        reply["cmd_seq"] = json!(self.command());
        Ok(ApiResponse::ok(reply))
    }

    fn keytype(&mut self, request: &ApiRequest) -> ApiResult {
        let body = parse_body(&request.text())?;
        let text = match find_key(&body, "text") {
            Some(Value::String(text)) => text.clone(),
            Some(_) => return Err(ApiError::field("text", "field \"text\" must be a string")),
            None => return Err(ApiError::field("text", "missing field \"text\": the characters to type"))
        };
        for c in text.chars() {
            let c = if c == '\r' { '\n' } else { c };
            let keys = keys_of_char(c).ok_or_else(|| ApiError::field("text", format!("cannot type {c:?}")))?;
            self.typing.push_back(keys);
        }
        Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
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
        }
        else if let Some(keys) = self.typing.pop_front() {
            for &key in &keys {
                motherboard.keyboard_mut().set_pressed(key, true);
            }
            self.typed_pressed = Some(keys);
        }
        self.typing_next_at = now + KEY_T_STATES;
    }
}

/// AMSpiriT's `beam` object: the beam in the video buffer (1024 ticks by
/// 350 lines) and the visible area, plus the position in NOPs.
fn beam_json(beam: Beam) -> Value {
    let mut value = beam.json();
    merge(&mut value, beam_crop(beam));
    value
}

fn beam_crop(beam: Beam) -> Value {
    json!({ "cx": beam.crop_x, "cy": beam.crop_y, "cw": SCREENSHOT_SIZE.0, "ch": SCREENSHOT_SIZE.1 })
}

fn basic_state(motherboard: &Motherboard) -> Value {
    let state = BasicState::read(motherboard);
    json!({
        "basic_ver": basic_program::BasicRom::of(motherboard.model()).version,
        "prog_start": state.prog_start,
        "txttop": state.txttop,
        "vartop": state.vartop,
        "arrend": state.arrend,
        "prog_size": state.txttop.wrapping_sub(state.prog_start),
        "var_size": state.vartop.wrapping_sub(state.txttop),
        "chain_heads_addr": basic_program::BasicRom::of(motherboard.model()).variable_chain_heads,
        "cur_linenum": state.line.unwrap_or(0xffff),
        "stmt_addr": state.stmt_addr
    })
}

fn basic_listing(motherboard: &Motherboard) -> ApiResult {
    let lines = basic_program::listing(motherboard).map_err(|e| ApiError::new(format!("cannot decode the program: {e}")))?;
    Ok(ApiResponse::ok(json!({
        "lines": lines.iter().map(|line| json!({
            "addr": line.addr,
            "num": line.number,
            "stmts": line.statements.iter().map(|s| json!({
                "addr": s.addr, "end": s.end, "colon": s.colon, "text": s.text, "vars": s.vars
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })))
}

/// The visible picture (what `GET /api/screenshot` returns).
const SCREENSHOT_SIZE: (usize, usize) = (768, 272);

/// The frame being drawn over the previous one: `drawing` where the beam
/// has been (the rows before `row`, and `row` up to `column`), `previous`
/// elsewhere.
fn composite(drawing: &image::RgbImage, previous: &image::RgbImage, row: isize, column: isize) -> image::RgbImage {
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
    use crate::ga::VideoMode;
    let ga = motherboard.ga();
    let palette = ga.palette();
    let mmr = motherboard.mmr();
    json!({
        "mode": match ga.active_video_mode() {
            VideoMode::Mode0 => 0, VideoMode::Mode1 => 1, VideoMode::Mode2 => 2, VideoMode::Mode3 => 3
        },
        "pen": ga.selected_pen(),
        // The hardware colour numbers (0x40 | n is the INKR write).
        "border_idx": palette[16].hardware_number(),
        "border_rgb": rgb(palette[16].into()),
        "ink_idx": palette[..16].iter().map(|ink| ink.hardware_number()).collect::<Vec<_>>(),
        "ink_rgb": palette[..16].iter().map(|&ink| rgb(ink.into())).collect::<Vec<_>>(),
        "rmr": ga.rmr(),
        "ram_mode": mmr & 7,
        "ram_page": (mmr >> 3) & 7,
        // Extension: firmware ink numbers, ROM enables, interrupt counter.
        "ink_firmware": palette.iter().map(|ink| ink.number()).collect::<Vec<_>>(),
        "lower_rom": ga.lower_rom_pagging().is_enabled(),
        "upper_rom": ga.upper_rom_pagging().is_enabled(),
        "r52": ga.r52()
    })
}

fn psg(motherboard: &Motherboard) -> Value {
    let psg = motherboard.psg();
    let word = |low: u8| u16::from_le_bytes([psg.register(low), psg.register(low + 1)]);
    json!({
        "period_a": word(0) & 0xfff, "vol_a": psg.register(8) & 0x1f,
        "period_b": word(2) & 0xfff, "vol_b": psg.register(9) & 0x1f,
        "period_c": word(4) & 0xfff, "vol_c": psg.register(10) & 0x1f,
        "mixer": psg.register(7), "noise": psg.register(6) & 0x1f,
        "env_period": word(11), "env_shape": psg.register(13) & 0x0f,
        // Extension: the 16 registers as they are, the selected one.
        "regs": (0..16).map(|r| psg.register(r)).collect::<Vec<_>>(),
        "selected_reg": psg.selected_register()
    })
}

fn fdc(motherboard: &Motherboard) -> Value {
    use crate::fdc::FdcDrive;
    let fdc = motherboard.fdc();
    let [st0, st1, st2] = fdc.status_registers();
    json!({
        "msr": fdc.read_status(),
        "sr0": st0,
        "sr1": st1,
        "sr2": st2,
        "motor": fdc.motor(),
        "phase": fdc.phase_name(),
        "command": fdc.command_name(),
        "drives": ([FdcDrive::A, FdcDrive::B]).iter().enumerate().map(|(unit, &drive)| json!({
            "connected": true,
            "disk": fdc.has_disk(drive),
            "track": fdc.track(unit)
        })).collect::<Vec<_>>()
    })
}

/// `GET /api/memmap`: each 16K region of the CPU's view, with the ROM
/// read there if any and the absolute RAM bank (written there in any case).
fn memmap(motherboard: &Motherboard) -> Value {
    let regions: Vec<Value> = (0..4u16)
        .map(|region| {
            let base = region * 0x4000;
            let ram_bank = match motherboard.physical_address_for_write(base) {
                PhysicalAddress::Ram { bank, .. } => bank,
                _ => unreachable!("writes always reach RAM")
            };
            let mut value = json!({ "base": base, "name": format!("{base:04X}") });
            match motherboard.physical_address(base) {
                PhysicalAddress::LowerRom { .. } => merge(&mut value, json!({ "rom": true, "rom_bank": 255 })),
                PhysicalAddress::UpperRom { rom, .. } => merge(&mut value, json!({ "rom": true, "rom_bank": rom })),
                PhysicalAddress::Ram { bank, .. } => {
                    merge(&mut value, json!({ "rom": false, "ram_bank": bank, "ext": bank >= 4 }))
                }
            }
            // Extension: where writes go, ROM or not.
            value["write_bank"] = json!(ram_bank);
            value
        })
        .collect();
    let mmr = motherboard.mmr();
    json!({
        "regions": regions,
        "rmr": motherboard.ga().rmr(),
        "ram_mode": mmr & 7,
        "ram_page": (mmr >> 3) & 7,
        "mmr": mmr,
        "ram_banks": motherboard.physical_bank_count()
    })
}

fn z80(motherboard: &Motherboard) -> Value {
    let cpu = motherboard.cpu();
    let r8 = |reg: Reg8| cpu.reg8(reg).get();
    let r16 = |reg: Reg16| cpu.reg16(reg).get();
    json!({
        "PC": r16(Reg16::Pc), "SP": r16(Reg16::Sp),
        "A": r8(Reg8::A), "F": r8(Reg8::F), "B": r8(Reg8::B), "C": r8(Reg8::C),
        "D": r8(Reg8::D), "E": r8(Reg8::E), "H": r8(Reg8::H), "L": r8(Reg8::L),
        "A2": r8(Reg8::A2), "F2": r8(Reg8::F2), "B2": r8(Reg8::B2), "C2": r8(Reg8::C2),
        "D2": r8(Reg8::D2), "E2": r8(Reg8::E2), "H2": r8(Reg8::H2), "L2": r8(Reg8::L2),
        "IX": r16(Reg16::Ix), "IY": r16(Reg16::Iy), "I": r8(Reg8::I), "R": r8(Reg8::R),
        "IFF1": cpu.iff1() as u8, "IFF2": cpu.iff2() as u8,
        "IM": match cpu.im() { InterruptMode::Mode0 => 0, InterruptMode::Mode1 => 1, InterruptMode::Mode2 => 2 },
        "ticks": motherboard.t_state_counter().value() / T_STATES_PER_NOP
    })
}

/// A binary to load: where, what, and where to start.
struct Binary<'b> {
    load: u16,
    data: &'b [u8],
    /// The program's entry (header or name).
    entry: Option<u16>,
    /// The entry to jump to: only one given in the name.
    jump: Option<u16>
}

/// An AMSDOS header (128 bytes, checksum of bytes 0-66 at 67-68) gives the
/// load address (21-22), the length (24-25) and the entry (26-27); a
/// headerless file needs `name@LOAD[@ENTRY].ext`, hex addresses.
fn parse_binary<'b>(name: &str, bytes: &'b [u8]) -> Result<Binary<'b>, String> {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let mut parts = stem.split('@').skip(1);
    let hex = |text: &str| u16::from_str_radix(text, 16).map_err(|_| format!("{text}: expected a hex address in the name"));
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
            jump: named_entry
        });
    }
    let load = named_load.ok_or("no AMSDOS header: give the load address in the name, e.g. game@4000.bin")?;
    Ok(Binary { load, data: bytes, entry: named_entry, jump: named_entry })
}

/// Decode `%XX` and `+` in a query value.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if let Some(byte) = text.get(i + 1..i + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok()) => {
                out.push(byte);
                i += 2;
            },
            b => out.push(b)
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
    beam: Option<Beam>
}

impl ConditionContext for MachineContext<'_> {
    fn variable(&self, name: &str) -> Option<i64> {
        let cpu = self.motherboard.cpu();
        let r8 = |reg: Reg8| Some(cpu.reg8(reg).get() as i64);
        let r16 = |reg: Reg16| Some(cpu.reg16(reg).get() as i64);
        match name {
            "A" => r8(Reg8::A), "F" => r8(Reg8::F), "B" => r8(Reg8::B), "C" => r8(Reg8::C),
            "D" => r8(Reg8::D), "E" => r8(Reg8::E), "H" => r8(Reg8::H), "L" => r8(Reg8::L),
            "I" => r8(Reg8::I), "R" => r8(Reg8::R),
            "IXH" => r8(Reg8::Ixh), "IXL" => r8(Reg8::Ixl), "IYH" => r8(Reg8::Iyh), "IYL" => r8(Reg8::Iyl),
            "AF" => r16(Reg16::Af), "BC" => r16(Reg16::Bc), "DE" => r16(Reg16::De), "HL" => r16(Reg16::Hl),
            "IX" => r16(Reg16::Ix), "IY" => r16(Reg16::Iy), "SP" => r16(Reg16::Sp), "PC" => r16(Reg16::Pc),
            "A'" => r8(Reg8::A2), "F'" => r8(Reg8::F2), "B'" => r8(Reg8::B2), "C'" => r8(Reg8::C2),
            "D'" => r8(Reg8::D2), "E'" => r8(Reg8::E2), "H'" => r8(Reg8::H2), "L'" => r8(Reg8::L2),
            "AF'" => r16(Reg16::Af2), "BC'" => r16(Reg16::Bc2), "DE'" => r16(Reg16::De2), "HL'" => r16(Reg16::Hl2),
            "IFF1" => Some(cpu.iff1() as i64),
            "IFF2" => Some(cpu.iff2() as i64),
            "IM" => Some(match cpu.im() { InterruptMode::Mode0 => 0, InterruptMode::Mode1 => 1, InterruptMode::Mode2 => 2 }),
            "ADDRESS" => self.access.map(|a| a.address as i64),
            "VALUE" => self.access.map(|a| a.value as i64),
            _ => self.ace_variable(name)
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
        if let Some(number) = name.strip_prefix("CRTCR").and_then(|n| n.parse::<u8>().ok()) {
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
            _ => None
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
fn registers_event(motherboard: &Motherboard) -> Value {
    let cpu = motherboard.cpu();
    let r8 = |reg: Reg8| cpu.reg8(reg).get();
    let r16 = |reg: Reg16| hex16(cpu.reg16(reg).get());
    json!({
        "pc": r16(Reg16::Pc), "sp": r16(Reg16::Sp),
        "a": r8(Reg8::A), "f": r8(Reg8::F), "b": r8(Reg8::B), "c": r8(Reg8::C),
        "d": r8(Reg8::D), "e": r8(Reg8::E), "h": r8(Reg8::H), "l": r8(Reg8::L),
        "ix": r16(Reg16::Ix), "iy": r16(Reg16::Iy)
    })
}

fn merge(target: &mut Value, extra: Value) {
    if let (Value::Object(target), Value::Object(extra)) = (target, extra) {
        target.extend(extra);
    }
}

fn kind_name(kind: BreakpointKind) -> &'static str {
    match kind {
        BreakpointKind::Exec => "exec",
        BreakpointKind::Memory => "mem",
        BreakpointKind::Io => "io"
    }
}

/// `B04:0200`, `LOWER:0038`, `UPPER07:0006`.
fn location_name(location: PhysicalAddress) -> String {
    match location {
        PhysicalAddress::Ram { bank, offset } => format!("B{bank:02X}:{offset:04X}"),
        PhysicalAddress::LowerRom { offset } => format!("LOWER:{offset:04X}"),
        PhysicalAddress::UpperRom { rom, offset } => format!("UPPER{rom:02X}:{offset:04X}")
    }
}

fn breakpoint_json(index: usize, brk: &AdvancedBreakpoint) -> Value {
    json!({
        "index": index,
        "kind": kind_name(brk.kind),
        "access": match brk.access { BreakpointAccess::Read => "r", BreakpointAccess::Write => "w", BreakpointAccess::ReadWrite => "rw" },
        "run": match brk.run { RunMode::Stop => "stop", RunMode::Watch => "watch", RunMode::Count => "count" },
        "addr": brk.address, "mask": brk.mask, "size": brk.size,
        "value": brk.value, "valmask": brk.value_mask,
        "name": brk.name,
        "condition": brk.condition.as_ref().map(Condition::source),
        "counters": brk.counters.iter().map(|(action, name)| json!({ "action": action.name(), "name": name })).collect::<Vec<_>>(),
        "after": brk.after,
        "hits": brk.hits
    })
}

fn breakpoint_of_json(body: &Value) -> Result<AdvancedBreakpoint, ApiError> {
    let text = |name: &str| -> Result<Option<String>, ApiError> {
        match find_key(body, name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.to_ascii_lowercase())),
            Some(_) => Err(ApiError::field(name, format!("field \"{name}\" must be a string")))
        }
    };
    let mut brk = AdvancedBreakpoint::exec(0);
    brk.kind = match text("kind")?.as_deref() {
        None | Some("exec") => BreakpointKind::Exec,
        Some("mem" | "memory") => BreakpointKind::Memory,
        Some("io") => BreakpointKind::Io,
        Some(_) => return Err(ApiError::field("kind", "field \"kind\" must be one of exec, mem, io"))
    };
    brk.access = match text("access")?.as_deref() {
        None | Some("rw") => BreakpointAccess::ReadWrite,
        Some("r" | "read") => BreakpointAccess::Read,
        Some("w" | "write") => BreakpointAccess::Write,
        Some(_) => return Err(ApiError::field("access", "field \"access\" must be one of r, w, rw"))
    };
    brk.run = match text("run")?.as_deref() {
        None | Some("stop") => RunMode::Stop,
        Some("watch") => RunMode::Watch,
        Some("count") => RunMode::Count,
        Some(_) => return Err(ApiError::field("run", "field \"run\" must be one of stop, watch, count"))
    };
    brk.address = int_field(body, "addr", 0, 0xffff)?.ok_or_else(|| ApiError::field("addr", "missing field \"addr\""))? as u16;
    brk.mask = int_field(body, "mask", 0, 0xffff)?.unwrap_or(0xffff) as u16;
    brk.size = int_field(body, "size", 1, 0xffff)?.unwrap_or(1) as u16;
    brk.value = int_field(body, "value", 0, 0xff)?.unwrap_or(0) as u8;
    brk.value_mask = int_field(body, "valmask", 0, 0xff)?.unwrap_or(0) as u8;
    brk.name = match find_key(body, "name") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.clone()),
        Some(_) => return Err(ApiError::field("name", "field \"name\" must be a string"))
    };
    brk.condition = match find_key(body, "condition") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) if text.trim().is_empty() => None,
        Some(Value::String(text)) => Some(Condition::parse(text).map_err(|e| ApiError::field("condition", e))?),
        Some(_) => return Err(ApiError::field("condition", "field \"condition\" must be a string"))
    };
    // "start"/"stop"/"reset": a counter name or a list of them.
    for action in [CounterAction::Reset, CounterAction::Start, CounterAction::Stop] {
        let field = action.name();
        let names = match find_key(body, field) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::String(name)) => vec![name.clone()],
            Some(Value::Array(names)) if names.iter().all(Value::is_string) => {
                names.iter().map(|n| n.as_str().unwrap().to_owned()).collect()
            },
            Some(_) => return Err(ApiError::field(field, format!("field \"{field}\" must be a counter name or a list of them")))
        };
        if names.iter().any(String::is_empty) {
            return Err(ApiError::field(field, "a counter name cannot be empty"));
        }
        brk.counters.extend(names.into_iter().map(|name| (action, name)));
    }
    brk.after = int_field(body, "after", 0, u32::MAX as i64)?.unwrap_or(0) as u64;
    if brk.run == RunMode::Count && brk.counters.is_empty() {
        return Err(ApiError::field("run", "run \"count\" needs counters to act on: start, stop or reset"));
    }
    Ok(brk)
}

fn read_ram(request: &ApiRequest, motherboard: &Motherboard) -> ApiResult {
    let param = |name: &str| request.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    let int_param = |name: &str, default: i64, min: i64, max: i64| -> Result<i64, ApiError> {
        match param(name) {
            None => Ok(default),
            Some(text) => parse_int(text)
                .filter(|v| (min..=max).contains(v))
                .ok_or_else(|| ApiError::field(name, format!("parameter \"{name}\" must be an integer between {min} and {max}")))
        }
    };
    let addr = int_param("addr", 0, 0, 0xffff)?;
    let len = int_param("len", 256, 1, 0x10000)?;
    let bank = int_param("bank", 0, 0, motherboard.physical_bank_count() as i64 - 1)?;
    let view = param("view").unwrap_or("raw");
    // Past the last bank, the read wraps to the first one.
    let total = motherboard.physical_bank_count() as i64 * 0x4000;
    let byte_at = |i: i64| -> u8 {
        match view {
            "cpu" => motherboard.read_byte((addr + i) as u16),
            _ => {
                let linear = (bank * 0x4000 + addr + i) % total;
                motherboard.read_bank_byte((linear / 0x4000) as u16, (linear % 0x4000) as u16).unwrap_or(0xff)
            }
        }
    };
    if !matches!(view, "raw" | "cpu") {
        return Err(ApiError::field("view", "parameter \"view\" must be one of raw, cpu"));
    }
    let hex: String = (0..len).map(|i| format!("{:02x}", byte_at(i))).collect();
    let mut body = json!({ "addr": addr, "len": len, "hex": hex });
    if view == "cpu" {
        body["view"] = json!("cpu");
    }
    Ok(ApiResponse::ok(body))
}

/// `GET /api/doc[?endpoint=][&format=json|openapi|html]`.
fn doc(request: &ApiRequest) -> ApiResult {
    match (request.param("format").unwrap_or("json"), request.param("endpoint")) {
        ("json", None) => Ok(ApiResponse::ok(api_doc::index())),
        ("json", Some(name)) => match api_doc::endpoint(name) {
            Some(value) => Ok(ApiResponse::ok(value)),
            None => Err(ApiError::field("endpoint", format!("no endpoint \"{name}\" (GET /api/doc lists them)")))
        },
        ("openapi", _) => Ok(ApiResponse::ok(api_doc::openapi())),
        ("html", _) => Ok(ApiResponse {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: api_doc::html().into_bytes(),
            headers: Vec::new()
        }),
        (other, _) => Err(ApiError::field("format", format!("unknown format \"{other}\": json, openapi or html")))
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
        Err(e) => Err(ApiError::new(format!("expected a JSON object: {e}")))
    }
}

/// AMSpiriT looks keys up "flatly, at any depth": `{"a":1}` and
/// `{"x":{"a":1}}` both carry `a`.
fn find_key<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|v| find_key(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None
    }
}

/// An integer field: a JSON integer, or a string in decimal or 0x hex.
fn int_field(body: &Value, name: &str, min: i64, max: i64) -> Result<Option<i64>, ApiError> {
    let value = match find_key(body, name) {
        None => return Ok(None),
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(text)) => parse_int(text),
        Some(_) => None
    }
    .ok_or_else(|| ApiError::field(name, format!("field \"{name}\" must be an integer")))?;
    if !(min..=max).contains(&value) {
        return Err(ApiError::field(name, format!("field \"{name}\" must be between {min} and {max}")));
    }
    Ok(Some(value))
}

fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok()
    }
}

/// Hex bytes; spaces and ':' are ignored.
fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<char> = text.chars().filter(|c| !c.is_whitespace() && *c != ':').collect();
    if digits.len() % 2 != 0 {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(&pair.iter().collect::<String>(), 16).ok())
        .collect()
}

// --------------------------------------------------------------- HTTP

/// The HTTP thread: parse, hand to the emulation thread, write the answer.
fn serve(server: tiny_http::Server, sender: Sender<(ApiRequest, Sender<ApiResponse>)>, events: EventHub) {
    for mut request in server.incoming_requests() {
        let url = request.url().to_owned();
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        if path == "/api/events" && *request.method() == tiny_http::Method::Get {
            let topics = query
                .split('&')
                .find_map(|pair| pair.strip_prefix("topics="))
                .map(|list| list.split(',').filter(|t| !t.is_empty()).map(str::to_owned).collect());
            match events.subscribe(topics) {
                Some(queue) => {
                    let writer = request.into_writer();
                    let _ = std::thread::Builder::new()
                        .name("debug-server-events".to_owned())
                        .spawn(move || stream_events(writer, queue));
                },
                None => {
                    let body = json!({ "error": format!("too many event clients (max {MAX_SSE_CLIENTS})") });
                    let _ = request.respond(tiny_http::Response::from_string(body.to_string()).with_status_code(503));
                }
            }
            continue;
        }
        let query = query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (percent_decode(k), percent_decode(v))
            })
            .collect();
        let mut body = Vec::new();
        let _ = request.as_reader().read_to_end(&mut body);
        let api_request = ApiRequest {
            method: request.method().as_str().to_uppercase(),
            path: path.to_owned(),
            query,
            body
        };
        let (reply_sender, reply) = mpsc::channel();
        let response = match sender.send((api_request, reply_sender)) {
            Ok(()) => reply
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|_| ApiResponse::json(503, json!({ "error": "the emulation did not answer" }))),
            Err(_) => return // the emulation is gone
        };
        let mut http = tiny_http::Response::from_data(response.body)
            .with_status_code(response.status)
            .with_header(header("Content-Type", response.content_type))
            .with_header(header("Access-Control-Allow-Origin", "*"))
            .with_header(header("Cache-Control", "no-store"));
        for (name, value) in &response.headers {
            http = http.with_header(header(name, value));
        }
        let _ = request.respond(http);
    }
}

/// One SSE client's connection: HTTP/1.1 headers, then the events as they
/// come, a `: ping` comment after 15s of silence.
fn stream_events(mut writer: Box<dyn Write + Send>, queue: Arc<(Mutex<SseQueue>, Condvar)>) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
                Access-Control-Allow-Origin: *\r\nConnection: keep-alive\r\n\r\n";
    let mut ok = writer.write_all(head.as_bytes()).and_then(|()| writer.flush()).is_ok();
    while ok {
        let (lock, ready) = &*queue;
        let pending: Vec<String> = {
            let guard = lock.lock().unwrap();
            let (mut guard, _) = ready.wait_timeout_while(guard, SSE_HEARTBEAT, |q| q.events.is_empty()).unwrap();
            guard.events.drain(..).collect()
        };
        ok = if pending.is_empty() {
            writer.write_all(b": ping\n\n")
        }
        else {
            pending.iter().try_for_each(|event| writer.write_all(event.as_bytes()))
        }
        .and_then(|()| writer.flush())
        .is_ok();
    }
    queue.0.lock().unwrap().closed = true;
}

fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::motherboard::MotherboardBuilder;

    fn machine() -> Motherboard {
        MotherboardBuilder::default().disable_lower_rom().disable_upper_rom().build()
    }

    fn call(server: &mut DebugServer, motherboard: &mut Motherboard, method: &str, path: &str, body: &str) -> (u16, Value) {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let query = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap();
                (k.to_owned(), v.to_owned())
            })
            .collect();
        let request = ApiRequest { method: method.to_owned(), path: path.to_owned(), query, body: body.as_bytes().to_vec() };
        let response = server.handle(&request, motherboard);
        (response.status, serde_json::from_slice(&response.body).unwrap())
    }

    fn server() -> DebugServer {
        DebugServer::with_requests(mpsc::channel().1)
    }

    #[test]
    fn memmap_and_ga_follow_the_mmr_and_the_inks() {
        let (mut server, mut mb) = (server(), machine());
        // LD BC,0x7FC4: OUT (C),C (page 0 bank 0 at 0x4000);
        // LD C,0: OUT (C),C (pen 0); LD C,0x54: OUT (C),C (hardware 20).
        mb.write_buffer(0x4000, &[
            0x01, 0xc4, 0x7f, 0xed, 0x49, 0x0e, 0x00, 0xed, 0x49, 0x0e, 0x54, 0xed, 0x49
        ]);
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
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
        assert!(memmap["regions"][0].get("ram_bank").is_none(), "only for RAM, as in AMSpiriT");
        assert_eq!(memmap["regions"][0]["write_bank"], 0, "writes go to the RAM below");
        assert_eq!(memmap["regions"][1]["name"], "4000");
        assert_eq!(memmap["regions"][3]["rom"], true);
        assert_eq!(memmap["regions"][3]["rom_bank"], 0);
    }

    fn beam(x: usize, line: usize) -> Beam {
        Beam { x, line, crop_x: 126, crop_y: 33 }
    }

    #[test]
    fn raster_breakpoints_fire_when_the_beam_passes() {
        let at = |line, nop| RasterBreakpoint { line, x: nop * BEAM_TICKS_PER_NOP };
        // Line 100, NOP 10.
        assert!(at(Some(100), 10).passed(beam(150, 100), beam(170, 100)));
        assert!(at(Some(100), 10).passed(beam(150, 100), beam(160, 100)));
        assert!(!at(Some(100), 10).passed(beam(160, 100), beam(200, 100)), "already passed");
        assert!(!at(Some(100), 10).passed(beam(150, 99), beam(170, 99)));
        // The start of a line, reached through the end of the previous.
        assert!(at(Some(100), 0).passed(beam(1000, 99), beam(30, 100)));
        // Through a new frame.
        assert!(at(Some(0), 1).passed(beam(1000, 311), beam(40, 0)));
        assert!(at(Some(311), 63).passed(beam(1000, 311), beam(40, 0)), "the end of the frame");
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
        let (status, set) = call(&mut server, &mut mb, "POST", "/api/raster_bp?x=200&y=50", "");
        assert_eq!(status, 200);
        assert_eq!(set["breakpoints"][0]["frame_nop"], 50 * 64 + 12);
        call(&mut server, &mut mb, "POST", "/api/raster_bp?frame_nop=19967&add=1", "");
        call(&mut server, &mut mb, "POST", "/api/raster_bp?nop=5&add=1", "");
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/raster_bp", "");
        assert_eq!(list["breakpoints"][1]["line"], 311);
        assert_eq!(list["breakpoints"][1]["nop"], 63);
        assert!(list["breakpoints"][2].get("line").is_none(), "{list}");
        // Picture coordinates need the beam.
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/raster_bp?x=0&y=0&crop=1", "").0, 400);
        server.set_beam(beam(0, 0));
        let (_, set) = call(&mut server, &mut mb, "POST", "/api/raster_bp?x=2&y=0&crop=1", "");
        assert_eq!(set["breakpoints"].as_array().unwrap().len(), 1, "replaces");
        assert_eq!(set["breakpoints"][0]["x"], 128);
        assert_eq!(set["breakpoints"][0]["line"], 33);
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/raster_bp?nop=64", "").0, 400);
        let (_, cleared) = call(&mut server, &mut mb, "POST", "/api/raster_bp?enable=0", "");
        assert_eq!(cleared["breakpoints"], json!([]));
        let (_, position) = call(&mut server, &mut mb, "GET", "/api/beam?crop=1", "");
        assert_eq!((position["x"].clone(), position["y"].clone()), (json!(-126), json!(-33)));
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
            server.set_beam(Beam { x, line, crop_x, crop_y });
            server.after_instruction(mb);
        };
        // Let the firmware set the CRTC up and the monitor lock.
        for _ in 0..4 * 19968 / 3 {
            step(&mut server, &mut mb);
        }
        call(&mut server, &mut mb, "POST", "/api/raster_bp?line=200&nop=20", "");
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
        let stop = events_of(&events).into_iter().find(|e| e.starts_with("event: z80_bp")).unwrap();
        assert!(stop.contains(r#""kind":"raster""#) && stop.contains(r#""line":200"#), "{stop}");
    }

    #[test]
    fn counters_count_nops_while_running() {
        let (mut server, mut mb) = (server(), machine());
        // NOP; NOP; LD A,1 (2 NOPs); JR -5 (3 NOPs).
        mb.write_buffer(0x4000, &[0x00, 0x00, 0x3e, 0x01, 0x18, 0xfa]);
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        server.after_instruction(&mut mb);
        let (status, _) = call(&mut server, &mut mb, "POST", "/api/counters", r#"{"name":"loop"}"#);
        assert_eq!(status, 200);
        call(&mut server, &mut mb, "POST", "/api/counters", r#"{"name":"idle","running":false}"#);
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/counters", r#"{"name":"loop"}"#).0, 400, "exists");
        run(&mut server, &mut mb, 4);
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(list["counters"][0], json!({ "name": "idle", "nops": 0, "running": false }));
        assert_eq!(list["counters"][1]["nops"], 7, "{list}");
        call(&mut server, &mut mb, "POST", "/api/counters", r#"{"name":"loop","action":"stop"}"#);
        run(&mut server, &mut mb, 4);
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(list["counters"][1]["nops"], 7);
        call(&mut server, &mut mb, "POST", "/api/counters", r#"{"name":"loop","action":"reset"}"#);
        call(&mut server, &mut mb, "POST", "/api/counters", r#"{"name":"loop","action":"start"}"#);
        run(&mut server, &mut mb, 2);
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(list["counters"][1]["nops"], 2, "{list}");
        // In the stop events.
        let events = server.events.subscribe(None).unwrap();
        call(&mut server, &mut mb, "POST", "/api/z80_bp", "0x4000");
        run(&mut server, &mut mb, 4);
        let stop = events_of(&events).into_iter().find(|e| e.starts_with("event: z80_bp")).unwrap();
        assert!(stop.contains(r#""counters":{"idle":0,"loop":"#), "{stop}");
        assert_eq!(call(&mut server, &mut mb, "DELETE", "/api/counters?name=idle", "").0, 200);
        assert_eq!(call(&mut server, &mut mb, "DELETE", "/api/counters?name=idle", "").0, 400);
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/counters", r#"{"name":"loop","action":"x"}"#).0, 400);
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
        let beam = Beam { x: 10 * BEAM_TICKS_PER_NOP + 3, line: 100, crop_x: 0, crop_y: 0 };
        let context = MachineContext { motherboard: &mb, access: None, beam: Some(beam) };
        let holds = |source: &str| Condition::parse(source).unwrap().holds(&context);
        assert!(holds("FlagZ AND FlagC AND NOT FlagNZ AND FlagNC = 0"));
        assert!(holds("FlagP = 1 AND FlagM = 0 AND FlagPO = 1"));
        // The firmware's CRTC set-up.
        assert!(holds("CrtcR0 = 63 AND CrtcR1 = 40 AND CrtcR2 = 46 AND CrtcR6 = 25 AND CrtcR7 = 30"));
        assert!(holds("C0 <= CrtcR0 AND C4 <= CrtcR4 AND C9 <= CrtcR9"));
        assert!(holds("LineCount = 100 AND LineCycle = 10"));
        assert!(holds("MemBankMode = 0 AND MemPage = 0"));
        // BASIC running: the upper ROM (BASIC, ROM 0) paged in at times; the
        // number is read, whatever it is now.
        assert!(holds(&format!("UpperROMNum = {}", mb.upper_rom_number())));
        assert!(holds("ASICPageOn = 0 AND ASICPageOff = 1 AND LowerROMNum = 0 AND LowerROMAdr = 0"));
        // No monitor: no beam, the raster variables are 0.
        let headless = MachineContext { motherboard: &mb, access: None, beam: None };
        assert!(!Condition::parse("LineCount OR LineCycle").unwrap().holds(&headless));
    }

    #[test]
    fn basic_programs_load_list_and_stop_on_lines_and_statements() {
        for model in crate::motherboard::CpcModel::ALL {
            basic_program_on(model);
        }
    }

    fn basic_program_on(model: crate::motherboard::CpcModel) {
        let (mut server, mut mb) = (server(), booted_model(model));
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/ping", "").1["emu"]["cpc_model"], model.name());
        let events = server.events.subscribe(Some(vec!["basic_bp".to_owned()])).unwrap();
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(state["prog_start"], 368, "{state}");
        assert_eq!(state["basic_ver"], model.basic_version());
        assert_eq!(state["prog_size"], 2, "an empty program: {state}");
        assert_eq!(state["cur_linenum"], 0xffff);

        let source = "10 A=1:B=2\n20 A=A+1:GOTO 20\n";
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/basic?run=1", "10 GOTO GOTO").0, 400);
        let (status, _) = call(&mut server, &mut mb, "POST", "/api/basic?run=1", source);
        assert_eq!(status, 200);
        let (_, listing) = call(&mut server, &mut mb, "GET", "/api/basic_listing", "");
        let line10 = &listing["lines"][0];
        assert_eq!((line10["addr"].clone(), line10["num"].clone()), (json!(368), json!(10)), "{listing}");
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
        assert!(server.paused(), "{:?}", call(&mut server, &mut mb, "GET", "/api/basic_state", ""));
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(state["cur_linenum"], 20);
        assert_eq!(state["stmt_addr"], line20["stmts"][0]["addr"]);
        let hits = events_of(&events);
        assert!(hits[0].contains(r#""line":20"#) && hits[0].contains(r#""reason":"breakpoint""#), "{hits:?}");

        // Statement by statement, then line by line (the same line again).
        call(&mut server, &mut mb, "POST", "/api/basic_bp", "");
        call(&mut server, &mut mb, "POST", "/api/basic_step", "");
        run(&mut server, &mut mb, 100_000);
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(state["stmt_addr"], goto, "{state}");
        call(&mut server, &mut mb, "POST", "/api/basic_step?mode=line", "");
        run(&mut server, &mut mb, 100_000);
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!((state["cur_linenum"].clone(), state["stmt_addr"].clone()), (json!(20), line20["stmts"][0]["addr"].clone()));
        call(&mut server, &mut mb, "POST", &format!("/api/basic_runto?addr={goto}"), "");
        run(&mut server, &mut mb, 100_000);
        assert!(server.paused());
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/basic_state", "").1["stmt_addr"], goto);
        assert!(events_of(&events).iter().any(|e| e.contains(r#""reason":"runto""#)));

        // The variables are where the state says.
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert!(state["var_size"].as_u64().unwrap() > 0, "{state}");
        let response = server.handle(
            &ApiRequest { method: "GET".into(), path: "/api/basic_export".into(), query: vec![], body: vec![] },
            &mut mb
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
        call(&mut server, &mut mb, "POST", "/api/keytype", r#"{"text":"RUN\r"}"#);
        call(&mut server, &mut mb, "POST", "/api/basic_bp", "10");
        run(&mut server, &mut mb, 4_000_000);
        assert!(server.paused(), "line 10 reached");
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!((state["prog_start"].clone(), state["cur_linenum"].clone()), (json!(368), json!(10)), "{state}");
    }

    #[test]
    fn basic_after_a_reset_loads_once_booted() {
        let (mut server, mut mb) = (server(), booted());
        let events = server.events.subscribe(Some(vec!["reset".to_owned()])).unwrap();
        let (status, _) = call(&mut server, &mut mb, "POST", "/api/basic?reset=1&run=1", "10 DIM X(10)\n20 GOTO 20\n");
        assert_eq!(status, 200);
        assert_eq!(events_of(&events).len(), 1);
        call(&mut server, &mut mb, "POST", "/api/basic_bp", "20");
        run(&mut server, &mut mb, 6_000_000);
        assert!(server.paused());
        let (_, state) = call(&mut server, &mut mb, "GET", "/api/basic_state", "");
        assert_eq!(state["cur_linenum"], 20, "{state}");
        let (txttop, vartop, arrend) =
            (state["txttop"].as_u64().unwrap(), state["vartop"].as_u64().unwrap(), state["arrend"].as_u64().unwrap());
        assert!(txttop > 368 + 2 && vartop >= txttop, "{state}");
        assert!(arrend >= vartop + 11 * 5, "the array of 11 reals: {state}");
    }

    #[test]
    fn breakpoints_drive_the_counters_and_count_their_hits() {
        let (mut server, mut mb) = (server(), machine());
        // LD B,10; loop: NOP; NOP; DJNZ loop; JR $
        mb.write_buffer(0x4000, &[0x06, 0x0a, 0x00, 0x00, 0x10, 0xfc, 0x18, 0xfe]);
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        let add = |server: &mut DebugServer, mb: &mut Motherboard, body: &str| call(server, mb, "POST", "/api/breakpoints", body);
        assert_eq!(add(&mut server, &mut mb, r#"{"addr":"0x4002","run":"count","reset":"loop","start":"loop","after":0}"#).0, 200);
        // Only reset at the first pass: from the second on, keep counting.
        call(&mut server, &mut mb, "DELETE", "/api/breakpoints", "");
        add(&mut server, &mut mb, r#"{"addr":"0x4000","run":"count","reset":"loop"}"#);
        add(&mut server, &mut mb, r#"{"addr":"0x4002","run":"count","start":"loop"}"#);
        add(&mut server, &mut mb, r#"{"addr":"0x4006","stop":"loop","name":"done"}"#);
        let events = server.events.subscribe(None).unwrap();
        run(&mut server, &mut mb, 100);
        assert!(server.paused());
        assert_eq!(mb.pc(), 0x4006);
        let (_, counters) = call(&mut server, &mut mb, "GET", "/api/counters", "");
        assert_eq!(counters["counters"][0], json!({ "name": "loop", "nops": 9 * 6 + 5, "running": false }), "{counters}");
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/breakpoints", "");
        assert_eq!(list["advanced"][1]["hits"], 10);
        assert_eq!(list["advanced"][1]["counters"], json!([{ "action": "start", "name": "loop" }]));
        let all = events_of(&events);
        assert!(!all.iter().any(|e| e.starts_with("event: watch")), "count mode is silent");
        let stop = all.iter().find(|e| e.starts_with("event: z80_bp")).unwrap();
        assert!(stop.contains(r#""counters":{"loop":59}"#) && stop.contains(r#""hits":1"#), "{stop}");

        // A pass count: stop at the 5th hit inside the loop.
        call(&mut server, &mut mb, "DELETE", "/api/breakpoints", "");
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        add(&mut server, &mut mb, r#"{"addr":"0x4003","after":4}"#);
        call(&mut server, &mut mb, "POST", "/api/config", r#"{"paused":false}"#);
        run(&mut server, &mut mb, 100);
        assert_eq!((mb.pc(), mb.cpu().reg8(crate::z80::Reg8::B).value()), (0x4003, 6));
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/breakpoints", "").1["advanced"][0]["hits"], 5);

        assert_eq!(add(&mut server, &mut mb, r#"{"addr":1,"run":"count"}"#).0, 400, "count without counters");
        assert_eq!(add(&mut server, &mut mb, r#"{"addr":1,"start":5}"#).0, 400);
    }

    /// cpclib-basic's tokenisation (what `POST /api/basic` loads) against
    /// the ROM's own, the line typed on a booted 6128: a program loaded
    /// through the debug server is the one the CPC would have stored.
    #[test]
    fn cpclib_tokenises_as_the_rom() {
        for line in ["10 ON ERROR GOTO 30", "20 ERROR 5", "30 GOTO 30", "40 A=1:PRINT A", "50 DATA 1,2,300", "60 X=RND*100"] {
            let (mut server, mut mb) = (self::server(), booted());
            call(&mut server, &mut mb, "POST", "/api/keytype", &format!(r#"{{"text":"{line}\r"}}"#));
            run(&mut server, &mut mb, 1_500_000);
            let ours = cpclib::basic::BasicProgram::parse(format!("{line}\n")).unwrap().as_bytes();
            let length = u16::from_le_bytes([mb.read_video_ram_byte(0x170), mb.read_video_ram_byte(0x171)]) as usize;
            let typed: Vec<u8> = (0x170..0x170 + length as u16 + 2).map(|a| mb.read_video_ram_byte(a)).collect();
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
            call(&mut server, &mut mb, "POST", "/api/basic?run=1", "10 ERROR 5\n");
            run(&mut server, &mut mb, 3_000_000);
            assert_eq!(flag(&mb, 1), 5, "{}: ERR", model.name());
            // AUTO keeps its next line and step after its flag: typed on a
            // fresh machine, both 10.
            let (mut auto_server, mut auto_mb) = (self::server(), booted_model(model));
            call(&mut auto_server, &mut auto_mb, "POST", "/api/keytype", r#"{"text":"AUTO\r"}"#);
            run(&mut auto_server, &mut auto_mb, 4_000_000);
            let auto = rom.flags[0];
            let fields = [auto + 1, auto + 3].map(|a| auto_mb.read_video_ram_byte(a));
            assert_eq!(fields, [10, 10], "{}: AUTO line and step after {auto:#06x}", model.name());
        }
    }

    #[test]
    fn soft_resets_keep_the_ram_hard_ones_clear_it() {
        let (mut server, mut mb) = (server(), MotherboardBuilder::default().build());
        mb.write_bank_byte(1, 0x10, 0x42);
        mb.write_bank_byte(5, 0x10, 0x43);
        call(&mut server, &mut mb, "POST", "/api/config", r#"{"do_soft_reset":true}"#);
        assert_eq!((mb.pc(), mb.read_bank_byte(1, 0x10), mb.read_bank_byte(5, 0x10)), (0, Some(0x42), Some(0x43)));
        call(&mut server, &mut mb, "POST", "/api/config", r#"{"do_hard_reset":true}"#);
        assert_eq!((mb.read_bank_byte(1, 0x10), mb.read_bank_byte(5, 0x10)), (Some(0), Some(0)));
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
        let request = ApiRequest { method: "PUT".into(), path: "/api/ram".into(), query: vec![], body: Vec::new() };
        let response = server.handle(&request, &mut mb);
        assert_eq!(response.status, 405);
        assert_eq!(response.header("allow"), Some("GET, POST"));
    }

    #[test]
    fn registers_round_trip_and_partial_update() {
        let (mut server, mut mb) = (server(), machine());
        let (status, body) = call(&mut server, &mut mb, "POST", "/api/z80", r#"{"PC":"0x4000","A":62,"IM":2}"#);
        assert_eq!((status, body["ok"].clone()), (200, json!(true)));
        let (_, z80) = call(&mut server, &mut mb, "GET", "/api/z80", "");
        assert_eq!((z80["PC"].clone(), z80["A"].clone(), z80["IM"].clone()), (json!(0x4000), json!(62), json!(2)));
        let (status, error) = call(&mut server, &mut mb, "POST", "/api/z80", r#"{"A":256}"#);
        assert_eq!((status, error["field"].clone()), (400, json!("A")));
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/z80", r#"{}"#).0, 400);
    }

    #[test]
    fn ram_write_then_read_and_nested_keys() {
        let (mut server, mut mb) = (server(), machine());
        let (status, body) =
            call(&mut server, &mut mb, "POST", "/api/ram", r#"{"write":{"addr":4096,"data":"3e 01:c9"}}"#);
        assert_eq!((status, body["seq"].clone()), (200, json!(1)));
        let (_, read) = call(&mut server, &mut mb, "GET", "/api/ram?addr=0x1000&len=3", "");
        assert_eq!(read["hex"], "3e01c9");
        let (_, cpu) = call(&mut server, &mut mb, "GET", "/api/ram?addr=4096&len=3&view=cpu", "");
        assert_eq!((cpu["hex"].clone(), cpu["view"].clone()), (json!("3e01c9"), json!("cpu")));
        let (status, error) = call(&mut server, &mut mb, "GET", "/api/ram?len=0", "");
        assert_eq!((status, error["field"].clone()), (400, json!("len")));
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/ram", "RUN").0, 400);
    }

    #[test]
    fn step_runs_one_instruction_then_pauses() {
        let (mut server, mut mb) = (server(), machine());
        mb.write_buffer(0x4000, &[0x3e, 0x42, 0x00]); // LD A,0x42; NOP
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        call(&mut server, &mut mb, "POST", "/api/config", r#"{"paused":true}"#);
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
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/z80_bp", "0x4002, 16387").0, 200);
        for _ in 0..2 {
            mb.execute_next_opcode_no_video();
            server.after_instruction(&mut mb);
        }
        assert!(server.paused());
        assert_eq!(server.take_stop(), Some(0x4002));
        let bank_count = mb.physical_bank_count();
        let missing = format!("B{bank_count:02X}:0000");
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/z80_bp", &missing).0, 400);
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

    #[test]
    fn cpu_addresses_resolve_to_what_is_mapped() {
        let mut server = server();
        let mut mb = MotherboardBuilder::default().disable_upper_rom().build(); // lower ROM on
        let (_, body) = call(&mut server, &mut mb, "POST", "/api/z80_bp", "0x38,0x4002,B04:4001");
        let names: Vec<_> = body["resolved"].as_array().unwrap().iter().map(|r| r["location"].clone()).collect();
        // B04 + 0x4001 carries into B05.
        assert_eq!(names, vec![json!("LOWER:0038"), json!("B01:0002"), json!("B05:0001")]);
        let (_, deprecated) = call(&mut server, &mut mb, "POST", "/api/z80_bp", "C4:0200");
        assert_eq!(deprecated["resolved"][0]["location"], "B04:0200");
        assert!(deprecated["warnings"].is_array());
    }

    #[test]
    fn a_physical_breakpoint_fires_only_on_its_bank() {
        let (mut server, mut mb) = (server(), machine());
        mb.write_buffer(0x4000, &[0x00, 0x00, 0x00, 0x18, 0xfb]); // NOPs; JR 0x4000
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
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
        call(&mut server, &mut mb, "POST", "/api/ram", r#"{"bank":4,"addr":0,"data":"a5"}"#);
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/ram?bank=4&addr=0&len=1", "").1["hex"], "a5");
        assert_ne!(call(&mut server, &mut mb, "GET", "/api/ram?addr=0&len=1", "").1["hex"], "a5");
        // addr past 0x3FFF carries into the next bank.
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/ram?bank=3&addr=0x4000&len=1", "").1["hex"], "a5");
        let beyond = format!("/api/ram?bank={}", mb.physical_bank_count());
        assert_eq!(call(&mut server, &mut mb, "GET", &beyond, "").0, 400);
    }

    #[test]
    fn memory_write_breakpoints_stop_or_watch() {
        let (mut server, mut mb) = (server(), machine());
        let events = server.events.subscribe(None).unwrap();
        // LD A,0x81; LD (0xC005),A; NOP; JR -6
        mb.write_buffer(0x4000, &[0x3e, 0x81, 0x32, 0x05, 0xc0, 0x00, 0x18, 0xf8]);
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        let (status, added) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/breakpoints",
            r#"{"kind":"mem","access":"w","run":"watch","addr":"0xC000","size":16,"name":"screen"}"#
        );
        assert_eq!((status, added["index"].clone()), (200, json!(0)));
        run(&mut server, &mut mb, 2);
        assert!(!server.paused(), "watch only reports");
        let watch: Vec<_> = events_of(&events).into_iter().filter(|e| e.starts_with("event: watch")).collect();
        assert_eq!(watch.len(), 1);
        assert!(watch[0].contains(r#""addr":"0xC005""#) && watch[0].contains(r#""name":"screen""#), "{}", watch[0]);

        // Now stop, written as rasm/basm store it in REMU.
        call(&mut server, &mut mb, "DELETE", "/api/breakpoints", "");
        let (status, _) =
            call(&mut server, &mut mb, "POST", "/api/breakpoints", r#"{"remu":"MEM,W,STOP,addr=49157,value=129,valmask=255"}"#);
        assert_eq!(status, 200);
        run(&mut server, &mut mb, 4);
        assert!(server.paused());
        assert_eq!(mb.pc(), 0x4005, "stops after the writing instruction");
        let stops = events_of(&events);
        assert!(stops.iter().any(|e| e.starts_with("event: z80_bp") && e.contains(r#""kind":"mem""#)), "{stops:?}");
        assert!(stops.iter().any(|e| e.starts_with("event: pause") && e.contains("true")), "{stops:?}");
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/breakpoints", "");
        assert_eq!(list["advanced"][0]["addr"], 0xc005);
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/breakpoints", r#"{"addr":1,"condition":"A=="}"#).0, 400);
    }

    #[test]
    fn conditions_select_the_hits() {
        let (mut server, mut mb) = (server(), machine());
        // INC A; LD (0xC000),A; JR -5 - writes 1, 2, 3...
        mb.write_buffer(0x4000, &[0x3c, 0x32, 0x00, 0xc0, 0x18, 0xfa]);
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        let (status, _) = call(
            &mut server,
            &mut mb,
            "POST",
            "/api/breakpoints",
            r#"{"kind":"mem","access":"w","addr":"0xC000","condition":"value == 3 && A = 3 && peek(0xC000) = 3"}"#
        );
        assert_eq!(status, 200);
        run(&mut server, &mut mb, 100);
        assert!(server.paused());
        assert_eq!(mb.read_byte(0xc000), 3);
        let (_, list) = call(&mut server, &mut mb, "GET", "/api/breakpoints", "");
        assert_eq!(list["advanced"][0]["condition"], "value == 3 && A = 3 && peek(0xC000) = 3");
    }

    #[test]
    fn reset_and_resume_events() {
        let (mut server, mut mb) = (server(), machine());
        let events = server.events.subscribe(Some(vec!["reset".to_owned()])).unwrap();
        call(&mut server, &mut mb, "POST", "/api/config", r#"{"do_hard_reset":true,"paused":true}"#);
        server.poll(&mut mb);
        // Filtered on "reset": the pause event is not delivered.
        assert_eq!(events_of(&events), vec!["event: reset\ndata: {\"hard\":true}\n\n".to_owned()]);
    }

    #[test]
    fn history_keeps_the_bytes_as_fetched() {
        let (mut server, mut mb) = (server(), machine());
        // LD A,7; XOR A; LD (0x4001),A - the last one zeroes the operand of the first.
        mb.write_buffer(0x4000, &[0x3e, 0x07, 0xaf, 0x32, 0x01, 0x40, 0x00]);
        call(&mut server, &mut mb, "POST", "/api/exec", r#"{"addr":16384}"#);
        server.after_instruction(&mut mb); // what a loop does once before running
        run(&mut server, &mut mb, 3);
        let (_, history) = call(&mut server, &mut mb, "GET", "/api/history", "");
        let history = history.as_array().unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!((history[0]["pc"].clone(), history[0]["hex"].clone()), (json!(0x4000), json!("3E07AF32")));
        assert_eq!(history[0]["current_hex"], "3E00AF32");
        assert_eq!(history[2]["pc"], 0x4003);
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/history?n=1", "").1.as_array().unwrap().len(), 1);
        call(&mut server, &mut mb, "POST", "/api/config", r#"{"history_size":1}"#);
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/history", "").1.as_array().unwrap().len(), 0);
    }

    #[test]
    fn screenshots_whole_live_and_composite() {
        use crate::monitor::{FULL_FRAME_HEIGHT, FULL_FRAME_WIDTH};
        let (mut server, mut mb) = (server(), MotherboardBuilder::default().build());
        let mut monitor = Monitor::default();
        let shot = |server: &mut DebugServer, mb: &mut Motherboard, monitor: Option<&Monitor>, query: &str| {
            let query = query
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (k, v) = p.split_once('=').unwrap();
                    (k.to_owned(), v.to_owned())
                })
                .collect();
            let request = ApiRequest { method: "GET".into(), path: "/api/screenshot".into(), query, body: Vec::new() };
            let response = server.handle_with(&request, mb, monitor);
            let picture = (response.status == 200)
                .then(|| image::load_from_memory(&response.body).unwrap().to_rgb8());
            (response.status, picture, response.header("X-Crop-X").map(str::to_owned))
        };
        assert_eq!(shot(&mut server, &mut mb, Some(&monitor), "crop=0").0, 503, "whole frames not kept");
        monitor.keep_full_frames();
        // Three frames, the firmware's border drawn, then half of one.
        let mut frames = 0;
        let mut step = |server: &mut DebugServer, mb: &mut Motherboard, monitor: &mut Monitor| {
            let mut done = None;
            mb.execute_next_opcode_with_video(|(rgb, sync)| {
                monitor.receive(rgb, sync);
                if done.is_none() {
                    done = monitor.take_frame();
                }
            });
            let (x, line) = monitor.beam();
            let (crop_x, crop_y) = monitor.crop_origin();
            server.set_beam(Beam { x, line, crop_x, crop_y });
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
        assert_eq!((status, whole.dimensions()), (200, (FULL_FRAME_WIDTH, FULL_FRAME_HEIGHT)));
        assert_eq!(origin.as_deref(), Some("0"));
        let visible = shot(&mut server, &mut mb, Some(&monitor), "").1.unwrap();
        assert_eq!(visible.dimensions(), (768, 272));
        // The visible area is inside the whole frame, where the crop says.
        let (cx, cy) = monitor.crop_origin();
        assert_eq!(*whole.get_pixel(cx as u32 + 10, cy as u32 + 100), *visible.get_pixel(10, 100));

        // Live: the half drawn, black below the beam; over the previous frame
        // with full=0.
        let below = 250;
        let live = shot(&mut server, &mut mb, Some(&monitor), "crop=0&live=1").1.unwrap();
        assert_eq!(*live.get_pixel(500, below), image::Rgb([0, 0, 0]), "not drawn yet");
        let composite = shot(&mut server, &mut mb, Some(&monitor), "crop=0&live=1&full=0").1.unwrap();
        assert_eq!(*composite.get_pixel(500, below), *whole.get_pixel(500, below), "the previous frame");
        assert_eq!(*composite.get_pixel(500, 100), *live.get_pixel(500, 100), "what is drawn");
        assert_ne!(*whole.get_pixel(500, below), image::Rgb([0, 0, 0]), "a border is drawn there");

        // Without the monitor: the last frame, and no whole frame.
        assert_eq!(shot(&mut server, &mut mb, None, "crop=0").0, 503);
        assert_eq!(shot(&mut server, &mut mb, None, "live=1").0, 200);
        assert_eq!(shot(&mut server, &mut mb, None, "crop=2").0, 400);
    }

    #[test]
    fn screenshot_is_a_png_of_the_last_frame() {
        let (mut server, mut mb) = (server(), machine());
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/screenshot", "").0, 503);
        server.set_frame(&image::RgbImage::from_pixel(768, 272, image::Rgb([0, 0, 128])));
        let request = ApiRequest { method: "GET".into(), path: "/api/screenshot".into(), query: vec![], body: Vec::new() };
        let response = server.handle(&request, &mut mb);
        assert_eq!((response.status, response.content_type), (200, "image/png"));
        assert_eq!(response.header("X-Crop-W"), Some("768"));
        let png = image::load_from_memory(&response.body).unwrap().to_rgb8();
        assert_eq!(*png.get_pixel(10, 10), image::Rgb([0, 0, 128]));
    }

    fn upload(server: &mut DebugServer, mb: &mut Motherboard, query: &str, body: &[u8]) -> (u16, Value) {
        let query = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap();
                (k.to_owned(), v.to_owned())
            })
            .collect();
        let request = ApiRequest { method: "POST".into(), path: "/api/media".into(), query, body: body.to_vec() };
        let response = server.handle(&request, mb);
        (response.status, serde_json::from_slice(&response.body).unwrap())
    }

    #[test]
    fn media_binaries_with_and_without_header() {
        let (mut server, mut mb) = (server(), machine());
        let (status, reply) = upload(&mut server, &mut mb, "name=game@4000@4001.bin", &[0xc9, 0x00]);
        assert_eq!((status, reply["load"].clone(), reply["len"].clone()), (200, json!(0x4000), json!(2)));
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
        assert_eq!((mb.read_byte(0x8000), mb.read_byte(0x8002), mb.read_byte(0x8003)), (1, 3, 0));
        assert_eq!(mb.pc(), 0x4001, "no @ENTRY in the name: no jump");

        assert_eq!(upload(&mut server, &mut mb, "name=raw.bin", &[0]).0, 400);
        assert_eq!(upload(&mut server, &mut mb, "name=x.cdt", &[0]).0, 400);
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
        assert_eq!((status, reply["media"].clone(), reply["breakpoints"].clone()), (200, json!("sna"), json!(1)));
        assert_eq!(mb.pc(), 0x1234);
        assert_eq!(call(&mut server, &mut mb, "GET", "/api/breakpoints", "").1["z80_bp"], json!(["B01:0000"]));
    }

    #[test]
    fn a_saved_snapshot_goes_back_through_media() {
        let (mut server, mut mb) = (server(), machine());
        mb.cpu_mut().set_pc(0x4321);
        let request = ApiRequest { method: "GET".into(), path: "/api/snapshot".into(), query: vec![], body: Vec::new() };
        let response = server.handle(&request, &mut mb);
        assert_eq!((response.status, response.content_type), (200, "application/octet-stream"));
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
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/keytype", r#"{}"#).0, 400);
        call(&mut server, &mut mb, "POST", "/api/keytype", r#"{"text":"RUN\r"}"#);
        let (_, ping) = call(&mut server, &mut mb, "GET", "/api/ping", "");
        assert_eq!((ping["emu"]["autotyping"].clone(), ping["emu"]["autotype_remaining"].clone()), (json!(true), json!(4)));
    }

    #[test]
    fn requests_reach_the_machine_through_poll() {
        let (sender, receiver) = mpsc::channel();
        let mut server = DebugServer::with_requests(receiver);
        let mut mb = machine();
        let (reply_sender, reply) = mpsc::channel();
        let request = ApiRequest { method: "GET".into(), path: "/api/state".into(), query: vec![], body: Vec::new() };
        sender.send((request, reply_sender)).unwrap();
        server.poll(&mut mb);
        let state: Value = serde_json::from_slice(&reply.recv().unwrap().body).unwrap();
        assert_eq!(state["crtc"]["regs"].as_array().unwrap().len(), 14);
        assert_eq!(state["ga"]["ink_rgb"].as_array().unwrap().len(), 16);
    }
}
