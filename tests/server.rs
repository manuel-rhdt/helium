//! `helium --serve` (see `helium::server`) and the library entry point.
mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use assert_cmd::prelude::*;
use serde_json::{Value, json};

struct Server {
    child: Child,
    stdout: BufReader<std::process::ChildStdout>,
}

fn frame(value: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend(body);
    out
}

struct Response {
    json: Value,
    attachment: Vec<u8>,
}

impl Server {
    fn start() -> Server {
        let mut command = Command::cargo_bin("helium").unwrap();
        command.args(["--serve", "--no-system-fonts"]);
        Server::spawn(command)
    }

    fn spawn(mut command: Command) -> Server {
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server { child, stdout }
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }

    fn send(&mut self, id: impl Into<Value>, op: &str, params: Value) {
        self.send_raw(&frame(&json!({"id": id.into(), "op": op, "params": params})));
    }

    fn receive(&mut self) -> Option<Response> {
        let mut length = None;
        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line).unwrap() == 0 {
                return None;
            }
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.expect("a Content-Length header")];
        self.stdout.read_exact(&mut body).unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let mut attachment = Vec::new();
        if let Some(n) = json["result"]["attachment_length"].as_u64() {
            attachment = vec![0; n as usize];
            self.stdout.read_exact(&mut attachment).unwrap();
        }
        Some(Response { json, attachment })
    }

    fn call(&mut self, id: i64, op: &str, params: Value) -> Response {
        self.send(id, op, params);
        let response = self.receive().expect("a response");
        assert_eq!(response.json["id"], json!(id));
        response
    }

    fn finish(mut self) -> i32 {
        drop(self.child.stdin.take());
        self.child.wait().unwrap().code().unwrap()
    }
}

fn document(name: &str) -> (String, Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/documents").join(name);
    let source = std::fs::read_to_string(&path).unwrap();
    let kind = if name.ends_with(".svg") { "xml" } else { "html" };
    let dir = path.parent().unwrap().to_str().unwrap().to_owned();
    (path.to_str().unwrap().to_owned(), json!({"source": source, "kind": kind, "base_dir": dir}))
}

fn cli(input: &str, args: &[&str]) -> Vec<u8> {
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("server-cli-{}.pdf", std::process::id()));
    let mut cmd = common::helium().unwrap();
    cmd.args(args).arg(input).arg(&output);
    cmd.assert().success();
    let bytes = std::fs::read(&output).unwrap();
    std::fs::remove_file(&output).ok();
    bytes
}

#[test]
fn hello_and_shutdown() {
    let mut server = Server::start();
    let hello = server.call(1, "hello", json!({"protocol": 1}));
    assert_eq!(hello.json["ok"], json!(true));
    assert_eq!(hello.json["result"]["protocol"], json!(1));
    assert_eq!(hello.json["result"]["fonts"]["system_fonts"], json!(false));
    let caps = hello.json["result"]["capabilities"].as_array().unwrap();
    assert!(caps.contains(&json!("render")) && caps.contains(&json!("layout")));
    server.call(2, "shutdown", json!({}));
    assert!(server.receive().is_none());
    assert_eq!(server.finish(), 0);
}

#[test]
fn string_ids_are_echoed_and_requests_can_be_pipelined() {
    let mut server = Server::start();
    server.send("a", "hello", json!({}));
    server.send(7, "hello", json!({}));
    server.send("c", "nope", json!({}));
    assert_eq!(server.receive().unwrap().json["id"], json!("a"));
    assert_eq!(server.receive().unwrap().json["id"], json!(7));
    let last = server.receive().unwrap().json;
    assert_eq!(last["id"], json!("c"));
    assert_eq!(last["error"]["code"], json!("unsupported_op"));
    assert_eq!(server.finish(), 0);
}

#[test]
fn render_is_byte_identical_to_the_cli() {
    let mut server = Server::start();
    for name in ["loremipsum.html", "flexbox.html", "gaussian_integral.html"] {
        let (path, doc) = document(name);
        let response = server.call(1, "render", json!({"document": doc}));
        assert_eq!(response.json["ok"], json!(true), "{name}: {}", response.json);
        assert!(response.attachment.starts_with(b"%PDF-"));
        assert_eq!(response.attachment.len() as u64, response.json["result"]["attachment_length"].as_u64().unwrap());
        assert!(response.attachment == cli(&path, &[]), "{name}: the PDF differs from the CLI's");
    }
    // the options of the CLI
    let (path, doc) = document("loremipsum.html");
    let response = server.call(
        2,
        "render",
        json!({"document": doc, "options": {"crop": true, "no_compression": true, "no_tags": true}}),
    );
    assert!(response.attachment == cli(&path, &["--crop", "--no-compression", "--no-tags"]));
    assert_eq!(server.finish(), 0);
}

#[test]
fn layout_dump_is_the_json_of_dump_layout() {
    let (path, doc) = document("flexbox.html");
    let mut server = Server::start();
    let response = server.call(1, "layout", json!({"document": doc, "query": {"dump": true}}));
    let dump_file = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("server-dump-{}.json", std::process::id()));
    let pdf_file = dump_file.with_extension("pdf");
    common::helium().unwrap().args(["--dump-layout"]).arg(&dump_file).arg(&path).arg(&pdf_file).assert().success();
    let expected: Value = serde_json::from_slice(&std::fs::read(&dump_file).unwrap()).unwrap();
    assert_eq!(response.json["result"]["dump"], expected);
    std::fs::remove_file(dump_file).ok();
    std::fs::remove_file(pdf_file).ok();
    server.finish();
}

#[test]
fn layout_boxes_of_elements_by_id() {
    let html = r#"<html><body style="margin:0"><div id="a" style="width:100px;height:50px;padding:5px;margin:10px">x</div></body></html>"#;
    let mut server = Server::start();
    let response = server.call(
        1,
        "layout",
        json!({"document": {"source": html, "kind": "html"}, "options": {"page": {"width": 400, "height": 300}},
               "query": {"ids": ["a", "missing"]}}),
    );
    let result = &response.json["result"];
    assert_eq!(result["missing_ids"], json!(["missing"]));
    let a = &result["boxes"]["a"];
    assert_eq!(a["content_box"]["width"].as_f64(), Some(100.0));
    assert_eq!(a["border_box"]["width"].as_f64(), Some(110.0));
    assert_eq!(a["padding"]["left"].as_f64(), Some(5.0));
    assert_eq!(a["margin"]["top"].as_f64(), Some(10.0));
    assert_eq!(a["page"], json!(0));
    server.finish();
}

#[test]
fn errors_have_codes() {
    let mut server = Server::start();
    let bad_xml = server.call(1, "layout", json!({"document": {"source": "<a>", "kind": "xml"}, "query": {"dump": true}}));
    assert_eq!(bad_xml.json["error"]["code"], json!("parse_error"));
    let bad_params = server.call(2, "render", json!({"document": {"kind": "html"}}));
    assert_eq!(bad_params.json["error"]["code"], json!("bad_request"));
    let bad_option = server.call(
        3,
        "render",
        json!({"document": {"source": "<p>x</p>", "kind": "html"}, "options": {"pdfa": "2a", "no_tags": true}}),
    );
    assert_eq!(bad_option.json["error"]["code"], json!("bad_request"));
    // a failed request changes nothing for the next one
    let (_, doc) = document("loremipsum.html");
    assert!(server.call(4, "render", json!({"document": doc})).json["ok"].as_bool().unwrap());
    server.finish();
}

#[test]
fn not_json_is_a_bad_request_with_a_null_id() {
    let mut server = Server::start();
    server.send_raw(&frame(&json!("x")).iter().map(|b| *b).collect::<Vec<_>>());
    let response = server.receive().unwrap().json;
    assert_eq!(response["error"]["code"], json!("bad_request"));
    assert_eq!(response["id"], Value::Null);
    server.send_raw(b"Content-Length: 5\r\n\r\n{nope");
    let response = server.receive().unwrap().json;
    assert_eq!(response["error"]["code"], json!("bad_request"));
    // still in sync
    assert_eq!(server.call(1, "hello", json!({}))["ok"].clone(), json!(true));
    assert_eq!(server.finish(), 0);
}

impl std::ops::Index<&str> for Response {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        &self.json[key]
    }
}

#[test]
fn a_truncated_frame_exits_with_65() {
    let mut server = Server::start();
    server.send_raw(b"Content-Length: 100\r\n\r\n{\"id\":1");
    assert_eq!(server.finish(), 65);
    let mut server = Server::start();
    server.send_raw(b"Content-Length: nope\r\n\r\n");
    assert_eq!(server.finish(), 65);
}

#[test]
fn a_frame_over_the_limit_is_skipped() {
    let mut command = Command::cargo_bin("helium").unwrap();
    command.args(["--serve", "--no-system-fonts"]);
    let mut server = Server::spawn(command);
    // 70 MiB of body
    let body = vec![b' '; 70 << 20];
    server.send_raw(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    let writer = server.child.stdin.take().unwrap();
    let thread = std::thread::spawn(move || {
        let mut writer = writer;
        writer.write_all(&body).unwrap();
        writer
    });
    let response = server.receive().unwrap().json;
    assert_eq!(response["error"]["code"], json!("too_large"));
    server.child.stdin = Some(thread.join().unwrap());
    assert_eq!(server.call(1, "hello", json!({})).json["ok"], json!(true));
    server.finish();
}

#[test]
fn warnings_come_back_with_the_request() {
    let mut server = Server::start();
    let doc = json!({"source": "<html><body><p>x</p></body></html>", "kind": "html"});
    let first = server.call(1, "render", json!({"document": doc}));
    let warnings = first.json["result"]["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("lang")), "{warnings:?}");
    let with_lang = json!({"source": "<html lang='en'><head><title>t</title></head><body><p>x</p></body></html>", "kind": "html"});
    let second = server.call(2, "render", json!({"document": with_lang}));
    assert_eq!(second.json["result"]["warnings"], json!([]), "a request sees only its own warnings");
    server.finish();
}

#[test]
fn results_do_not_depend_on_earlier_requests() {
    let docs: Vec<Value> = ["loremipsum.html", "flexbox.html", "book.html"].iter().map(|n| document(n).1).collect();
    let fresh: Vec<Vec<u8>> = docs
        .iter()
        .map(|doc| {
            let mut server = Server::start();
            let pdf = server.call(1, "render", json!({"document": doc})).attachment;
            server.finish();
            pdf
        })
        .collect();
    let mut server = Server::start();
    for round in 0..3 {
        for (i, doc) in docs.iter().enumerate().rev() {
            // an error in between
            server.call(0, "render", json!({"document": {"source": "<a>", "kind": "xml"}}));
            let pdf = server.call(1, "render", json!({"document": doc})).attachment;
            assert!(pdf == fresh[i], "round {round}, document {i} differs from a fresh process");
        }
    }
    server.finish();
}

#[test]
fn the_cli_reads_stdin_and_writes_stdout() {
    let (path, _) = document("loremipsum.html");
    let source = std::fs::read(&path).unwrap();
    let mut cmd = common::helium().unwrap();
    cmd.args(["--html", "-", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&source).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.starts_with(b"%PDF-"));
}

#[test]
fn the_engine_renders_the_same_bytes_as_the_cli() {
    use helium::{Document, DocumentKind, Engine, EngineConfig, RenderOptions};
    let config = EngineConfig { system_fonts: false, ..Default::default() };
    let engine = Engine::new(config.clone()).unwrap();
    let (path, _) = document("loremipsum.html");
    let mut doc = Document::new(std::fs::read_to_string(&path).unwrap(), DocumentKind::Html);
    doc.base_dir = Path::new(&path).parent().map(Into::into);
    let rendered = engine.render(&doc, &RenderOptions::default()).unwrap();
    assert!(rendered.pdf == cli(&path, &[]));
    assert!(rendered.pages >= 1);
    // the same settings again are fine; other ones are an error
    assert!(Engine::new(config).is_ok());
    let other = EngineConfig { system_fonts: true, ..Default::default() };
    assert!(Engine::new(other).is_err());
}
