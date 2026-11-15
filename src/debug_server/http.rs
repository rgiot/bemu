//! The debug API over HTTP (the desktop's; the web page has none): axum, on a tokio runtime of
//! its own thread.
//!
//! Every request, typed or not, goes to the emulation thread as an [`ApiRequest`], which
//! [`DebugServer::handle`] answers: the same answers as in process (the window's commands, the
//! tests). What this module adds to the routes migrated to [`bemu_api`]'s types is their
//! description: each handler names the types its request is parsed into and its reply built from
//! ([`DocQuery`], [`DocJson`], [`Reply`]), and aide generates the OpenAPI description from them.
//! The others reach the emulation thread through the fallback, as before, documented by hand
//! (`api_doc.rs`) until they are migrated.
//!
//! `/api/events` (Server-Sent Events) is served here, from the [`EventHub`] the emulation
//! publishes to, and `/api/doc/ui` is a page to read the description in (Scalar, built in).

use std::convert::Infallible;
use std::marker::PhantomData;
use std::sync::mpsc::{self, Sender};
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
use bemu_api::{CommandAck, ErrorReply, Ping, SetZ80, StepQuery, Z80Registers};
use schemars::JsonSchema;
use serde_json::{json, Value};

use super::{percent_decode, ApiRequest, ApiResponse, EventHub, MAX_SSE_CLIENTS, SSE_HEARTBEAT};

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
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let app = router(Shared { requests, events });
    std::thread::Builder::new()
        .name("debug-server".to_owned())
        .spawn(move || {
            runtime.block_on(async move {
                let Ok(socket) = tokio::net::TcpListener::from_std(socket) else {
                    return;
                };
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

/// The routes typed by [`bemu_api`]: served, and described from their types.
fn typed_routes() -> ApiRouter<Shared> {
    ApiRouter::new()
        .api_route(
            "/api/ping",
            get_with(ping, |op| {
                op.summary("connectivity probe, with the emu object")
                    .description("Is the server there; the emulator's state, and what this server offers beyond the AMSpiriT API (features).")
            }),
        )
        .api_route(
            "/api/z80",
            get_with(get_z80, |op| op.summary("the Z80 registers"))
                .post_with(set_z80, |op| {
                    op.summary("set Z80 registers")
                        .description("Set registers by name; all are checked before any changes. A pair sets its two halves.")
                }),
        )
        .api_route(
            "/api/step",
            post_with(step, |op| {
                op.summary("step: one instruction, over it, out of the routine, or a frame")
                    .description(
                        "Run one instruction, then pause (a z80_bp event, reason step); or step over it (a CALL, RST, \
                         DJNZ or block loop run whole: reason run_to), out of the routine (reason step_out), or to \
                         the end of the frame (a pause event).",
                    )
            }),
        )
}

/// The OpenAPI description of the typed routes.
pub(super) fn openapi() -> Value {
    let mut api = OpenApi {
        info: Info {
            title: "bemu debug API".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Info::default()
        },
        ..OpenApi::default()
    };
    let _ = typed_routes().finish_api(&mut api);
    serde_json::to_value(&api).unwrap_or(Value::Null)
}

/// Every path served.
fn router(shared: Shared) -> Router {
    let mut api = OpenApi::default();
    typed_routes()
        .finish_api(&mut api)
        .route("/api/events", axum::routing::get(events))
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

fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

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

/// The emulation thread's answer, described as `T` (or an [`ErrorReply`], 400).
struct Reply<T>(ApiResponse, PhantomData<fn() -> T>);

impl<T> Reply<T> {
    async fn of(shared: &Shared, request: Forwarded) -> Self {
        Reply(forward(shared, request).await, PhantomData)
    }
}

impl<T> IntoResponse for Reply<T> {
    fn into_response(self) -> Response {
        to_response(self.0)
    }
}

impl<T: JsonSchema> OperationOutput for Reply<T> {
    type Inner = T;

    fn operation_response(ctx: &mut GenContext, operation: &mut Operation) -> Option<aide::openapi::Response> {
        <axum::Json<T> as OperationOutput>::operation_response(ctx, operation)
    }

    fn inferred_responses(
        ctx: &mut GenContext,
        operation: &mut Operation,
    ) -> Vec<(Option<u16>, aide::openapi::Response)> {
        let mut responses = <axum::Json<T> as OperationOutput>::inferred_responses(ctx, operation);
        if let Some(error) = <axum::Json<ErrorReply> as OperationOutput>::operation_response(ctx, operation) {
            responses.push((Some(400), error));
        }
        responses
    }
}

// ------------------------------------------------------------------ handlers

async fn ping(State(shared): State<Shared>, request: Forwarded) -> Reply<Ping> {
    Reply::of(&shared, request).await
}

async fn get_z80(State(shared): State<Shared>, request: Forwarded) -> Reply<Z80Registers> {
    Reply::of(&shared, request).await
}

async fn set_z80(State(shared): State<Shared>, _: DocJson<SetZ80>, request: Forwarded) -> Reply<CommandAck> {
    Reply::of(&shared, request).await
}

async fn step(State(shared): State<Shared>, _: DocQuery<StepQuery>, request: Forwarded) -> Reply<CommandAck> {
    Reply::of(&shared, request).await
}

/// A route not typed yet: the emulation thread routes it (404 and 405 included).
async fn untyped(State(shared): State<Shared>, request: Forwarded) -> Response {
    to_response(forward(&shared, request).await)
}

/// The page to read the description in.
async fn doc_page() -> Html<String> {
    Html(
        aide::scalar::Scalar::new("/api/doc?format=openapi")
            .with_title("bemu debug API")
            .html(),
    )
}

/// `GET /api/events`: Server-Sent Events, the ones in `?topics=` (default all), a `: ping`
/// comment after 15s of silence.
async fn events(State(shared): State<Shared>, Forwarded(request): Forwarded) -> Response {
    let topics = request
        .query
        .iter()
        .find(|(k, _)| k == "topics")
        .map(|(_, list)| list.split(',').filter(|t| !t.is_empty()).map(str::to_owned).collect());
    let Some(queue) = shared.events.subscribe(topics) else {
        let error = format!("too many event clients (max {MAX_SSE_CLIENTS})");
        return to_response(ApiResponse::json(503, json!({ "error": error })));
    };
    // The queue is the emulation's (a mutex and a condition variable): a thread of its own waits
    // on it, and hands the events over until the client is gone.
    let (sender, receiver) = tokio::sync::mpsc::channel::<String>(64);
    let _ = std::thread::Builder::new()
        .name("debug-server-events".to_owned())
        .spawn(move || {
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
    response
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
    }
}
