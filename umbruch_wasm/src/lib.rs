//! Umbruch in the browser. `render(html)` lays an HTML document out and writes it as a PDF; the
//! warnings of the document (unknown CSS, missing glyphs, ...) come back with it.
//!
//! The engine runs on the calling thread and reads no files: the fonts are built in and only
//! `data:` URIs load. Run it in a worker if the page must stay responsive while it renders.

use umbruch::{Document, DocumentKind, Engine, EngineConfig, RenderOptions};
use wasm_bindgen::prelude::*;

/// What `render` returns: either the PDF, or the reason there is none. `log` holds the warnings
/// in both cases.
#[wasm_bindgen]
pub struct Rendered {
    pdf: Option<Vec<u8>>,
    pages: usize,
    log: String,
    error: Option<String>,
}

#[wasm_bindgen]
impl Rendered {
    /// The PDF, or `undefined` if the document could not be rendered.
    #[wasm_bindgen(getter)]
    pub fn pdf(&self) -> Option<Vec<u8>> {
        self.pdf.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn pages(&self) -> usize {
        self.pages
    }

    /// One line per warning.
    #[wasm_bindgen(getter)]
    pub fn log(&self) -> String {
        self.log.clone()
    }

    /// Why there is no PDF.
    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }
}

/// Hands what is logged at `warn` or above to the render call that is running (the others are
/// dropped: there is nobody to read them).
struct Warnings;

impl log::Log for Warnings {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            umbruch::engine::note_warning(format!("[{} {}] {}", record.level(), record.target(), record.args()));
        }
    }
    fn flush(&self) {}
}

thread_local! {
    static ENGINE: Engine = {
        let _ = log::set_logger(&Warnings).map(|()| log::set_max_level(log::LevelFilter::Warn));
        Engine::new(EngineConfig { system_fonts: false, font_dirs: Vec::new(), resource_root: None })
            .expect("the engine is set up once")
    };
}

/// Called once by the generated glue: a panic in Rust shows up in the browser console with its
/// message instead of as `unreachable`.
#[wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(|info| wasm_bindgen::throw_str(&info.to_string())));
}

/// Renders an HTML document to a PDF. It does not throw for a document that cannot be rendered:
/// look at `error` of the result.
#[wasm_bindgen]
pub fn render(html: &str) -> Rendered {
    let document = Document::new(html, DocumentKind::Html);
    let options = RenderOptions::default();
    match ENGINE.with(|engine| engine.render(&document, &options)) {
        Ok(rendered) => Rendered {
            pdf: Some(rendered.pdf),
            pages: rendered.pages,
            log: rendered.warnings.join("\n"),
            error: None,
        },
        Err(error) => Rendered { pdf: None, pages: 0, log: String::new(), error: Some(error.to_string()) },
    }
}
