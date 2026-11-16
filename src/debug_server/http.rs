//! The debug API over HTTP (the desktop's; the web page has none): axum, on a tokio runtime of
//! its own thread.
//!
//! Every request goes to the emulation thread as an [`ApiRequest`], which
//! [`super::DebugServer::handle`] answers: the same answers as in process (the window's
//! commands, the tests), parsed into and built from [`bemu_api`]'s types there. What this module
//! adds is the description: each route names the types of its query, its body and its reply
//! ([`DocQuery`], [`DocJson`], [`TextBody`], [`Reply`]...), and aide generates the OpenAPI
//! description from them ([`openapi`]). An unknown path, or a method a path does not take, goes
//! to the emulation thread too, for AMSpiriT's 404 and 405.
//!
//! `/api/events` (Server-Sent Events) is served here, from the [`EventHub`] the emulation
//! publishes to, and `/api/doc/ui` is a page to read the description in (Scalar, built in).

use std::convert::Infallible;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use aide::axum::routing::{get_with, post_with};
use aide::axum::ApiRouter;
use aide::generate::GenContext;
use aide::openapi::{Info, OpenApi, Operation};
use aide::{OperationInput, OperationOutput};
use axum::body::Body;
use axum::extract::{FromRequest, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Router;
use bemu_api as api;
use bemu_api::*;
use schemars::JsonSchema;
use serde_json::{json, Value};

use super::{query_pairs, ApiRequest, ApiResponse, EventHub, MAX_SSE_CLIENTS, SSE_HEARTBEAT};

/// The largest request body (a disc image, a snapshot).
const MAX_BODY: usize = 64 << 20;
/// How long the emulation is given to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);

/// The HTTP server, while it runs: stopped when dropped.
pub(super) struct Listener {
    pub port: u16,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// What every handler needs: the way to the emulation thread, and the events.
#[derive(Clone)]
struct Shared {
    requests: Sender<(ApiRequest, Sender<ApiResponse>)>,
    events: EventHub,
}

/// Serve the API on `127.0.0.1:port`, the requests going to `requests`.
pub(super) fn listen(
    port: u16,
    requests: Sender<(ApiRequest, Sender<ApiResponse>)>,
    events: EventHub,
) -> Result<Listener, String> {
    // the server just stopped may hold the port a moment
    let mut tries = 0;
    let socket = loop {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(socket) => break socket,
            Err(_) if tries < 20 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("debug server on port {port}: {e}")),
        }
    };
    let port = socket.local_addr().map_err(|e| e.to_string())?.port();
    socket.set_nonblocking(true).map_err(|e| e.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|e| format!("debug server: {e}"))?;
    // Handed to tokio here, in its runtime, for a failure to be told rather than leave a server
    // that says it listens and serves nothing.
    let socket = {
        let _runtime = runtime.enter();
        tokio::net::TcpListener::from_std(socket).map_err(|e| format!("debug server: {e}"))?
    };
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let app = router(Shared { requests, events });
    std::thread::Builder::new()
        .name("debug-server".to_owned())
        .spawn(move || {
            runtime.block_on(async move {
                // Not a graceful shutdown: an event stream never ends by itself.
                tokio::select! {
                    _ = axum::serve(socket, app) => {}
                    _ = stopped => {}
                }
            });
            // the event streams' threads end with their clients
            runtime.shutdown_background();
        })
        .map_err(|e| e.to_string())?;
    Ok(Listener {
        port,
        stop: Some(stop),
    })
}

/// The API: every route typed by [`bemu_api`], served, and described from its types.
fn typed_routes() -> ApiRouter<Shared> {
    use aide::axum::routing::delete_with;
    // The query, the body and the reply of each route: what describes it.
    macro_rules! api {
        ($router:expr; $($method:ident $path:literal ($query:ty, $body:ty) -> $reply:ty: $summary:literal, $description:literal;)*) => {{
            let router = $router;
            $(
                let router = router.api_route(
                    $path,
                    $method(typed::<$query, $body, $reply>, |op| op.summary($summary).description($description)),
                );
            )*
            router
        }};
    }
    api! { ApiRouter::new();
        get_with "/api/ping" (NoInput, NoInput) -> axum::Json<Ping>:
            "connectivity probe, with the emu object",
            "Is the server there; the emulator's state, and what this server offers beyond the AMSpiriT API (features).";
        get_with "/api/state" (NoInput, NoInput) -> axum::Json<api::State>:
            "z80, ga, crtc, psg, fdc, beam and emu state",
            "Everything at once: the Z80's registers, the Gate Array, the CRTC, the PSG, the FDC, the beam and the emulator.";
        get_with "/api/z80" (NoInput, NoInput) -> axum::Json<Z80Registers>:
            "the Z80 registers", "The Z80's registers; the alternate set is A2..L2.";
        post_with "/api/z80" (NoInput, DocJson<SetZ80>) -> axum::Json<CommandAck>:
            "set Z80 registers", "Set registers by name; all are checked before any changes. A pair sets its two halves.";
        get_with "/api/ram" (DocQuery<ReadRam>, NoInput) -> axum::Json<Ram>:
            "read memory: the CPU's, a RAM bank, a ROM, a cartridge page",
            "Read memory, as hex. at= names where: a CPU address, read through paging= (default the CPU's \
             paging now: C4, U7, -L... see Paging), or an offset in a space: Bnn (a RAM bank), LOWER, UPPERnn, \
             CARTnn (a CPC+ cartridge page), ASIC (its registers). The older form: the RAM banks (view=raw, \
             default: addr an offset in bank) or the CPU's addresses (view=cpu).";
        post_with "/api/ram" (NoInput, DocJson<WriteRam>) -> axum::Json<MemoryAck>:
            "write RAM, and/or jump",
            "Write bytes to RAM, and/or set PC (exec). at names where: a CPU address (the RAM a write there \
             reaches, through paging) or an offset in a RAM bank (Bnn:hhhh); a ROM, a cartridge page, the \
             registers are refused. The older form: addr an offset in bank.";
        post_with "/api/exec" (NoInput, DocJson<SetPc>) -> axum::Json<MemoryAck>:
            "set PC", "Set PC.";
        post_with "/api/step" (DocQuery<StepQuery>, NoInput) -> axum::Json<CommandAck>:
            "step: one instruction, over it, out of the routine, or a frame",
            "Run one instruction, then pause (a z80_bp event, reason step); or step over it (a CALL, RST, DJNZ or block \
             loop run whole: reason run_to), out of the routine (reason step_out), or to the end of the frame (a pause event).";
        post_with "/api/run_to" (NoInput, DocJson<RunTo>) -> axum::Json<CommandAck>:
            "run to an address or a beam position, once",
            "Run to an address or symbol (a z80_bp event, reason run_to), or to a beam position (reason run_to_beam), once, then pause.";
        get_with "/api/config" (NoInput, NoInput) -> axum::Json<Config>:
            "the emulator's settings", "The CRTC type, whether paused, the history's size.";
        post_with "/api/config" (NoInput, DocJson<SetConfig>) -> axum::Json<CommandAck>:
            "pause, resume, reset, size the history", "Pause or resume, reset (soft: RAM kept; hard: power off and on), size the instruction history.";
        post_with "/api/z80_bp" (DocQuery<ExecBreakpointsQuery>, TextBody) -> axum::Json<ExecBreakpointsSet>:
            "replace (or add to) the execution breakpoints",
            "The body: comma-separated CPU addresses or symbols (label, label+n: the snapshot's and --sym files' labels), \
             resolved against the current mapping to the RAM bank or ROM they reach, or Bnn:hhhh (hex bank, hex offset); \
             empty clears. ?add=1 adds them to the others.";
        delete_with "/api/z80_bp" (NoInput, TextBody) -> axum::Json<ExecBreakpointsRemoved>:
            "remove execution breakpoints", "The body: the breakpoints to remove, as POST takes them; empty removes them all.";
        get_with "/api/symbols" (NoInput, NoInput) -> axum::Json<Vec<Symbol>>:
            "the symbols, by address", "The symbols known: the loaded snapshot's (SYMB chunk, REMU labels) and the --sym files'.";
        get_with "/api/breakpoints" (NoInput, NoInput) -> axum::Json<Breakpoints>:
            "the execution and advanced breakpoints", "The execution breakpoints' locations, and the advanced ones with their hits.";
        post_with "/api/breakpoints" (NoInput, DocJson<NewBreakpoint>) -> axum::Json<BreakpointAdded>:
            "add an advanced breakpoint", "Add a memory, I/O or execution breakpoint that stops, reports (watch) or counts.";
        delete_with "/api/breakpoints" (DocQuery<RemoveBreakpoint>, NoInput) -> axum::Json<CommandAck>:
            "remove an advanced breakpoint, or all", "Remove one advanced breakpoint (the others are renumbered), or all of them.";
        post_with "/api/keytype" (NoInput, DocJson<KeyType>) -> axum::Json<CommandAck>:
            "type text on the CPC keyboard", "Type text on the CPC keyboard, a key at a time (\\r or \\n is RETURN).";
        post_with "/api/quit" (NoInput, NoInput) -> axum::Json<CommandAck>:
            "leave the emulator", "Leave the emulator.";
        get_with "/api/history" (DocQuery<HistoryQuery>, NoInput) -> axum::Json<Vec<HistoryEntry>>:
            "the last instructions run, oldest first", "Each one's bytes as fetched and as in memory now, and the symbol it is at.";
        get_with "/api/screenshot" (DocQuery<ScreenshotQuery>, NoInput) -> Png:
            "the screen as a PNG",
            "Headers: X-Crop-X/Y/W/H (where the picture is in the beam's 1024x350 raster), X-Beam-X/Y (the beam, in the \
             picture) when known. 503 while there is no frame yet, or for crop=0 when whole frames are not kept.";
        post_with "/api/media" (DocQuery<MediaQuery>, BinaryBody) -> axum::Json<MediaLoaded>:
            "load a snapshot, a disc, a tape or a binary",
            "The body: the file. A .sna (its breakpoints and symbols set), .dsk/.hfe (or an archive holding one), and .cdt \
             (PLAY down) are recognised by their content; a .bin/.amsdos by ?name=.";
        get_with "/api/snapshot" (NoInput, NoInput) -> Binary:
            "the machine as a .sna",
            "The machine's state as a version 3 .sna file: CPU, RAM, CRTC with its counters, Gate Array, PSG, PPI, ROM and \
             RAM banking. POST it to /api/media to go back to it.";
        get_with "/api/memmap" (DocQuery<MemMapQuery>, NoInput) -> axum::Json<MemMap>:
            "what is mapped in each 16K region",
            "Each 16K region of the CPU: what is read there (source: a space, as at= names it; rom_bank or \
             ram_bank), the bank written; RMR and MMR. paging= describes another paging than the CPU's.";
        get_with "/api/crtc" (NoInput, NoInput) -> axum::Json<Crtc>:
            "the CRTC", "The CRTC's registers, the selected one and its counters.";
        get_with "/api/ga" (NoInput, NoInput) -> axum::Json<GateArray>:
            "the Gate Array", "The Gate Array: the mode, the pen, the inks, RMR and banking.";
        get_with "/api/psg" (NoInput, NoInput) -> axum::Json<Psg>:
            "the PSG, decoded", "The PSG's registers, decoded, and each channel as a musician reads it: its frequency, \
             note, volume; the noise's and the envelope's frequencies.";
        get_with "/api/fdc" (NoInput, NoInput) -> axum::Json<Fdc>:
            "the FDC", "The FDC: its status registers, phase, motor and heads; each disc as the head sees it (its format, \
             the track map, the sectors under the head, the AMSDOS catalogue).";
        get_with "/api/asic" (NoInput, NoInput) -> axum::Json<Option<Asic>>:
            "a CPC+'s ASIC", "A CPC+'s ASIC (null on a CPC): the lock, the palette, the registers, the DMA channels, \
             the sprites.";
        post_with "/api/drive" (DocQuery<DriveCommand>, NoInput) -> axum::Json<Fdc>:
            "set a drive up, turn its disc over",
            "A 3\" drive (one head: the side read is the side put up), a double-sided 3.5\" one or a Gotek; turn the disc \
             of a 3\" drive over. Answers the FDC.";
        get_with "/api/beam" (DocQuery<BeamQuery>, NoInput) -> axum::Json<BeamPosition>:
            "the beam's position", "Where the beam is, in the 1024x350 raster and in NOPs. 400 without a monitor.";
        get_with "/api/raster_bp" (NoInput, NoInput) -> axum::Json<RasterBreakpoints>:
            "the beam breakpoints", "The beam breakpoints.";
        post_with "/api/raster_bp" (DocQuery<SetRasterBreakpoint>, NoInput) -> axum::Json<RasterBreakpointsSet>:
            "set a beam breakpoint",
            "Stop when the beam reaches a position: x/y (or with crop=1, in the picture), or nop in line, or frame_nop. \
             nop alone stops in every line, line alone at its start. enable=0 clears them all.";
        get_with "/api/counters" (NoInput, NoInput) -> axum::Json<Counters>:
            "the NOP counters", "The named counters of the NOPs run.";
        post_with "/api/counters" (NoInput, DocJson<CounterCommand>) -> axum::Json<CountersChanged>:
            "create, reset, start or stop a counter", "Create, reset, start or stop a counter of the NOPs run.";
        delete_with "/api/counters" (DocQuery<RemoveCounter>, NoInput) -> axum::Json<CountersChanged>:
            "delete a counter, or all", "Delete a counter, or all of them.";
        post_with "/api/basic" (DocQuery<BasicLoad>, TextBody) -> axum::Json<CommandAck>:
            "load a BASIC program", "The body: the program, as typed (numbered lines), loaded in place of the one in RAM (variables cleared).";
        get_with "/api/basic_state" (NoInput, NoInput) -> axum::Json<api::BasicState>:
            "where the BASIC program is, what is run", "Where the BASIC program and its variables are, the line and statement being run.";
        get_with "/api/basic_export" (NoInput, NoInput) -> Text:
            "the BASIC program as text", "The BASIC program in RAM, as text.";
        get_with "/api/basic_listing" (NoInput, NoInput) -> axum::Json<BasicListing>:
            "the BASIC program as statements", "The BASIC program as lines of statements with their addresses.";
        post_with "/api/basic_step" (DocQuery<BasicStep>, NoInput) -> axum::Json<CommandAck>:
            "run to the next BASIC statement or line", "Run to the next statement (or line), then stop (a basic_bp event, reason step).";
        get_with "/api/basic_bp" (NoInput, NoInput) -> axum::Json<BasicBreakpoints>:
            "the BASIC line breakpoints", "The BASIC line breakpoints.";
        post_with "/api/basic_bp" (NoInput, TextBody) -> axum::Json<BasicBreakpointsSet>:
            "replace the BASIC line breakpoints", "The body: comma-separated line numbers; empty clears.";
        post_with "/api/basic_runto" (DocQuery<BasicRunTo>, NoInput) -> axum::Json<CommandAck>:
            "run to a BASIC line or statement, once", "Run to a line or a statement, once, then stop (a basic_bp event, reason runto).";
        get_with "/api/doc" (DocQuery<api::DocQuery>, NoInput) -> axum::Json<Value>:
            "this documentation",
            "The endpoints (default), one in detail (?endpoint=), the whole API as OpenAPI 3.1 (format=openapi) or a page \
             (format=html); /api/doc/ui is an interactive reference.";
    }
    .api_route(
        "/api/events",
        get_with(events, |op| {
            op.summary("Server-Sent Events")
                .description(
                    "The events, as they come: z80_bp (the machine stopped: StopEvent), watch (WatchEvent), frame (every 10 \
                     frames while running: FrameEvent), pause (PauseEvent), reset (ResetEvent), basic_bp (BasicStopEvent), \
                     basic_error (BasicErrorEvent). Each event's data is JSON, of the schema named.",
                )
        }),
    )
}

/// The OpenAPI description generated from the routes' types, and the events' (`x-events`):
/// generated once, as the types do not change.
pub(super) fn openapi() -> Value {
    static OPENAPI: OnceLock<Value> = OnceLock::new();
    OPENAPI.get_or_init(generate_openapi).clone()
}

fn generate_openapi() -> Value {
    let mut api = OpenApi {
        info: Info {
            title: "bemu debug API".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Info::default()
        },
        ..OpenApi::default()
    };
    let _ = typed_routes().finish_api(&mut api);
    let mut value = serde_json::to_value(&api).unwrap_or(Value::Null);
    // The events' data: not a reply of any route, but described all the same.
    let mut generator = schemars::generate::SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();
    let events = bemu_api::event_schemas(&mut generator);
    if let Some(schemas) = value["components"]["schemas"].as_object_mut() {
        for (name, schema) in generator.take_definitions(true) {
            schemas.entry(name).or_insert(schema);
        }
        for (name, schema) in events {
            schemas.entry(name.to_owned()).or_insert(schema.to_value());
        }
    }
    value["x-events"] = serde_json::to_value(
        bemu_api::EVENTS
            .iter()
            .map(|(event, schema)| (event.to_string(), json!({ "$ref": format!("#/components/schemas/{schema}") })))
            .collect::<serde_json::Map<_, _>>(),
    )
    .unwrap_or(Value::Null);
    value
}

/// Every path served.
fn router(shared: Shared) -> Router {
    let mut api = OpenApi::default();
    typed_routes()
        .finish_api(&mut api)
        .route("/api/doc/ui", axum::routing::get(doc_page))
        // the AMSpiriT answers (a JSON 405 with its Allow), not axum's
        .method_not_allowed_fallback(untyped)
        .fallback(untyped)
        .with_state(shared)
}

// ------------------------------------------------------------------ forwarding

/// The request as the emulation thread takes it.
struct Forwarded(ApiRequest);

impl<S: Send + Sync> FromRequest<S> for Forwarded {
    type Rejection = Response;

    async fn from_request(request: Request, _: &S) -> Result<Self, Self::Rejection> {
        let (parts, body) = request.into_parts();
        let body = axum::body::to_bytes(body, MAX_BODY).await.map_err(|e| {
            to_response(ApiResponse::json(413, json!({ "error": format!("body: {e}") })))
        })?;
        Ok(Forwarded(ApiRequest {
            method: parts.method.as_str().to_uppercase(),
            path: parts.uri.path().to_owned(),
            query: query_pairs(parts.uri.query().unwrap_or("")),
            body: body.to_vec(),
        }))
    }
}

impl OperationInput for Forwarded {}

/// Ask the emulation thread.
async fn forward(shared: &Shared, Forwarded(request): Forwarded) -> ApiResponse {
    let requests = shared.requests.clone();
    tokio::task::spawn_blocking(move || {
        let (reply, answer) = mpsc::channel();
        if requests.send((request, reply)).is_err() {
            return ApiResponse::json(503, json!({ "error": "the emulation is gone" }));
        }
        answer
            .recv_timeout(ANSWER_TIMEOUT)
            .unwrap_or_else(|_| ApiResponse::json(503, json!({ "error": "the emulation did not answer" })))
    })
    .await
    .unwrap_or_else(|e| ApiResponse::json(503, json!({ "error": e.to_string() })))
}

fn to_response(answer: ApiResponse) -> Response {
    let mut response = Response::new(Body::from(answer.body));
    *response.status_mut() = StatusCode::from_u16(answer.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let headers = response.headers_mut();
    let mut set = |name: &str, value: &str| {
        if let (Ok(name), Ok(value)) = (HeaderName::try_from(name), HeaderValue::try_from(value)) {
            headers.insert(name, value);
        }
    };
    set("content-type", answer.content_type);
    set("access-control-allow-origin", "*");
    set("cache-control", "no-store");
    for (name, value) in &answer.headers {
        set(name, value);
    }
    response
}

// ------------------------------------------------------------------ what describes a route

/// A typed query, `T`: for the description only; the emulation thread parses it.
struct DocQuery<T>(PhantomData<fn() -> T>);

impl<T, S: Send + Sync> FromRequestParts<S> for DocQuery<T> {
    type Rejection = Infallible;

    async fn from_request_parts(_: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(DocQuery(PhantomData))
    }
}

impl<T: JsonSchema> OperationInput for DocQuery<T> {
    fn operation_input(ctx: &mut GenContext, operation: &mut Operation) {
        <axum::extract::Query<T> as OperationInput>::operation_input(ctx, operation);
    }
}

/// A typed JSON body, `T`: for the description only; the emulation thread parses it.
struct DocJson<T>(PhantomData<fn() -> T>);

impl<T, S: Send + Sync> FromRequestParts<S> for DocJson<T> {
    type Rejection = Infallible;

    async fn from_request_parts(_: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(DocJson(PhantomData))
    }
}

impl<T: JsonSchema> OperationInput for DocJson<T> {
    fn operation_input(ctx: &mut GenContext, operation: &mut Operation) {
        <axum::Json<T> as OperationInput>::operation_input(ctx, operation);
    }
}

/// No query, or no body.
struct NoInput;

impl<S: Send + Sync> FromRequestParts<S> for NoInput {
    type Rejection = Infallible;

    async fn from_request_parts(_: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(NoInput)
    }
}

impl OperationInput for NoInput {}

/// A body that is not JSON: `text/plain` ([`TextBody`]) or a file ([`BinaryBody`]).
fn raw_body(operation: &mut Operation, content_type: &str, format: &str) {
    operation.request_body = Some(aide::openapi::ReferenceOr::Item(aide::openapi::RequestBody {
        content: [(
            content_type.to_owned(),
            aide::openapi::MediaType {
                schema: Some(aide::openapi::SchemaObject {
                    json_schema: schemars::json_schema!({ "type": "string", "format": format }),
                    example: None,
                    external_docs: None,
                }),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    }));
}

/// A `text/plain` body (described by the operation).
struct TextBody;

impl<S: Send + Sync> FromRequestParts<S> for TextBody {
    type Rejection = Infallible;

    async fn from_request_parts(_: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(TextBody)
    }
}

impl OperationInput for TextBody {
    fn operation_input(_: &mut GenContext, operation: &mut Operation) {
        raw_body(operation, "text/plain", "text");
    }
}

/// A file as the body.
struct BinaryBody;

impl<S: Send + Sync> FromRequestParts<S> for BinaryBody {
    type Rejection = Infallible;

    async fn from_request_parts(_: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(BinaryBody)
    }
}

impl OperationInput for BinaryBody {
    fn operation_input(_: &mut GenContext, operation: &mut Operation) {
        raw_body(operation, "application/octet-stream", "binary");
    }
}

/// What a reply is: JSON of a type (`axum::Json<T>`), text, a PNG, a file.
trait ReplyKind {
    fn response(ctx: &mut GenContext, operation: &mut Operation) -> Option<aide::openapi::Response>;
}

impl<T: JsonSchema> ReplyKind for axum::Json<T> {
    fn response(ctx: &mut GenContext, operation: &mut Operation) -> Option<aide::openapi::Response> {
        <axum::Json<T> as OperationOutput>::operation_response(ctx, operation)
    }
}

fn raw_response(description: &str, content_type: &str, format: &str) -> aide::openapi::Response {
    aide::openapi::Response {
        description: description.to_owned(),
        content: [(
            content_type.to_owned(),
            aide::openapi::MediaType {
                schema: Some(aide::openapi::SchemaObject {
                    json_schema: schemars::json_schema!({ "type": "string", "format": format }),
                    example: None,
                    external_docs: None,
                }),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    }
}

/// Plain text.
struct Text;

impl ReplyKind for Text {
    fn response(_: &mut GenContext, _: &mut Operation) -> Option<aide::openapi::Response> {
        Some(raw_response("text", "text/plain", "text"))
    }
}

/// A PNG image.
struct Png;

impl ReplyKind for Png {
    fn response(_: &mut GenContext, _: &mut Operation) -> Option<aide::openapi::Response> {
        Some(raw_response("a PNG image", "image/png", "binary"))
    }
}

/// A file.
struct Binary;

impl ReplyKind for Binary {
    fn response(_: &mut GenContext, _: &mut Operation) -> Option<aide::openapi::Response> {
        Some(raw_response("the file", "application/octet-stream", "binary"))
    }
}

/// The emulation thread's answer, described as `R` (and an [`ErrorReply`], 400).
struct Reply<R>(ApiResponse, PhantomData<fn() -> R>);

impl<R> Reply<R> {
    async fn of(shared: &Shared, request: Forwarded) -> Self {
        Reply(forward(shared, request).await, PhantomData)
    }
}

impl<R> IntoResponse for Reply<R> {
    fn into_response(self) -> Response {
        to_response(self.0)
    }
}

impl<R: ReplyKind> OperationOutput for Reply<R> {
    type Inner = R;

    fn operation_response(ctx: &mut GenContext, operation: &mut Operation) -> Option<aide::openapi::Response> {
        R::response(ctx, operation)
    }

    fn inferred_responses(
        ctx: &mut GenContext,
        operation: &mut Operation,
    ) -> Vec<(Option<u16>, aide::openapi::Response)> {
        let mut responses: Vec<_> = R::response(ctx, operation).map(|r| (Some(200), r)).into_iter().collect();
        if let Some(mut error) = <axum::Json<ErrorReply> as OperationOutput>::operation_response(ctx, operation) {
            error.description = "the request is wrong: what, and the field at fault".to_owned();
            responses.push((Some(400), error));
        }
        responses
    }
}

/// The event stream.
struct EventStream(Response);

impl IntoResponse for EventStream {
    fn into_response(self) -> Response {
        self.0
    }
}

impl OperationOutput for EventStream {
    type Inner = ();

    fn operation_response(_: &mut GenContext, _: &mut Operation) -> Option<aide::openapi::Response> {
        Some(raw_response("Server-Sent Events (see x-events)", "text/event-stream", "text"))
    }

    fn inferred_responses(
        ctx: &mut GenContext,
        operation: &mut Operation,
    ) -> Vec<(Option<u16>, aide::openapi::Response)> {
        Self::operation_response(ctx, operation).map(|r| (Some(200), r)).into_iter().collect()
    }
}

// ------------------------------------------------------------------ handlers

/// A typed route: `Q` and `B` describe its query and body, `R` its reply; the emulation
/// thread answers.
async fn typed<Q, B, R>(State(shared): State<Shared>, _: Q, _: B, request: Forwarded) -> Reply<R> {
    Reply::of(&shared, request).await
}

/// An unknown path, or a method a path does not take: the emulation thread answers (404, 405),
/// but for a browser's CORS preflight (`OPTIONS`), allowed.
async fn untyped(State(shared): State<Shared>, request: Forwarded) -> Response {
    if request.0.method == "OPTIONS" {
        let mut response = to_response(ApiResponse {
            status: 204,
            content_type: "text/plain",
            body: Vec::new(),
            headers: vec![
                ("access-control-allow-methods".to_owned(), "GET, POST, DELETE".to_owned()),
                ("access-control-allow-headers".to_owned(), "content-type".to_owned()),
                ("access-control-max-age".to_owned(), "86400".to_owned()),
            ],
        });
        response.headers_mut().remove("content-type");
        return response;
    }
    to_response(forward(&shared, request).await)
}

/// The page to read the description in.
async fn doc_page() -> Html<&'static str> {
    static PAGE: OnceLock<String> = OnceLock::new();
    Html(PAGE.get_or_init(|| {
        aide::scalar::Scalar::new("/api/doc?format=openapi")
            .with_title("bemu debug API")
            .html()
    }))
}

/// `GET /api/events`: Server-Sent Events, the ones in `?topics=` (default all), a `: ping`
/// comment after 15s of silence.
async fn events(
    State(shared): State<Shared>,
    _: DocQuery<EventsQuery>,
    Forwarded(request): Forwarded,
) -> EventStream {
    let query: EventsQuery = match super::query_as(&request) {
        Ok(query) => query,
        Err(error) => return EventStream(to_response(error.response())),
    };
    let topics = query
        .topics
        .map(|list| list.split(',').filter(|t| !t.is_empty()).map(str::to_owned).collect());
    let Some(queue) = shared.events.subscribe(topics) else {
        let error = format!("too many event clients (max {MAX_SSE_CLIENTS})");
        return EventStream(to_response(ApiResponse::json(503, json!({ "error": error }))));
    };
    // The queue is the emulation's (a mutex and a condition variable): a thread of its own waits
    // on it, and hands the events over until the client is gone.
    let (sender, receiver) = tokio::sync::mpsc::channel::<String>(64);
    let streamed = Arc::clone(&queue);
    let spawned = std::thread::Builder::new()
        .name("debug-server-events".to_owned())
        .spawn(move || {
            let queue = streamed;
            loop {
                let (lock, ready) = &*queue;
                let pending: Vec<String> = {
                    let guard = lock.lock().unwrap();
                    let (mut guard, _) = ready
                        .wait_timeout_while(guard, SSE_HEARTBEAT, |q| q.events.is_empty())
                        .unwrap();
                    guard.events.drain(..).collect()
                };
                let chunk = if pending.is_empty() {
                    ": ping\n\n".to_owned()
                } else {
                    pending.concat()
                };
                if sender.blocking_send(chunk).is_err() {
                    break;
                }
            }
            queue.0.lock().unwrap().closed = true;
        });
    if let Err(e) = spawned {
        // the client's place is given back
        queue.0.lock().unwrap().closed = true;
        let error = format!("cannot stream the events: {e}");
        return EventStream(to_response(ApiResponse::json(503, json!({ "error": error }))));
    }
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver
            .recv()
            .await
            .map(|chunk| (Ok::<_, Infallible>(chunk), receiver))
    });
    let mut response = Response::new(Body::from_stream(stream));
    let headers = response.headers_mut();
    headers.insert("content-type", HeaderValue::from_static("text/event-stream"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    EventStream(response)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_typed_routes_are_described_from_their_types() {
        let api = openapi();
        let paths = &api["paths"];
        assert!(paths["/api/ping"]["get"].is_object(), "{api}");
        assert!(paths["/api/z80"]["get"].is_object());
        let set = &paths["/api/z80"]["post"];
        assert!(set["requestBody"].is_object(), "{set}");
        let step = &paths["/api/step"]["post"];
        assert_eq!(step["parameters"][0]["name"], "mode", "{step}");
        assert!(step["responses"]["400"].is_object(), "{step}");
        let schemas = &api["components"]["schemas"];
        for name in ["Ping", "EmuState", "Z80Registers", "SetZ80", "CommandAck", "ErrorReply", "Int"] {
            assert!(schemas[name].is_object(), "{name}: {schemas}");
        }
        // A field's description is its doc comment.
        assert!(schemas["EmuState"]["properties"]["crtc_type"]["description"]
            .as_str()
            .is_some_and(|d| d.contains("CRTC")));
        // The events' data, by event.
        for (event, schema) in bemu_api::EVENTS {
            assert_eq!(api["x-events"][event]["$ref"], format!("#/components/schemas/{schema}"));
            assert!(schemas[schema].is_object(), "{schema}");
        }
        // Bodies that are not JSON, replies that are not JSON.
        assert!(paths["/api/z80_bp"]["post"]["requestBody"]["content"]["text/plain"].is_object());
        assert!(paths["/api/media"]["post"]["requestBody"]["content"]["application/octet-stream"].is_object());
        assert!(paths["/api/screenshot"]["get"]["responses"]["200"]["content"]["image/png"].is_object());
        assert!(paths["/api/events"]["get"]["responses"]["200"]["content"]["text/event-stream"].is_object());
    }

    /// A server on a free port: every request is answered by `answer`, standing for the
    /// emulation thread.
    fn serve(answer: impl Fn(ApiRequest) -> ApiResponse + Send + 'static) -> (Listener, u16) {
        let (requests, received) = mpsc::channel::<(ApiRequest, Sender<ApiResponse>)>();
        std::thread::spawn(move || {
            for (request, reply) in received {
                let _ = reply.send(answer(request));
            }
        });
        let listener = listen(0, requests, EventHub::default()).unwrap();
        let port = listener.port;
        (listener, port)
    }

    /// The raw HTTP answer to `request`.
    fn ask(port: u16, request: &str) -> String {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer
    }

    #[test]
    fn requests_reach_the_emulation_thread_decoded() {
        let (_listener, port) = serve(|request| {
            ApiResponse::json(
                200,
                json!({ "method": request.method, "path": request.path, "query": request.query,
                        "body": String::from_utf8_lossy(&request.body) }),
            )
        });
        let answer = ask(
            port,
            "POST /api/raster_bp?name=split%20screen&add=1 HTTP/1.1\r\nHost: x\r\n\
             Content-Length: 3\r\nConnection: close\r\n\r\nabc",
        );
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(answer.contains(r#"["name","split screen"]"#), "{answer}");
        assert!(answer.contains(r#""body":"abc""#), "{answer}");
        assert!(answer.to_ascii_lowercase().contains("access-control-allow-origin: *"), "{answer}");
    }

    #[test]
    fn a_browser_preflight_is_allowed() {
        let (_listener, port) = serve(|_| panic!("a preflight is not the emulation's"));
        let answer = ask(
            port,
            "OPTIONS /api/z80 HTTP/1.1\r\nHost: x\r\nOrigin: http://example\r\nConnection: close\r\n\r\n",
        );
        assert!(answer.starts_with("HTTP/1.1 204"), "{answer}");
        assert!(answer.to_ascii_lowercase().contains("access-control-allow-methods: get, post, delete"), "{answer}");
    }

    #[test]
    fn an_unknown_path_is_the_emulation_threads_to_answer() {
        let (_listener, port) = serve(|request| ApiResponse::json(404, json!({ "path": request.path })));
        let answer = ask(port, "GET /api/nothing HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 404"), "{answer}");
        assert!(answer.contains(r#""path":"/api/nothing""#), "{answer}");
    }
}
