//! The debug server describing itself: each operation's parameters, body,
//! answer and an example, served by `GET /api/doc` (`?endpoint=` for one,
//! `?format=openapi` for an OpenAPI 3 description, `?format=html` for a
//! page to read).
//!
//! Kept beside the handlers by hand; the tests check every route and
//! method has its operation, and that the examples are accepted.

use serde_json::{json, Map, Value};

use super::{Route, ROUTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    Query,
    /// A field of the JSON body.
    Body
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Param {
    pub name: &'static str,
    pub place: Place,
    /// integer, boolean, string, hex (a string of hex bytes).
    pub kind: &'static str,
    pub required: bool,
    pub description: &'static str
}

const fn q(name: &'static str, kind: &'static str, description: &'static str) -> Param {
    Param { name, place: Place::Query, kind, required: false, description }
}

const fn b(name: &'static str, kind: &'static str, description: &'static str) -> Param {
    Param { name, place: Place::Body, kind, required: false, description }
}

impl Param {
    const fn required(self) -> Self {
        Param { required: true, ..self }
    }
}

/// A body that is not a JSON object of [`Param`]s.
#[derive(Debug, Clone, Copy)]
pub(super) struct RawBody {
    pub content_type: &'static str,
    pub description: &'static str
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Example {
    /// Without the `?`.
    pub query: &'static str,
    pub body: &'static str
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Operation {
    pub method: &'static str,
    pub path: &'static str,
    pub description: &'static str,
    pub params: &'static [Param],
    pub raw_body: Option<RawBody>,
    pub response_type: &'static str,
    pub response: &'static str,
    pub example: Option<Example>
}

const JSON: &str = "application/json";
const OK: &str = "{ok: true, cmd_seq}: cmd_seq numbers the commands, for matching the events they cause";

const fn op(method: &'static str, path: &'static str, description: &'static str, response: &'static str) -> Operation {
    Operation {
        method,
        path,
        description,
        params: &[],
        raw_body: None,
        response_type: JSON,
        response,
        example: None
    }
}

impl Operation {
    const fn params(self, params: &'static [Param]) -> Self {
        Operation { params, ..self }
    }

    const fn raw_body(self, content_type: &'static str, description: &'static str) -> Self {
        Operation { raw_body: Some(RawBody { content_type, description }), ..self }
    }

    const fn returns(self, response_type: &'static str) -> Self {
        Operation { response_type, ..self }
    }

    const fn example(self, query: &'static str, body: &'static str) -> Self {
        Operation { example: Some(Example { query, body }), ..self }
    }

    fn has_json_body(&self) -> bool {
        self.params.iter().any(|p| p.place == Place::Body)
    }
}

/// How values are written, for every operation.
const CONVENTIONS: &[&str] = &[
    "Integers in the query or in a JSON body: decimal, or hexadecimal with 0x (a JSON body also takes JSON numbers).",
    "JSON bodies are objects; a field is looked up at any depth ({\"a\":1} and {\"x\":{\"a\":1}} both carry a).",
    "Errors answer 400 with {error, field?}, 404 for an unknown path, 405 (with Allow) for a method a path does not take.",
    "Commands answer cmd_seq; the events they cause (GET /api/events) carry it.",
    "Addresses are CPU addresses (the current mapping) unless a bank is given; Bnn:hhhh names offset hhhh of 16K RAM bank nn (hex)."
];

pub(super) const OPERATIONS: &[Operation] = &[
    op("GET", "/api/ping", "Is the server there; the emulator's description.", "{ok, emu: {cpc_model, crtc_type, paused, frames, fps, ticks (T-states run), applied_cmd_seq, ram_apply_seq, autotyping, autotype_remaining}, frontend: \"bemu\", version}"),
    op(
        "GET",
        "/api/state",
        "Everything at once: the Z80 registers, the Gate Array, the CRTC, the PSG, the FDC, the beam and the emulator.",
        "{z80, ga, crtc, psg, fdc, beam (null without a monitor), emu}: as the endpoints of the same names"
    ),
    op("GET", "/api/z80", "The Z80 registers.", "{PC, SP, IX, IY, A, F, B, C, D, E, H, L, A2..L2, I, R, IFF1, IFF2, IM, ticks}"),
    op("POST", "/api/z80", "Set Z80 registers; all are checked before any changes.", OK)
        .params(&[
            b("PC", "integer", "any 16-bit register by name: PC, SP, AF, BC, DE, HL, IX, IY, AF2, BC2, DE2, HL2"),
            b("A", "integer", "any 8-bit register by name: A, F, B, C, D, E, H, L, the A2..L2 alternates, I, R"),
            b("IFF1", "integer", "0 or 1"),
            b("IFF2", "integer", "0 or 1"),
            b("IM", "integer", "interrupt mode, 0 to 2")
        ])
        .example("", r#"{"BC":"0x7f10","A":84}"#),
    op("GET", "/api/ram", "Read memory as hex.", "{addr, len, hex, view?}")
        .params(&[
            q("addr", "integer", "first byte (default 0)"),
            q("len", "integer", "number of bytes, 1 to 65536 (default 256)"),
            q("bank", "integer", "16K RAM bank addr is counted from, view=raw (default 0); offsets carry into the next banks"),
            q("view", "string", "raw: the RAM banks, whatever is mapped (default); cpu: what the CPU reads now, ROMs included")
        ])
        .example("addr=0xc000&len=16&view=cpu", ""),
    op("POST", "/api/ram", "Write bytes to RAM and/or jump.", "{ok, seq, cmd_seq}: seq counts the memory changes")
        .params(&[
            b("addr", "integer", "where to write (an offset in bank); required with data"),
            b("bank", "integer", "16K RAM bank addr is counted from (default 0)"),
            b("data", "hex", "the bytes, as hex (spaces and : ignored)"),
            b("exec", "boolean", "set PC afterwards (default false)"),
            b("entry", "integer", "the PC exec sets (default addr)")
        ])
        .example("", r#"{"addr":"0x4000","data":"C3 00 40"}"#),
    op("POST", "/api/exec", "Set PC.", "{ok, seq, cmd_seq}")
        .params(&[b("addr", "integer", "the new PC").required()])
        .example("", r#"{"addr":"0x4000"}"#),
    op("POST", "/api/step", "Run one instruction, then pause (a z80_bp event, reason step).", OK),
    op("GET", "/api/config", "Emulator settings.", "{crtc_type, paused, history_size}"),
    op("POST", "/api/config", "Pause or resume, reset, size the instruction history.", OK)
        .params(&[
            b("paused", "boolean", "pause (true) or resume (false)"),
            b("do_soft_reset", "boolean", "the reset button: RAM kept"),
            b("do_hard_reset", "boolean", "power off and on"),
            b("history_size", "integer", "instructions kept for /api/history, 1 to 100000; empties it")
        ])
        .example("", r#"{"paused":true}"#),
    op("POST", "/api/z80_bp", "Replace the execution breakpoints.", "{ok, cmd_seq, resolved: [{entry, location}], warnings?}")
        .raw_body(
            "text/plain",
            "comma-separated: CPU addresses (resolved against the current mapping to the RAM bank or ROM they reach) or \
             Bnn:hhhh (hex bank, hex offset); empty clears"
        )
        .example("", "0x4000, B01:0010"),
    op("GET", "/api/breakpoints", "The execution and advanced breakpoints.", "{z80_bp: [location], advanced: [{index, kind, access, run, addr, mask, size, value, valmask, name, condition, counters, after, hits}]}"),
    op("POST", "/api/breakpoints", "Add an advanced breakpoint (memory, IO or execution; stop, watch or count).", "{ok, index, cmd_seq}")
        .params(&[
            b("remu", "string", "the breakpoint as the text of a REMU acebreak entry, in place of the fields below"),
            b("kind", "string", "exec (default), mem or io"),
            b("access", "string", "r, w or rw (default)"),
            b("run", "string", "stop (default), watch (a watch event, no stop) or count (only acts on counters)"),
            b("addr", "integer", "address or port").required(),
            b("mask", "integer", "bits of the address compared (default 0xffff)"),
            b("size", "integer", "bytes or ports covered from addr (default 1)"),
            b("value", "integer", "byte read or written to match, under valmask"),
            b("valmask", "integer", "bits of value compared (default 0: any value)"),
            b("name", "string", "shown in its events"),
            b(
                "condition",
                "string",
                "ACE's condition language (BASIC-like: A = &12 AND NOT FlagZ, LineCount > 100, CrtcR12 = &30, \
                 MOD, \\, ^ power; registers, flags, CRTC C0..C9 and CrtcR0..17, beam, MemPage...) or C operators \
                 (A == #12 && peek(HL) <> 0, value & %1000)"
            ),
            b("start", "string", "counter name(s) started on a hit (a string or a list)"),
            b("stop", "string", "counter name(s) stopped on a hit"),
            b("reset", "string", "counter name(s) reset on a hit"),
            b("after", "integer", "hits ignored first")
        ])
        .example("", r#"{"kind":"mem","access":"w","run":"watch","addr":"0xc000","size":16,"name":"screen"}"#),
    op("DELETE", "/api/breakpoints", "Remove one advanced breakpoint, or all.", OK)
        .params(&[q("index", "integer", "the one to remove (others are renumbered); none: all")])
        .example("", ""),
    op("POST", "/api/keytype", "Type text on the CPC keyboard, a key at a time.", OK)
        .params(&[b("text", "string", "the characters; \\r or \\n is RETURN").required()])
        .example("", r#"{"text":"PRINT 1\r"}"#),
    op(
        "GET",
        "/api/events",
        "Server-Sent Events: frame (each frame, while running), z80_bp (a stop: breakpoint, step...), pause, reset, watch \
         (a watch breakpoint's hit), basic_bp (a BASIC stop). Each event's data is JSON.",
        "an event stream"
    )
    .params(&[q("topics", "string", "comma-separated event names to receive (default all)")])
    .returns("text/event-stream"),
    op("POST", "/api/quit", "Leave the emulator.", OK),
    op("GET", "/api/history", "The last instructions run, oldest first.", "[{pc, hex, current_hex}]: hex as fetched, current_hex as in memory now")
        .params(&[q("n", "integer", "the last n only, 1 to 100000")])
        .example("n=8", ""),
    op(
        "GET",
        "/api/screenshot",
        "The screen as a PNG. Headers: X-Crop-X/Y/W/H (where the picture is in the beam's 1024x350 raster), \
         X-Beam-X/Y (the beam, in the picture) when known.",
        "a PNG image; 503 while there is no frame yet, or for crop=0 when whole frames are not kept"
    )
    .params(&[
        q("crop", "integer", "1: the visible 768x272 picture (default); 0: the whole 1024x350 raster, borders and syncs included"),
        q("live", "integer", "1: the frame being drawn (default while paused); 0: the last complete frame"),
        q("full", "integer", "with live=1: 1 the drawn part alone (default), 0 drawn over the previous frame")
    ])
    .returns("image/png")
    .example("crop=1&live=0", ""),
    op(
        "POST",
        "/api/media",
        "Load a snapshot (its breakpoints set), insert a disc or a tape (PLAY down), or load a binary.",
        "{ok, media: sna|dsk|cdt|bin, cmd_seq, breakpoints?, warnings?, load?, len?, entry?}"
    )
    .params(&[
        q("name", "string", "the file's name: .bin/.amsdos (an AMSDOS header is used when there is one)"),
        q("drive", "integer", "drive for a disc: 0 (A, default) or 1 (B)"),
        q("crtc", "string", "keep or sna (accepted; the CRTC is kept)")
    ])
    .raw_body("application/octet-stream", "the file: .sna, .dsk and .cdt are recognised by their content"),
    op(
        "GET",
        "/api/snapshot",
        "The machine's state as a version 3 .sna file: CPU, RAM (the main 64K and the extra pages), CRTC with its \
         counters, Gate Array, PSG, PPI, ROM and RAM banking. POST it to /api/media to go back to it.",
        "the .sna file"
    )
    .returns("application/octet-stream"),
    op("GET", "/api/memmap", "What each 16K region of the CPU maps: ROM, or which RAM bank; RMR and MMR.", "{regions: [{base, name, rom, rom_bank?, ram_bank?, ext?, write_bank}], rmr, mmr, ram_mode, ram_page, ram_banks}"),
    op("GET", "/api/crtc", "The CRTC registers, the selected one and the counters.", "{type, regs, selected_reg, hcc, vcc, vlc, vsc, vtac, ma}"),
    op("GET", "/api/ga", "The Gate Array: mode, pen, inks, RMR and banking.", "{mode, pen, ink_idx, ink_rgb, ink_firmware, border_idx, border_rgb, rmr, r52, lower_rom, upper_rom, ram_mode, ram_page}"),
    op("GET", "/api/psg", "The PSG registers, decoded.", "{regs, selected_reg, period_a, period_b, period_c, noise, mixer, vol_a, vol_b, vol_c, env_period, env_shape}"),
    op("GET", "/api/fdc", "The FDC: status registers, phase, motor and heads.", "{msr, sr0, sr1, sr2, phase, command, motor, drives: [{connected, disk, track}]}"),
    op(
        "GET",
        "/api/beam",
        "Where the beam is: x (ticks of 1/16 us) and y (lines) in the 1024x350 raster, and in NOPs.",
        "{x, y, nop, line, frame_nop, cx, cy, cw, ch (the picture in the raster), nops_per_line, nops_per_frame}; 400 without a monitor"
    )
    .params(&[q("crop", "integer", "1: x and y relative to the visible picture")])
    .example("crop=1", ""),
    op("GET", "/api/raster_bp", "The beam breakpoints.", "{breakpoints: [{index, x, nop, y?, line?, frame_nop?}]}"),
    op(
        "POST",
        "/api/raster_bp",
        "Stop when the beam reaches a position: x/y (or with crop=1, in the picture), or nop in line, or frame_nop. \
         nop alone stops in every line, line alone at its start.",
        "{ok, cmd_seq, breakpoints}"
    )
    .params(&[
        q("x", "integer", "beam ticks"),
        q("y", "integer", "raster line"),
        q("crop", "integer", "1: x/y relative to the visible picture"),
        q("nop", "integer", "NOP in the line, 0 to 63"),
        q("line", "integer", "line of the frame, from the VSYNC"),
        q("frame_nop", "integer", "NOP of the frame, 0 to 19967"),
        q("add", "integer", "1: keep the others (default 0: replace them)"),
        q("enable", "integer", "0: clear them all")
    ])
    .example("line=100&nop=10", ""),
    op("GET", "/api/counters", "The NOP counters.", "{counters: [{name, nops, running}]}"),
    op("POST", "/api/counters", "Create, reset, start or stop a counter of the NOPs run.", "{ok, counters: [{name, nops, running}]}")
        .params(&[
            b("name", "string", "the counter").required(),
            b("action", "string", "create (default), reset, start or stop"),
            b("running", "boolean", "on create: start counting now (default true)")
        ])
        .example("", r#"{"name":"loop","action":"create"}"#),
    op("DELETE", "/api/counters", "Delete a counter, or all.", "{ok, counters}")
        .params(&[q("name", "string", "the one to delete; none: all")])
        .example("", ""),
    op("POST", "/api/basic", "Load a BASIC program in place of the one in RAM (variables cleared).", OK)
        .params(&[
            q("reset", "integer", "1: reset first; loaded once the firmware has booted"),
            q("run", "integer", "1: RUN it")
        ])
        .raw_body("text/plain", "the program, as typed (numbered lines)")
        .example("", "10 PRINT \"HELLO\"\n20 GOTO 10\n"),
    op(
        "GET",
        "/api/basic_state",
        "Where the BASIC program and its variables are, the line and statement being run.",
        "{basic_ver, prog_start, txttop, vartop, arrend, prog_size, var_size, chain_heads_addr, cur_linenum (0xffff: direct mode), stmt_addr}"
    ),
    op("GET", "/api/basic_export", "The BASIC program in RAM, as text.", "the listing").returns("text/plain"),
    op(
        "GET",
        "/api/basic_listing",
        "The BASIC program as lines of statements with their addresses (stmt_addr of /api/basic_state names one).",
        "{lines: [{addr, num, stmts: [{addr, end, colon, text, vars}]}]}"
    ),
    op("POST", "/api/basic_step", "Run to the next BASIC statement (a basic_bp event, reason step).", OK)
        .params(&[q("mode", "string", "statement (default) or line: the next line")])
        .example("mode=line", ""),
    op("GET", "/api/basic_bp", "The BASIC line breakpoints.", "{lines}"),
    op("POST", "/api/basic_bp", "Replace the BASIC line breakpoints.", "{ok, lines, cmd_seq}")
        .raw_body("text/plain", "comma-separated line numbers; empty clears")
        .example("", "10, 20"),
    op("POST", "/api/basic_runto", "Run to a BASIC line or statement, once (a basic_bp event, reason runto).", OK)
        .params(&[
            q("line", "integer", "the line number"),
            q("addr", "integer", "a statement's address (as /api/basic_listing gives them)")
        ])
        .example("line=20", ""),
    op(
        "GET",
        "/api/doc",
        "This documentation: the endpoints (default), one in detail, or the whole API as OpenAPI 3 or HTML.",
        "{frontend, conventions, endpoints: [{path, methods, summary, extension?, doc}]}; ?endpoint=: {path, operations}"
    )
    .params(&[
        q("endpoint", "string", "one endpoint in detail: its path, with or without /api/ (ram, /api/ram)"),
        q("format", "string", "json (default), openapi or html")
    ])
    .example("endpoint=ram", "")
];

fn param_json(param: &Param) -> Value {
    let mut value = json!({ "name": param.name, "type": param.kind, "description": param.description });
    if param.required {
        value["required"] = json!(true);
    }
    value
}

fn operation_json(operation: &Operation) -> Value {
    let list = |place| operation.params.iter().filter(|p| p.place == place).map(param_json).collect::<Vec<_>>();
    let mut value = json!({ "method": operation.method, "description": operation.description });
    let query = list(Place::Query);
    if !query.is_empty() {
        value["query"] = json!(query);
    }
    if operation.has_json_body() {
        value["body"] = json!({ "content_type": JSON, "fields": list(Place::Body) });
    }
    if let Some(body) = operation.raw_body {
        value["body"] = json!({ "content_type": body.content_type, "description": body.description });
    }
    value["response"] = json!({ "content_type": operation.response_type, "description": operation.response });
    if let Some(example) = operation.example {
        let mut request = format!("{} {}", operation.method, operation.path);
        if !example.query.is_empty() {
            request = format!("{request}?{}", example.query);
        }
        value["example"] = json!({ "request": request });
        if !example.body.is_empty() {
            value["example"]["body"] = json!(example.body);
        }
    }
    value
}

fn operations_of(path: &str) -> impl Iterator<Item = &'static Operation> + '_ {
    OPERATIONS.iter().filter(move |o| o.path == path)
}

fn doc_link(route: &Route) -> String {
    format!("/api/doc?endpoint={}", route.path.trim_start_matches("/api/"))
}

/// `GET /api/doc`: the endpoints.
pub(super) fn index() -> Value {
    let endpoints: Vec<Value> = ROUTES
        .iter()
        .map(|route| {
            let mut endpoint = json!({ "path": route.path, "methods": route.methods, "summary": route.summary });
            if route.extension {
                endpoint["extension"] = json!(true);
            }
            endpoint["doc"] = json!(doc_link(route));
            endpoint
        })
        .collect();
    json!({
        "frontend": "bemu",
        "version": env!("CARGO_PKG_VERSION"),
        "conventions": CONVENTIONS,
        "formats": ["json", "openapi", "html"],
        "endpoints": endpoints
    })
}

/// `GET /api/doc?endpoint=`: `None` for an unknown endpoint.
pub(super) fn endpoint(name: &str) -> Option<Value> {
    let name = name.trim_start_matches('/');
    let name = name.strip_prefix("api/").unwrap_or(name);
    let route = ROUTES.iter().find(|r| r.path.trim_start_matches("/api/") == name)?;
    let mut value = json!({
        "path": route.path,
        "summary": route.summary,
        "operations": operations_of(route.path).map(operation_json).collect::<Vec<_>>()
    });
    if route.extension {
        value["extension"] = json!(true);
    }
    Some(value)
}

fn schema_of(kind: &str) -> Value {
    match kind {
        // Numbers, or decimal / 0x hex strings.
        "integer" => json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }] }),
        "boolean" => json!({ "type": "boolean" }),
        "hex" => json!({ "type": "string", "pattern": "^[0-9A-Fa-f :]*$" }),
        _ => json!({ "type": "string" })
    }
}

/// `GET /api/doc?format=openapi`: an OpenAPI 3.0 description.
pub(super) fn openapi() -> Value {
    let mut paths = Map::new();
    for route in ROUTES {
        let mut item = Map::new();
        for operation in operations_of(route.path) {
            let parameters: Vec<Value> = operation
                .params
                .iter()
                .filter(|p| p.place == Place::Query)
                .map(|p| {
                    json!({
                        "name": p.name,
                        "in": "query",
                        "required": p.required,
                        "description": p.description,
                        // Query values are text: integers in decimal or 0x hex.
                        "schema": if p.kind == "integer" { json!({ "type": "string" }) } else { schema_of(p.kind) }
                    })
                })
                .collect();
            let mut value = json!({
                "summary": route.summary,
                "description": operation.description,
                "operationId": format!("{}_{}", operation.method.to_ascii_lowercase(), route.path.trim_start_matches("/api/")),
                "parameters": parameters,
                "responses": {
                    "200": {
                        "description": operation.response,
                        "content": { operation.response_type: { "schema": if operation.response_type == JSON {
                            json!({ "type": "object" })
                        } else {
                            json!({ "type": "string" })
                        } } }
                    },
                    "400": {
                        "description": "a parameter or field is wrong",
                        "content": { JSON: { "schema": { "$ref": "#/components/schemas/Error" } } }
                    }
                }
            });
            if operation.has_json_body() {
                let mut properties = Map::new();
                for p in operation.params.iter().filter(|p| p.place == Place::Body) {
                    let mut schema = schema_of(p.kind);
                    schema["description"] = json!(p.description);
                    properties.insert(p.name.to_owned(), schema);
                }
                let required: Vec<&str> =
                    operation.params.iter().filter(|p| p.place == Place::Body && p.required).map(|p| p.name).collect();
                let mut schema = json!({ "type": "object", "properties": properties });
                if !required.is_empty() {
                    schema["required"] = json!(required);
                }
                value["requestBody"] = json!({ "required": true, "content": { JSON: { "schema": schema } } });
            }
            if let Some(body) = operation.raw_body {
                let format = if body.content_type == "application/octet-stream" { "binary" } else { "text" };
                value["requestBody"] = json!({
                    "description": body.description,
                    "content": { body.content_type: { "schema": { "type": "string", "format": format } } }
                });
            }
            if let Some(example) = operation.example.filter(|e| !e.body.is_empty()) {
                let content_type = operation.raw_body.map_or(JSON, |b| b.content_type);
                value["requestBody"]["content"][content_type]["example"] = match content_type {
                    JSON => serde_json::from_str(example.body).unwrap_or(json!(example.body)),
                    _ => json!(example.body)
                };
            }
            if route.extension {
                value["x-bemu-extension"] = json!(true);
            }
            item.insert(operation.method.to_ascii_lowercase(), value);
        }
        paths.insert(route.path.to_owned(), Value::Object(item));
    }
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "bemu debug server",
            "version": env!("CARGO_PKG_VERSION"),
            "description": format!(
                "The AMSpiriT-Lite debug API of bemu, with extensions (x-bemu-extension).\n\n- {}",
                CONVENTIONS.join("\n- ")
            )
        },
        "servers": [{ "url": "/" }],
        "paths": paths,
        "components": {
            "schemas": {
                "Error": {
                    "type": "object",
                    "properties": { "error": { "type": "string" }, "field": { "type": "string" } },
                    "required": ["error"]
                }
            }
        }
    })
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `GET /api/doc?format=html`: the same, to read in a browser.
pub(super) fn html() -> String {
    let mut page = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\">\
         <title>bemu debug API</title><style>\
         body{font:15px/1.45 system-ui,sans-serif;max-width:60em;margin:auto;padding:0 1em;color:#222;background:#fff}\
         code,pre{font:13px ui-monospace,monospace;background:#f3f3f3;padding:0 .2em}pre{padding:.5em;overflow:auto}\
         h2{margin-top:2em;border-bottom:1px solid #ddd}h3{font-size:1em}table{border-collapse:collapse}\
         td,th{border:1px solid #ddd;padding:.2em .5em;text-align:left;vertical-align:top}.ext{color:#a50}\
         @media(prefers-color-scheme:dark){body{background:#181818;color:#ddd}code,pre{background:#262626}td,th,h2{border-color:#444}}\
         </style></head><body><h1>bemu debug API</h1>",
    );
    page.push_str(&format!(
        "<p>The AMSpiriT-Lite debug API, with bemu's extensions (marked <span class=ext>extension</span>). \
         Also as <a href=\"/api/doc\">JSON</a> and <a href=\"/api/doc?format=openapi\">OpenAPI 3</a>. Version {}.</p><ul>",
        env!("CARGO_PKG_VERSION")
    ));
    for convention in CONVENTIONS {
        page.push_str(&format!("<li>{}</li>", escape(convention)));
    }
    page.push_str("</ul><h2>Endpoints</h2><ul>");
    for route in ROUTES {
        page.push_str(&format!(
            "<li><a href=\"#{0}\"><code>{0}</code></a> {1}</li>",
            route.path,
            escape(route.summary)
        ));
    }
    page.push_str("</ul>");
    for route in ROUTES {
        page.push_str(&format!("<h2 id=\"{0}\"><code>{0}</code>", route.path));
        if route.extension {
            page.push_str(" <span class=ext>extension</span>");
        }
        page.push_str("</h2>");
        for operation in operations_of(route.path) {
            page.push_str(&format!(
                "<h3><code>{} {}</code></h3><p>{}</p>",
                operation.method,
                operation.path,
                escape(operation.description)
            ));
            for (place, title) in [(Place::Query, "Query"), (Place::Body, "JSON body")] {
                let params: Vec<_> = operation.params.iter().filter(|p| p.place == place).collect();
                if params.is_empty() {
                    continue;
                }
                page.push_str(&format!("<table><tr><th>{title}</th><th>type</th><th></th></tr>"));
                for p in params {
                    page.push_str(&format!(
                        "<tr><td><code>{}</code>{}</td><td>{}</td><td>{}</td></tr>",
                        p.name,
                        if p.required { " (required)" } else { "" },
                        p.kind,
                        escape(p.description)
                    ));
                }
                page.push_str("</table>");
            }
            if let Some(body) = operation.raw_body {
                page.push_str(&format!("<p>Body (<code>{}</code>): {}</p>", body.content_type, escape(body.description)));
            }
            page.push_str(&format!(
                "<p>Answer (<code>{}</code>): {}</p>",
                operation.response_type,
                escape(operation.response)
            ));
            if let Some(example) = operation.example {
                let mut request = format!("{} {}", operation.method, operation.path);
                if !example.query.is_empty() {
                    request = format!("{request}?{}", example.query);
                }
                if !example.body.is_empty() {
                    request = format!("{request}\n\n{}", example.body);
                }
                page.push_str(&format!("<pre>{}</pre>", escape(&request)));
            }
        }
    }
    page.push_str("</body></html>");
    page
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::debug_server::{ApiRequest, DebugServer};
    use crate::motherboard::{Motherboard, MotherboardBuilder};

    #[test]
    fn every_route_and_method_is_documented_once() {
        for route in ROUTES {
            for method in route.methods {
                let count = OPERATIONS.iter().filter(|o| o.path == route.path && o.method == *method).count();
                assert_eq!(count, 1, "{method} {}", route.path);
            }
        }
        for operation in OPERATIONS {
            assert!(
                ROUTES.iter().any(|r| r.path == operation.path && r.methods.contains(&operation.method)),
                "{} {} is not served",
                operation.method,
                operation.path
            );
            assert!(
                !(operation.has_json_body() && operation.raw_body.is_some()),
                "{} {}: one kind of body",
                operation.method,
                operation.path
            );
        }
    }

    fn request(method: &str, path: &str, query: &str, body: &str) -> ApiRequest {
        let query = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap();
                (k.to_owned(), v.to_owned())
            })
            .collect();
        ApiRequest { method: method.to_owned(), path: path.to_owned(), query, body: body.as_bytes().to_vec() }
    }

    fn booted() -> Motherboard {
        let mut mb = MotherboardBuilder::default().build();
        while mb.t_state_counter().value() < crate::debug_server::BASIC_BOOT_T_STATES {
            mb.execute_next_opcode_no_video();
        }
        mb
    }

    /// Each example is accepted, run in order on one machine (the counter
    /// created before it is deleted, the program loaded before it runs).
    #[test]
    fn the_examples_are_accepted() {
        let mut server = DebugServer::with_requests(std::sync::mpsc::channel().1);
        let mut mb = booted();
        server.handle(&request("POST", "/api/basic", "", "10 PRINT 1\n20 GOTO 10\n"), &mut mb);
        let mut order: Vec<&Operation> = OPERATIONS.iter().filter(|o| o.example.is_some()).collect();
        // Create before delete.
        order.sort_by_key(|o| o.method == "DELETE");
        for operation in order {
            let example = operation.example.unwrap();
            let response = server.handle(&request(operation.method, operation.path, example.query, example.body), &mut mb);
            let accepted = response.status == 200
                // The beam is known only when a monitor drives the emulation.
                || operation.path == "/api/beam" && response.status == 400
                || operation.path == "/api/screenshot" && response.status == 503;
            assert!(
                accepted,
                "{} {}?{}: {} {}",
                operation.method,
                operation.path,
                example.query,
                response.status,
                String::from_utf8_lossy(&response.body)
            );
        }
    }

    #[test]
    fn doc_formats() {
        let mut server = DebugServer::with_requests(std::sync::mpsc::channel().1);
        let mut mb = booted();
        let mut get = |query: &str| server.handle(&request("GET", "/api/doc", query, ""), &mut mb);
        let index: Value = serde_json::from_slice(&get("").body).unwrap();
        assert_eq!(index["endpoints"].as_array().unwrap().len(), ROUTES.len());
        assert_eq!(index["endpoints"][3]["doc"], "/api/doc?endpoint=ram");

        for name in ["ram", "/api/ram", "api/ram"] {
            let ram: Value = serde_json::from_slice(&get(&format!("endpoint={name}")).body).unwrap();
            assert_eq!(ram["path"], "/api/ram", "{name}");
            assert_eq!(ram["operations"][0]["method"], "GET");
            assert_eq!(ram["operations"][0]["query"][0]["name"], "addr");
            assert_eq!(ram["operations"][1]["body"]["fields"][2]["name"], "data");
        }
        assert_eq!(get("endpoint=nope").status, 400);
        assert_eq!(get("format=yaml").status, 400);

        let openapi: Value = serde_json::from_slice(&get("format=openapi").body).unwrap();
        assert_eq!(openapi["openapi"], "3.0.3");
        let breakpoints = &openapi["paths"]["/api/breakpoints"];
        assert_eq!(breakpoints["post"]["x-bemu-extension"], true);
        assert_eq!(breakpoints["post"]["requestBody"]["content"][JSON]["schema"]["required"], json!(["addr"]));
        assert_eq!(breakpoints["delete"]["parameters"][0]["name"], "index");
        assert_eq!(openapi["paths"]["/api/media"]["post"]["requestBody"]["content"]["application/octet-stream"]["schema"]["format"], "binary");
        assert_eq!(openapi["paths"]["/api/z80_bp"]["post"]["requestBody"]["content"]["text/plain"]["example"], "0x4000, B01:0010");

        let html = get("format=html");
        assert_eq!(html.content_type, "text/html; charset=utf-8");
        let html = String::from_utf8(html.body).unwrap();
        assert!(html.contains("<h2 id=\"/api/raster_bp\">"));
        assert!(html.contains("&quot;screen&quot;"), "examples escaped");
    }
}
