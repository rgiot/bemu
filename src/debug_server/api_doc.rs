//! The debug server describing itself, served by `GET /api/doc`: the endpoints (default), one in
//! detail (`?endpoint=`), the whole API as OpenAPI 3.1 (`?format=openapi`), a page to read
//! (`?format=html`); `/api/doc/ui` is an interactive reference (see `http.rs`).
//!
//! Everything is generated from the routes' types (`bemu_api`, `http.rs`): what is documented is
//! what is parsed. Kept here by hand: the conventions, an example for most operations (the tests
//! check each is accepted), and which routes are bemu's extensions ([`ROUTES`]).

use serde_json::{json, Value};

use super::{Route, ROUTES};

/// How values are written, for every operation.
const CONVENTIONS: &[&str] = &[
    "Integers in the query or in a JSON body: decimal, or hexadecimal with 0x (a JSON body also takes JSON numbers).",
    "JSON bodies are objects; a field is looked up at any depth ({\"a\":1} and {\"x\":{\"a\":1}} both carry a).",
    "A query flag is 1 or 0; an empty query value (?mode=) is no value.",
    "Errors answer 400 with {error, field?}, 404 for an unknown path, 405 (with Allow) for a method a path does not take.",
    "Commands answer cmd_seq; the events they cause (GET /api/events) carry it.",
    "Addresses are CPU addresses (the current mapping) unless a bank is given; Bnn:hhhh names offset hhhh of 16K RAM bank nn (hex).",
    "Memory is named by a location: a CPU address, read through a paging (C0-C7, &C0-&FF, Pn, L, -L, U, -U, Un, LPn, LBn, R, -R, W; default the CPU's), or SPACE:hhhh in a space: Bnn (RAM bank), LOWER, UPPERnn, CARTnn (cartridge page), ASIC."
];

/// An example of a request: its method, path, query (without `?`) and body.
pub(super) struct Example {
    pub method: &'static str,
    pub path: &'static str,
    pub query: &'static str,
    pub body: &'static str,
}

const fn example(method: &'static str, path: &'static str, query: &'static str, body: &'static str) -> Example {
    Example {
        method,
        path,
        query,
        body,
    }
}

pub(super) const EXAMPLES: &[Example] = &[
    example("POST", "/api/z80", "", r#"{"BC":"0x7f10","A":84}"#),
    example("GET", "/api/ram", "at=0xc000&len=16&paging=U7", ""),
    example("POST", "/api/ram", "", r#"{"at":"B04:0000","data":"C3 00 40"}"#),
    example("GET", "/api/memmap", "paging=C4,-L", ""),
    example("POST", "/api/exec", "", r#"{"addr":"0x4000"}"#),
    example("POST", "/api/step", "mode=over", ""),
    example("POST", "/api/run_to", "", r#"{"addr":"0x4000"}"#),
    example("POST", "/api/config", "", r#"{"paused":true}"#),
    example("POST", "/api/z80_bp", "", "0x4000, B01:0010"),
    example("DELETE", "/api/z80_bp", "", "0x4000"),
    example(
        "POST",
        "/api/breakpoints",
        "",
        r#"{"kind":"mem","access":"w","run":"watch","addr":"0xc000","size":16,"name":"screen"}"#,
    ),
    example("DELETE", "/api/breakpoints", "", ""),
    example("POST", "/api/keytype", "", r#"{"text":"PRINT 1\r"}"#),
    example("GET", "/api/history", "n=8", ""),
    example("GET", "/api/screenshot", "crop=1&live=0", ""),
    example("GET", "/api/beam", "crop=1", ""),
    example("POST", "/api/raster_bp", "line=100&nop=10", ""),
    example("POST", "/api/counters", "", r#"{"name":"loop","action":"create"}"#),
    example("DELETE", "/api/counters", "", ""),
    example("POST", "/api/basic", "", "10 PRINT \"HELLO\"\n20 GOTO 10\n"),
    example("POST", "/api/basic_step", "mode=line", ""),
    example("POST", "/api/basic_bp", "", "10, 20"),
    example("POST", "/api/basic_runto", "line=20", ""),
    example("GET", "/api/doc", "endpoint=ram", ""),
];

/// The description generated from the routes' types (see `http.rs`); `Null` where the API is
/// not served over HTTP (the web page).
fn typed() -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    return super::http::openapi();
    #[cfg(target_arch = "wasm32")]
    Value::Null
}

/// An operation, as the generated description has it.
fn operation<'t>(typed: &'t Value, method: &str, path: &str) -> Option<&'t Value> {
    typed
        .get("paths")?
        .get(path)?
        .get(method.to_ascii_lowercase())
        .filter(|op| op.is_object())
}

fn example_of(method: &str, path: &str) -> Option<&'static Example> {
    EXAMPLES.iter().find(|e| e.method == method && e.path == path)
}

fn doc_link(route: &Route) -> String {
    format!(
        "/api/doc?endpoint={}",
        route.path.trim_start_matches("/api/")
    )
}

/// `GET /api/doc`: the endpoints.
pub(super) fn index() -> Value {
    let endpoints: Vec<Value> = ROUTES
        .iter()
        .map(|route| {
            let mut endpoint =
                json!({ "path": route.path, "methods": route.methods, "summary": route.summary });
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
        "reference": "/api/doc/ui",
        "endpoints": endpoints
    })
}

/// `GET /api/doc?endpoint=`: one endpoint, its operations as OpenAPI describes them (their
/// schemas in `components`); `None` for an unknown endpoint.
pub(super) fn endpoint(name: &str) -> Option<Value> {
    let name = name.trim_start_matches('/');
    let name = name.strip_prefix("api/").unwrap_or(name);
    let route = ROUTES
        .iter()
        .find(|r| r.path.trim_start_matches("/api/") == name)?;
    let api = openapi();
    let operations: Vec<Value> = route
        .methods
        .iter()
        .filter_map(|method| {
            operation(&api, method, route.path).map(|op| {
                json!({
                    "method": method,
                    "summary": op.get("summary"),
                    "description": op.get("description"),
                    "openapi": op
                })
            })
        })
        .collect();
    let mut value = json!({
        "path": route.path,
        "summary": route.summary,
        "operations": operations,
        "components": api.get("components")
    });
    if route.extension {
        value["extension"] = json!(true);
    }
    Some(value)
}

/// `GET /api/doc?format=openapi`: the OpenAPI 3.1 description, generated, with the conventions,
/// the examples and the extensions marked (`x-bemu-extension`).
pub(super) fn openapi() -> Value {
    let mut api = typed();
    if !api.is_object() {
        api = json!({ "openapi": "3.1.0", "paths": {}, "components": { "schemas": {} } });
    }
    api["info"] = json!({
        "title": "bemu debug server",
        "version": env!("CARGO_PKG_VERSION"),
        "description": format!(
            "The AMSpiriT-Lite debug API of bemu, with extensions (x-bemu-extension). The data of \
             each event of /api/events is described in x-events.\n\n- {}",
            CONVENTIONS.join("\n- ")
        )
    });
    api["servers"] = json!([{ "url": "/" }]);
    // AMSpiriT's name for the error, as the old description had it.
    if let Some(error) = api["components"]["schemas"].get("ErrorReply").cloned() {
        api["components"]["schemas"]["Error"] = error;
    }
    for route in ROUTES {
        for method in route.methods {
            let Some(op) = api["paths"][route.path].get_mut(method.to_ascii_lowercase()) else {
                continue;
            };
            op["operationId"] = json!(format!(
                "{}_{}",
                method.to_ascii_lowercase(),
                route.path.trim_start_matches("/api/")
            ));
            if route.extension {
                op["x-bemu-extension"] = json!(true);
            }
            if let Some(example) = example_of(method, route.path) {
                if !example.query.is_empty() {
                    op["x-example-query"] = json!(example.query);
                }
                if !example.body.is_empty() {
                    if let Some(content) = op["requestBody"]["content"].as_object_mut() {
                        for (content_type, media) in content.iter_mut() {
                            media["example"] = if content_type == "application/json" {
                                serde_json::from_str(example.body).unwrap_or(json!(example.body))
                            } else {
                                json!(example.body)
                            };
                        }
                    }
                }
            }
        }
    }
    api
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A schema's name (`#/components/schemas/Name`), or its type.
fn schema_name(schema: &Value) -> String {
    if let Some(name) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.rsplit('/').next())
    {
        return name.to_owned();
    }
    match schema.get("type") {
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .filter(|t| *t != "null")
            .collect::<Vec<_>>()
            .join("|"),
        Some(Value::String(t)) if t == "array" => {
            format!("[{}]", schema.get("items").map(schema_name).unwrap_or_default())
        }
        Some(Value::String(t)) => t.clone(),
        _ => schema
            .get("allOf")
            .or(schema.get("oneOf"))
            .and_then(Value::as_array)
            .and_then(|list| list.first())
            .map(schema_name)
            .unwrap_or_else(|| "any".to_owned()),
    }
}

/// `GET /api/doc?format=html`: the same, to read in a browser (the interactive reference is
/// /api/doc/ui).
pub(super) fn html() -> String {
    let api = openapi();
    let schemas = api["components"]["schemas"].clone();
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
         Also as <a href=\"/api/doc\">JSON</a>, <a href=\"/api/doc?format=openapi\">OpenAPI 3.1</a> and \
         <a href=\"/api/doc/ui\">an interactive reference</a>. Version {}.</p><ul>",
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
    // A schema's fields, as a table.
    let fields = |schema: &Value| -> String {
        let schema = schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| r.rsplit('/').next())
            .and_then(|name| schemas.get(name))
            .unwrap_or(schema);
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return String::new();
        };
        let required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut table = String::from("<table><tr><th>field</th><th>type</th><th></th></tr>");
        for (name, field) in properties {
            table.push_str(&format!(
                "<tr><td><code>{name}</code>{}</td><td>{}</td><td>{}</td></tr>",
                if required.contains(&name.as_str()) { " (required)" } else { "" },
                escape(&schema_name(field)),
                escape(field.get("description").and_then(Value::as_str).unwrap_or(""))
            ));
        }
        table.push_str("</table>");
        table
    };
    for route in ROUTES {
        page.push_str(&format!("<h2 id=\"{0}\"><code>{0}</code>", route.path));
        if route.extension {
            page.push_str(" <span class=ext>extension</span>");
        }
        page.push_str("</h2>");
        for method in route.methods {
            let Some(op) = operation(&api, method, route.path) else {
                continue;
            };
            page.push_str(&format!(
                "<h3><code>{method} {}</code></h3><p>{}</p>",
                route.path,
                escape(op.get("description").or(op.get("summary")).and_then(Value::as_str).unwrap_or(""))
            ));
            if let Some(parameters) = op.get("parameters").and_then(Value::as_array).filter(|p| !p.is_empty()) {
                page.push_str("<table><tr><th>query</th><th>type</th><th></th></tr>");
                for p in parameters {
                    page.push_str(&format!(
                        "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                        p["name"].as_str().unwrap_or(""),
                        escape(&schema_name(&p["schema"])),
                        escape(p.get("description").and_then(Value::as_str).unwrap_or(""))
                    ));
                }
                page.push_str("</table>");
            }
            if let Some(content) = op["requestBody"]["content"].as_object() {
                for (content_type, media) in content {
                    page.push_str(&format!(
                        "<p>Body (<code>{content_type}</code>): {}</p>{}",
                        escape(&schema_name(&media["schema"])),
                        fields(&media["schema"])
                    ));
                }
            }
            if let Some(content) = op["responses"]["200"]["content"].as_object() {
                for (content_type, media) in content {
                    page.push_str(&format!(
                        "<p>Answer (<code>{content_type}</code>): {}</p>{}",
                        escape(&schema_name(&media["schema"])),
                        fields(&media["schema"])
                    ));
                }
            }
            if let Some(example) = example_of(method, route.path) {
                let mut request = format!("{method} {}", route.path);
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

    /// Every route and method has its operation generated from its types, and every one
    /// generated is served.
    #[test]
    fn every_route_and_method_is_described() {
        let api = typed();
        for route in ROUTES {
            for method in route.methods {
                assert!(
                    operation(&api, method, route.path).is_some(),
                    "{method} {} is not described",
                    route.path
                );
            }
        }
        for (path, item) in api["paths"].as_object().unwrap() {
            for method in item.as_object().unwrap().keys() {
                assert!(
                    ROUTES.iter().any(|r| r.path == path
                        && r.methods.iter().any(|m| m.eq_ignore_ascii_case(method))),
                    "{method} {path} is described but not routed"
                );
            }
        }
        for example in EXAMPLES {
            assert!(
                operation(&api, example.method, example.path).is_some(),
                "an example for {} {}, not served",
                example.method,
                example.path
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
        ApiRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            query,
            body: body.as_bytes().to_vec(),
        }
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
        server.handle(
            &request("POST", "/api/basic", "", "10 PRINT 1\n20 GOTO 10\n"),
            &mut mb,
        );
        let mut order: Vec<&Example> = EXAMPLES.iter().collect();
        // Create before delete.
        order.sort_by_key(|e| e.method == "DELETE");
        for example in order {
            let response = server.handle(
                &request(example.method, example.path, example.query, example.body),
                &mut mb,
            );
            let accepted = response.status == 200
                // The beam is known only when a monitor drives the emulation.
                || example.path == "/api/beam" && response.status == 400
                || example.path == "/api/screenshot" && response.status == 503;
            assert!(
                accepted,
                "{} {}?{}: {} {}",
                example.method,
                example.path,
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
            let ram: Value =
                serde_json::from_slice(&get(&format!("endpoint={name}")).body).unwrap();
            assert_eq!(ram["path"], "/api/ram", "{name}");
            assert_eq!(ram["operations"][0]["method"], "GET");
            assert_eq!(ram["operations"][0]["openapi"]["parameters"][0]["name"], "addr");
            assert!(ram["components"]["schemas"]["WriteRam"]["properties"]["data"].is_object());
        }
        assert_eq!(get("endpoint=nope").status, 400);
        assert_eq!(get("format=yaml").status, 400);

        let openapi: Value = serde_json::from_slice(&get("format=openapi").body).unwrap();
        assert_eq!(openapi["openapi"], "3.1.0");
        assert_eq!(openapi["paths"]["/api/step"]["post"]["parameters"][0]["name"], "mode");
        let schemas = &openapi["components"]["schemas"];
        for name in ["SetZ80", "Error", "ErrorReply", "NewBreakpoint", "StopEvent", "MemMap"] {
            assert!(schemas[name].is_object(), "{name}");
        }
        assert_eq!(openapi["x-events"]["z80_bp"]["$ref"], "#/components/schemas/StopEvent");
        let breakpoints = &openapi["paths"]["/api/breakpoints"];
        assert_eq!(breakpoints["post"]["x-bemu-extension"], true);
        assert_eq!(breakpoints["post"]["operationId"], "post_breakpoints");
        assert_eq!(breakpoints["delete"]["parameters"][0]["name"], "index");
        assert!(breakpoints["post"]["responses"]["400"].is_object());
        assert_eq!(
            openapi["paths"]["/api/media"]["post"]["requestBody"]["content"]
                ["application/octet-stream"]["schema"]["format"],
            "binary"
        );
        assert_eq!(
            openapi["paths"]["/api/z80_bp"]["post"]["requestBody"]["content"]["text/plain"]
                ["example"],
            "0x4000, B01:0010"
        );
        assert_eq!(
            openapi["paths"]["/api/screenshot"]["get"]["responses"]["200"]["content"]["image/png"]
                ["schema"]["format"],
            "binary"
        );

        let html = get("format=html");
        assert_eq!(html.content_type, "text/html; charset=utf-8");
        let html = String::from_utf8(html.body).unwrap();
        assert!(html.contains("<h2 id=\"/api/raster_bp\">"));
        assert!(html.contains("&quot;screen&quot;"), "examples escaped");
        assert!(html.contains("<code>crtc_type</code>"), "a reply's fields");
    }
}
