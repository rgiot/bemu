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
//! Implemented so far: `ping`, `state`, `z80` (GET/POST), `ram` (GET/POST),
//! `exec`, `step`, `config` (GET/POST: pause/resume, resets), `z80_bp`
//! (CPU addresses), `keytype`, `doc`. Extensions to the AMSpiriT API are
//! listed in `/api/doc` with `"extension":true`; replies identify this
//! emulator with `"frontend":"bemu"`.

use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::csl_runner::keys_of_char;
use crate::keyboard::CpcKey;
use crate::motherboard::Motherboard;
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
    pub body: String
}

/// The answer to an [`ApiRequest`].
#[derive(Debug, PartialEq)]
pub struct ApiResponse {
    pub status: u16,
    pub body: String,
    /// The `Allow` header of a 405.
    pub allow: Option<String>
}

impl ApiResponse {
    fn json(status: u16, value: Value) -> Self {
        Self { status, body: value.to_string(), allow: None }
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
    Route { path: "/api/z80_bp", methods: &["POST"], summary: "replace the PC breakpoints (comma-separated CPU addresses)", extension: false },
    Route { path: "/api/keytype", methods: &["POST"], summary: "type text on the CPC keyboard (\\r = RETURN)", extension: false },
    Route { path: "/api/doc", methods: &["GET"], summary: "this list", extension: false }
];

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
    breakpoints: HashSet<u16>,
    frames: u64,
    fps: f64,
    fps_window: (Instant, u64),
    typing: VecDeque<Vec<CpcKey>>,
    typed_pressed: Option<Vec<CpcKey>>,
    typing_next_at: usize,
    /// Stops reported (breakpoint hits), for the emulation loop to show.
    last_stop: Option<u16>
}

impl DebugServer {
    /// Listen on `127.0.0.1:port` (loopback only).
    pub fn start(port: u16) -> Result<Self, String> {
        let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| format!("debug server on port {port}: {e}"))?;
        let (sender, requests) = mpsc::channel();
        std::thread::Builder::new()
            .name("debug-server".to_owned())
            .spawn(move || serve(server, sender))
            .map_err(|e| e.to_string())?;
        Ok(Self::with_requests(requests))
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
            breakpoints: HashSet::new(),
            frames: 0,
            fps: 0.0,
            fps_window: (Instant::now(), 0),
            typing: VecDeque::new(),
            typed_pressed: None,
            typing_next_at: 0,
            last_stop: None
        }
    }

    /// Whether the emulation must stay stopped.
    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// The breakpoint the CPU stopped on since the last call, if any.
    pub fn take_stop(&mut self) -> Option<u16> {
        self.last_stop.take()
    }

    /// Answer every pending request.
    pub fn poll(&mut self, motherboard: &mut Motherboard) {
        while let Ok((request, reply)) = self.requests.try_recv() {
            let response = self.handle(&request, motherboard);
            let _ = reply.send(response);
        }
        // Paused: nothing will run after the commands, they are applied.
        if self.paused && !self.stepping {
            self.mark_applied();
        }
    }

    /// To call after each instruction the emulation executes.
    pub fn after_instruction(&mut self, motherboard: &mut Motherboard) {
        self.mark_applied();
        if self.stepping {
            self.stepping = false;
            self.paused = true;
        }
        else if !self.breakpoints.is_empty() && self.breakpoints.contains(&motherboard.pc()) {
            self.paused = true;
            self.last_stop = Some(motherboard.pc());
        }
        self.type_due_keys(motherboard);
    }

    /// To call once per completed video frame.
    pub fn frame_completed(&mut self) {
        self.frames += 1;
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
        let Some(route) = ROUTES.iter().find(|route| route.path == request.path)
        else {
            return ApiResponse::json(404, json!({ "error": "not found", "path": request.path }));
        };
        if !route.methods.contains(&request.method.as_str()) {
            let allow = route.methods.join(", ");
            return ApiResponse {
                status: 405,
                body: json!({ "error": "method not allowed", "allow": route.methods }).to_string(),
                allow: Some(allow)
            };
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
                "paused": self.paused
            }))),
            ("POST", "/api/config") => self.config(request, motherboard),
            ("POST", "/api/z80_bp") => self.set_breakpoints(request),
            ("POST", "/api/keytype") => self.keytype(request),
            ("GET", "/api/doc") => Ok(ApiResponse::ok(doc())),
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
            "cpc_model": "6128",
            "crtc_type": motherboard.crtc().crtc_type() as u8,
            "autotyping": !self.typing.is_empty() || self.typed_pressed.is_some(),
            "autotype_remaining": self.typing.len(),
            "ram_apply_seq": self.ram_seq,
            "applied_cmd_seq": self.applied_cmd_seq
        })
    }

    fn state(&self, motherboard: &Motherboard) -> Value {
        use crate::ga::{RgbLevels, VideoMode};
        let rgb = |levels: RgbLevels| -> u32 {
            let to_u8 = |v: f32| (v * 255.0 / 100.0).round().clamp(0.0, 255.0) as u32;
            to_u8(levels.r()) << 16 | to_u8(levels.g()) << 8 | to_u8(levels.b())
        };
        let palette = motherboard.ga().palette();
        let crtc = motherboard.crtc();
        let psg = motherboard.psg();
        let psg_word = |low: u8| u16::from_le_bytes([psg.register(low), psg.register(low + 1)]);
        json!({
            "z80": z80(motherboard),
            "ga": {
                "mode": match motherboard.ga().active_video_mode() {
                    VideoMode::Mode0 => 0, VideoMode::Mode1 => 1, VideoMode::Mode2 => 2, VideoMode::Mode3 => 3
                },
                "border_idx": palette[16].number(),
                "border_rgb": rgb(palette[16].into()),
                "ink_idx": palette[..16].iter().map(|ink| ink.number()).collect::<Vec<_>>(),
                "ink_rgb": palette[..16].iter().map(|&ink| rgb(ink.into())).collect::<Vec<_>>()
            },
            "crtc": {
                "regs": (0..14).map(|r| crtc.do_read_register(r)).collect::<Vec<_>>(),
                "selected_reg": crtc.selected_register()
            },
            "psg": {
                "period_a": psg_word(0) & 0xfff, "vol_a": psg.register(8) & 0x1f,
                "period_b": psg_word(2) & 0xfff, "vol_b": psg.register(9) & 0x1f,
                "period_c": psg_word(4) & 0xfff, "vol_c": psg.register(10) & 0x1f,
                "mixer": psg.register(7), "noise": psg.register(6) & 0x1f,
                "env_period": psg_word(11), "env_shape": psg.register(13) & 0x0f,
                // Extension: the 16 registers as they are.
                "regs": (0..16).map(|r| psg.register(r)).collect::<Vec<_>>()
            },
            "emu": self.emu(motherboard)
        })
    }

    // --------------------------------------------------------- commands

    fn set_z80(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let body = parse_body(&request.body)?;
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
        let body = parse_body(&request.body)?;
        let addr = int_field(&body, "addr", 0, 0xffff)?;
        let bank = int_field(&body, "bank", 0, 3)?.unwrap_or(0) as usize;
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
            let linear = bank * 0x4000 + addr + i;
            motherboard.write_base_ram(linear as u16, byte);
        }
        if exec {
            let target = entry.unwrap_or(addr as i64) as u16;
            motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(target);
        }
        self.ram_seq += 1;
        Ok(ApiResponse::ok(json!({ "ok": true, "seq": self.ram_seq, "cmd_seq": self.command() })))
    }

    fn exec(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let body = parse_body(&request.body)?;
        let addr = int_field(&body, "addr", 0, 0xffff)?.ok_or_else(|| ApiError::field("addr", "missing field \"addr\": the new PC"))?;
        motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(addr as u16);
        self.ram_seq += 1;
        Ok(ApiResponse::ok(json!({ "ok": true, "seq": self.ram_seq, "cmd_seq": self.command() })))
    }

    fn config(&mut self, request: &ApiRequest, motherboard: &mut Motherboard) -> ApiResult {
        let body = parse_body(&request.body)?;
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
        if paused.is_none() && !soft && !hard {
            return Err(ApiError::new("no recognised field: supported are paused, do_soft_reset, do_hard_reset"));
        }
        if soft || hard {
            // No separate soft reset on a CPC: both are the RESET line.
            motherboard.reset();
        }
        if let Some(paused) = paused {
            self.paused = paused;
        }
        Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
    }

    fn set_breakpoints(&mut self, request: &ApiRequest) -> ApiResult {
        let mut breakpoints = HashSet::new();
        for entry in request.body.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            if entry.starts_with(['B', 'b', 'C', 'c']) && entry.contains(':') {
                return Err(ApiError::new(format!("{entry}: physical (bank) breakpoints are not supported yet, use a CPU address")));
            }
            let address = parse_int(entry)
                .filter(|v| (0..=0xffff).contains(v))
                .ok_or_else(|| ApiError::new(format!("{entry}: expected a CPU address, decimal or 0x hex, 0-65535")))?;
            breakpoints.insert(address as u16);
        }
        self.breakpoints = breakpoints;
        Ok(ApiResponse::ok(json!({ "ok": true, "cmd_seq": self.command() })))
    }

    fn keytype(&mut self, request: &ApiRequest) -> ApiResult {
        let body = parse_body(&request.body)?;
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
    let bank = int_param("bank", 0, 0, 3)?;
    let view = param("view").unwrap_or("raw");
    let byte_at = |i: i64| -> u8 {
        match view {
            "cpu" => motherboard.read_byte((addr + i) as u16),
            _ => motherboard.read_video_ram_byte((bank * 0x4000 + addr + i) as u16)
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

fn doc() -> Value {
    let endpoints: Vec<Value> = ROUTES
        .iter()
        .map(|route| {
            let mut endpoint = json!({ "path": route.path, "methods": route.methods, "summary": route.summary });
            if route.extension {
                endpoint["extension"] = json!(true);
            }
            endpoint
        })
        .collect();
    json!({ "frontend": "bemu", "endpoints": endpoints })
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
fn serve(server: tiny_http::Server, sender: Sender<(ApiRequest, Sender<ApiResponse>)>) {
    for mut request in server.incoming_requests() {
        let url = request.url().to_owned();
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        let query = query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (k.to_owned(), v.to_owned())
            })
            .collect();
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);
        let api_request = ApiRequest {
            method: request.method().as_str().to_uppercase(),
            path: path.to_owned(),
            query,
            body
        };
        let (reply_sender, reply) = mpsc::channel();
        let response = match sender.send((api_request, reply_sender)) {
            Ok(()) => reply.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| ApiResponse {
                status: 503,
                body: json!({ "error": "the emulation did not answer" }).to_string(),
                allow: None
            }),
            Err(_) => return // the emulation is gone
        };
        let mut http = tiny_http::Response::from_string(response.body)
            .with_status_code(response.status)
            .with_header(header("Content-Type", "application/json"))
            .with_header(header("Access-Control-Allow-Origin", "*"))
            .with_header(header("Cache-Control", "no-store"));
        if let Some(allow) = response.allow {
            http = http.with_header(header("Allow", &allow));
        }
        let _ = request.respond(http);
    }
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
        let request = ApiRequest { method: method.to_owned(), path: path.to_owned(), query, body: body.to_owned() };
        let response = server.handle(&request, motherboard);
        (response.status, serde_json::from_str(&response.body).unwrap())
    }

    fn server() -> DebugServer {
        DebugServer::with_requests(mpsc::channel().1)
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
        let request = ApiRequest { method: "PUT".into(), path: "/api/ram".into(), query: vec![], body: String::new() };
        let response = server.handle(&request, &mut mb);
        assert_eq!(response.status, 405);
        assert_eq!(response.allow.as_deref(), Some("GET, POST"));
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
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/z80_bp", "B04:0200").0, 400);
        assert_eq!(call(&mut server, &mut mb, "POST", "/api/z80_bp", "").0, 200);
        assert!(server.breakpoints.is_empty());
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
        let request = ApiRequest { method: "GET".into(), path: "/api/state".into(), query: vec![], body: String::new() };
        sender.send((request, reply_sender)).unwrap();
        server.poll(&mut mb);
        let state: Value = serde_json::from_str(&reply.recv().unwrap().body).unwrap();
        assert_eq!(state["crtc"]["regs"].as_array().unwrap().len(), 14);
        assert_eq!(state["ga"]["ink_rgb"].as_array().unwrap().len(), 16);
    }
}
