//! `helium --serve`: a request and response protocol over stdio, one client per process.
//!
//! **Transport.** Requests arrive on stdin, responses leave on stdout, stderr carries the log only.
//!
//! **Framing.** As in LSP: `Content-Length: N\r\n\r\n`, then N bytes of UTF-8 JSON. A response
//! whose result has `attachment_length: M` is followed immediately by M raw bytes (no header);
//! `render` returns its PDF this way.
//!
//! **Envelope.** A request is `{"id", "op", "params"}` (`id`: integer or string). A response is
//! `{"id", "ok": true, "result"}` or `{"id", "ok": false, "error": {"code", "message"}}`. Requests
//! are answered in order.
//!
//! **Ops.** `hello`, `measure`, `layout`, `render`, `shutdown`. `measure` takes `probes`, a list of
//! `{markup | text, css}`, and returns `metrics`: one entry per probe, either `{width, ascent, descent,
//! height, ink, missing_glyphs}` or `{error: {code, message}}`. See the specification for their parameters;
//! `hello` reports the `capabilities` that a client can rely on.
//!
//! **Exit status.** 0 for `shutdown` or EOF between frames, 65 if the stream can no longer be
//! framed, 70 after a panic (the server may be in an unknown state), 71 after a timeout.
//!
//! **Trust.** The server trusts its parent process. It is not an endpoint for untrusted input.

use std::io::{BufRead, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::measure::Probe;
use crate::engine::{
    Document, DocumentKind, Engine, Error, LayoutOptions, RenderOptions, capture_warnings, note_warning,
};

/// The version of the protocol. Within a version, changes only add optional fields and ops.
pub const PROTOCOL: u64 = 1;

pub const EXIT_FRAMING: i32 = 65;
pub const EXIT_PANIC: i32 = 70;
pub const EXIT_TIMEOUT: i32 = 71;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The largest request frame in bytes.
    pub frame_size: usize,
    /// How long one request may take. Layout cannot be cancelled, so after this the process exits.
    pub request_time: Option<Duration>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { frame_size: 64 << 20, request_time: Some(Duration::from_secs(120)) }
    }
}

/// Installs a logger for the server that is like the default one (`RUST_LOG`, to stderr) and also
/// hands what is logged at `warn` or above to the request that is running, which returns it in
/// `warnings`. The log call sites are the same as for the CLI.
pub fn init_logging() {
    struct Tee(env_logger::Logger);
    impl log::Log for Tee {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Warn || self.0.enabled(metadata)
        }
        fn log(&self, record: &log::Record) {
            if record.level() <= log::Level::Warn {
                // (a warning that no request is waiting for goes to the log)
                if note_warning(record.args().to_string()) {
                    return;
                }
            }
            if self.0.enabled(record.metadata()) {
                self.0.log(record);
            }
        }
        fn flush(&self) {
            self.0.flush();
        }
    }
    let inner = env_logger::Builder::from_default_env().build();
    let max = inner.filter().max(log::LevelFilter::Warn);
    if log::set_boxed_logger(Box::new(Tee(inner))).is_ok() {
        log::set_max_level(max);
    }
}

/// A request that cannot be answered, with the code of the error.
struct Failure {
    code: &'static str,
    message: String,
}

impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Failure {
        Failure { code, message: message.into() }
    }
    fn bad(message: impl Into<String>) -> Failure {
        Failure::new("bad_request", message)
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Failure {
        Failure::new(error.code(), error.message())
    }
}

/// What a request gives back: the result and the bytes that follow the response.
struct Reply {
    result: Value,
    attachment: Vec<u8>,
}

impl Reply {
    fn plain(result: Value) -> Reply {
        Reply { result, attachment: Vec::new() }
    }
}

enum Next {
    Continue,
    Exit(i32),
}

struct Output<W> {
    writer: Mutex<W>,
}

impl<W: Write> Output<W> {
    fn send(&self, id: &Value, outcome: &Result<Reply, Failure>) -> std::io::Result<()> {
        let (envelope, attachment): (Value, &[u8]) = match outcome {
            Ok(reply) => {
                let mut result = reply.result.clone();
                if !reply.attachment.is_empty() {
                    result["attachment_length"] = json!(reply.attachment.len());
                }
                (json!({"id": id, "ok": true, "result": result}), &reply.attachment)
            }
            Err(failure) => (
                json!({"id": id, "ok": false, "error": {"code": failure.code, "message": failure.message}}),
                &[],
            ),
        };
        let body = serde_json::to_vec(&envelope).expect("JSON values serialize");
        let mut writer = self.writer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // (one write for the frame, so that a watchdog cannot cut in between the parts)
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend_from_slice(&body);
        frame.extend_from_slice(attachment);
        writer.write_all(&frame)?;
        writer.flush()
    }
}

/// What the watchdog looks at: the request that is running and when it must be done.
type Deadline = Arc<Mutex<Option<(Instant, Value)>>>;

/// Serves the requests of `input` until `shutdown`, the end of the input or a failure; returns the
/// exit status of the process.
///
/// If a request takes longer than `limits.request_time`, a watchdog thread answers it with
/// `timeout` and exits the process (the engine cannot be stopped), so this is for the process
/// whose job is to serve.
pub fn serve<R, W>(engine: &Engine, mut input: R, output: W, limits: Limits) -> i32
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let output = Arc::new(Output { writer: Mutex::new(output) });
    let deadline: Deadline = Arc::new(Mutex::new(None));
    if let Some(limit) = limits.request_time {
        let (deadline, output) = (deadline.clone(), output.clone());
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(50));
                let expired = {
                    let mut slot = deadline.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    match slot.as_ref() {
                        Some((at, _)) if Instant::now() >= *at => slot.take(),
                        _ => None,
                    }
                };
                if let Some((_, id)) = expired {
                    let failure = Failure::new("timeout", format!("the request took longer than {} s", limit.as_secs_f32()));
                    let _ = output.send(&id, &Err(failure));
                    std::process::exit(EXIT_TIMEOUT);
                }
            }
        });
    }

    loop {
        let frame = match read_frame(&mut input, limits.frame_size) {
            Ok(Some(frame)) => frame,
            Ok(None) => return 0,
            Err(FrameError::TooLarge(length)) => {
                let failure = Failure::new("too_large", format!("the request is {length} bytes, more than the limit of {}", limits.frame_size));
                if output.send(&Value::Null, &Err(failure)).is_err() {
                    return EXIT_FRAMING;
                }
                continue;
            }
            Err(FrameError::Broken(message)) => {
                log::error!("cannot read the next request: {message}");
                // (the client may still read this)
                let _ = output.send(&Value::Null, &Err(Failure::bad(message)));
                return EXIT_FRAMING;
            }
        };
        let request: Value = match serde_json::from_slice(&frame) {
            Ok(value) => value,
            Err(error) => {
                if output.send(&Value::Null, &Err(Failure::bad(format!("the request is not JSON: {error}")))).is_err() {
                    return EXIT_FRAMING;
                }
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if !(id.is_i64() || id.is_u64() || id.is_string()) {
            let failure = Failure::bad("`id` must be an integer or a string");
            if output.send(&Value::Null, &Err(failure)).is_err() {
                return EXIT_FRAMING;
            }
            continue;
        }

        if let Some(limit) = limits.request_time {
            *deadline.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((Instant::now() + limit, id.clone()));
        }
        let handled = catch_unwind(AssertUnwindSafe(|| handle(engine, &request)));
        // (taken before the answer is written, so that the watchdog does not answer a second time)
        let still_running = deadline.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take().is_some() || limits.request_time.is_none();
        if !still_running {
            // the watchdog has answered and is exiting
            std::thread::sleep(Duration::from_secs(5));
            return EXIT_TIMEOUT;
        }
        let (outcome, next) = match handled {
            Ok(handled) => handled,
            Err(panic) => {
                let what = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                log::error!("a request panicked: {what}");
                // a panic can leave a mutex poisoned or a thread-local half set: the client starts a new process
                (Err(Failure::new("internal", format!("internal error: {what}"))), Next::Exit(EXIT_PANIC))
            }
        };
        if output.send(&id, &outcome).is_err() {
            // the client is gone
            return 0;
        }
        if let Next::Exit(status) = next {
            return status;
        }
    }
}

enum FrameError {
    TooLarge(usize),
    Broken(String),
}

/// Reads one frame. `None` is the end of the input between frames.
fn read_frame<R: BufRead>(input: &mut R, limit: usize) -> Result<Option<Vec<u8>>, FrameError> {
    let mut length: Option<usize> = None;
    let mut first = true;
    loop {
        let mut line = Vec::new();
        let read = input.read_until(b'\n', &mut line).map_err(|e| FrameError::Broken(format!("read error: {e}")))?;
        if read == 0 {
            return if first { Ok(None) } else { Err(FrameError::Broken("the input ended inside of a header".into())) };
        }
        first = false;
        if line.last() != Some(&b'\n') {
            return Err(FrameError::Broken("the input ended inside of a header".into()));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        let Some((name, value)) = text.split_once(':') else {
            return Err(FrameError::Broken(format!("bad header line `{text}`")));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            length = Some(
                value.trim().parse().map_err(|_| FrameError::Broken(format!("bad Content-Length `{}`", value.trim())))?,
            );
        }
    }
    let length = length.ok_or_else(|| FrameError::Broken("the header has no Content-Length".into()))?;
    if length > limit {
        // skip the body, so that the next frame can be found
        let skipped = std::io::copy(&mut input.by_ref().take(length as u64), &mut std::io::sink())
            .map_err(|e| FrameError::Broken(format!("read error: {e}")))?;
        if skipped != length as u64 {
            return Err(FrameError::Broken("the input ended inside of a frame".into()));
        }
        return Err(FrameError::TooLarge(length));
    }
    let mut body = vec![0; length];
    input
        .read_exact(&mut body)
        .map_err(|_| FrameError::Broken("the input ended inside of a frame".into()))?;
    Ok(Some(body))
}

fn handle(engine: &Engine, request: &Value) -> (Result<Reply, Failure>, Next) {
    let Some(op) = request.get("op").and_then(Value::as_str) else {
        return (Err(Failure::bad("the request has no `op`")), Next::Continue);
    };
    let empty = Value::Object(Map::new());
    let params = request.get("params").unwrap_or(&empty);
    if !params.is_object() {
        return (Err(Failure::bad("`params` must be an object")), Next::Continue);
    }
    match op {
        "hello" => (Ok(hello(engine)), Next::Continue),
        "measure" => (op_measure(engine, params), Next::Continue),
        "layout" => (op_layout(engine, params), Next::Continue),
        "render" => (op_render(engine, params), Next::Continue),
        "shutdown" => (Ok(Reply::plain(json!({}))), Next::Exit(0)),
        other => (Err(Failure::new("unsupported_op", format!("unknown op `{other}`"))), Next::Continue),
    }
}

fn hello(engine: &Engine) -> Reply {
    let config = engine.config();
    Reply::plain(json!({
        "protocol": PROTOCOL,
        "helium": env!("CARGO_PKG_VERSION"),
        "capabilities": ["measure", "layout", "render", "warnings", "attachments"],
        "fonts": {
            "system_fonts": config.system_fonts,
            "font_dirs": config.font_dirs,
            "resource_root": config.resource_root,
        },
    }))
}

fn str_field<'a>(object: &'a Value, name: &str) -> Result<Option<&'a str>, Failure> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(Failure::bad(format!("`{name}` must be a string"))),
    }
}

fn bool_field(object: &Value, name: &str) -> Result<bool, Failure> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(Failure::bad(format!("`{name}` must be a boolean"))),
    }
}

fn parse_document(engine: &Engine, params: &Value) -> Result<Document, Failure> {
    let document = params.get("document").filter(|d| d.is_object()).ok_or_else(|| Failure::bad("`document` is missing"))?;
    let source = str_field(document, "source")?.ok_or_else(|| Failure::bad("`document.source` is missing"))?;
    let kind = match str_field(document, "kind")? {
        Some("xml") => DocumentKind::Xml,
        Some("html") => DocumentKind::Html,
        _ => return Err(Failure::bad("`document.kind` must be \"xml\" or \"html\"")),
    };
    let mut document_ = Document::new(source, kind);
    document_.base_dir = str_field(document, "base_dir")?.map(std::path::PathBuf::from);
    // a client may only point at directories inside of the resource root, if there is one
    if let (Some(dir), Some(root)) = (&document_.base_dir, &engine.config().resource_root) {
        let inside = dir
            .canonicalize()
            .ok()
            .zip(root.canonicalize().ok())
            .is_some_and(|(dir, root)| dir.starts_with(root));
        if !inside {
            return Err(Failure::bad(format!(
                "`document.base_dir` {} is not inside of the resource root {}",
                dir.display(),
                root.display()
            )));
        }
    }
    Ok(document_)
}

fn parse_layout_options(options: &Value) -> Result<LayoutOptions, Failure> {
    let crop = bool_field(options, "crop")?;
    let page = match options.get("page") {
        None | Some(Value::Null) => None,
        Some(page) => {
            let number = |name: &str| {
                page.get(name)
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .ok_or_else(|| Failure::bad(format!("`page.{name}` must be a positive number")))
            };
            Some((number("width")? as f32, number("height")? as f32))
        }
    };
    Ok(LayoutOptions { crop, page })
}

fn options_of(params: &Value) -> Value {
    params.get("options").filter(|o| o.is_object()).cloned().unwrap_or_else(|| Value::Object(Map::new()))
}

fn op_layout(engine: &Engine, params: &Value) -> Result<Reply, Failure> {
    let document = parse_document(engine, params)?;
    let options = parse_layout_options(&options_of(params))?;
    let query = params.get("query").filter(|q| q.is_object()).ok_or_else(|| Failure::bad("`query` is missing"))?;
    let dump = bool_field(query, "dump")?;
    let ids: Vec<String> = match query.get("ids") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(ids)) => ids
            .iter()
            .map(|id| id.as_str().map(str::to_owned).ok_or_else(|| Failure::bad("`query.ids` must be strings")))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(Failure::bad("`query.ids` must be an array")),
    };
    if !dump && query.get("ids").is_none() {
        return Err(Failure::bad("`query` needs `ids` or `dump`"));
    }
    let (warnings, result) = capture_warnings(|| -> Result<Value, Failure> {
        let layout = engine.layout(&document, &options)?;
        let mut result = Map::new();
        if dump {
            let dump = serde_json::from_str::<Value>(&layout.dump_json())
                .map_err(|e| Failure::new("internal", format!("the layout dump is not JSON: {e}")))?;
            result.insert("dump".into(), dump);
        }
        if !ids.is_empty() || !dump {
            let (boxes, missing) = layout.boxes(&ids);
            result.insert("boxes".into(), Value::Object(boxes));
            result.insert("missing_ids".into(), json!(missing));
        }
        if let Some(report) = layout.missing_glyphs_report() {
            result.insert("missing_glyphs".into(), json!(report));
        }
        Ok(Value::Object(result))
    });
    let mut result = result?;
    result["warnings"] = json!(warnings);
    Ok(Reply::plain(result))
}

/// The most probes in one `measure`.
const MAX_PROBES: usize = 10_000;

fn op_measure(engine: &Engine, params: &Value) -> Result<Reply, Failure> {
    let probes = params.get("probes").and_then(Value::as_array).ok_or_else(|| Failure::bad("`probes` must be an array"))?;
    if probes.len() > MAX_PROBES {
        return Err(Failure::new("too_large", format!("{} probes, more than the limit of {MAX_PROBES}", probes.len())));
    }
    // a probe that is not well-formed is an error of its entry only
    let parsed: Vec<Result<Probe, Failure>> = probes
        .iter()
        .map(|probe| {
            Ok(Probe {
                markup: str_field(probe, "markup")?.map(str::to_owned),
                text: str_field(probe, "text")?.map(str::to_owned),
                css: str_field(probe, "css")?.unwrap_or_default().to_owned(),
            })
        })
        .collect();
    let valid: Vec<Probe> = parsed.iter().filter_map(|probe| probe.as_ref().ok().cloned()).collect();
    let mut measured = engine.measure(&valid).into_iter();
    let metrics: Vec<Value> = parsed
        .iter()
        .map(|probe| match probe {
            Err(failure) => json!({"error": {"code": failure.code, "message": failure.message}}),
            Ok(_) => match measured.next().expect("one result per probe") {
                Ok(m) => json!({
                    "width": m.width, "ascent": m.ascent, "descent": m.descent, "height": m.height,
                    "ink": m.ink.map(|i| json!({"left": i.left, "top": i.top, "right": i.right, "bottom": i.bottom})),
                    "missing_glyphs": m.missing_glyphs,
                }),
                Err(error) => json!({"error": {"code": error.code(), "message": error.message()}}),
            },
        })
        .collect();
    Ok(Reply::plain(json!({"metrics": metrics})))
}

fn op_render(engine: &Engine, params: &Value) -> Result<Reply, Failure> {
    let document = parse_document(engine, params)?;
    let options = options_of(params);
    let render = RenderOptions {
        layout: parse_layout_options(&options)?,
        no_compression: bool_field(&options, "no_compression")?,
        no_tags: bool_field(&options, "no_tags")?,
        pdfa: match str_field(&options, "pdfa")? {
            None => None,
            Some(level) => Some(level.parse().map_err(|e: String| Failure::bad(format!("`options.pdfa`: {e}")))?),
        },
    };
    let rendered = engine.render(&document, &render)?;
    Ok(Reply {
        result: json!({
            "pages": rendered.pages,
            "page_size_pt": rendered.page_size_pt.map(|(w, h)| [w, h]),
            "warnings": rendered.warnings,
        }),
        attachment: rendered.pdf,
    })
}
