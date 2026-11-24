//! The library entry point: [`Engine`] turns a [`Document`] into a PDF ([`Engine::render`]) or
//! into the computed layout ([`Engine::layout`]), without a process per document.
//!
//! Process-wide settings (system fonts, extra font directories, the resource root) are fixed by
//! [`Engine::new`]; everything else is per call. Every call runs to completion on the calling
//! thread. A [`LayoutResult`] holds the DOM (which uses `Rc`) and is not `Send`.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::info;
use specs::WorldExt;

use crate::display_list;
use crate::layout::{self, components::LayoutContext};
use crate::parser;
use crate::pdf::{self, PdfALevel};
use crate::style::device::Device;
use crate::style::properties::computed::PixelLength;
use crate::tree::DomRef;

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The options contradict each other or the engine was set up twice with different settings.
    BadRequest(String),
    /// The document is not well-formed XML (the message is the parser's, as the CLI prints it).
    Parse(String),
    /// The document could not be laid out.
    Layout(String),
    /// The PDF could not be written.
    Render(String),
}

impl Error {
    /// The protocol code of the error (see [`crate::server`]).
    pub fn code(&self) -> &'static str {
        match self {
            Error::BadRequest(_) => "bad_request",
            Error::Parse(_) => "parse_error",
            Error::Layout(_) => "layout_error",
            Error::Render(_) => "render_error",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Error::BadRequest(m) | Error::Parse(m) | Error::Layout(m) | Error::Render(m) => m,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The process-wide settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// Use the fonts installed on the system, not only the built-in ones.
    pub system_fonts: bool,
    /// Directories with fonts to use in addition to the others.
    pub font_dirs: Vec<PathBuf>,
    /// The directory that the files of a document must be inside of. Without it, a document may
    /// use the files that are inside of its own directory.
    pub resource_root: Option<PathBuf>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig { system_fonts: true, font_dirs: Vec::new(), resource_root: None }
    }
}

static CONFIG: Mutex<Option<EngineConfig>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentKind {
    /// Well-formed XML: an SVG or XHTML document.
    Xml,
    /// HTML, which the parser repairs.
    Html,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub source: String,
    pub kind: DocumentKind,
    /// The directory that relative URLs (images, style sheets) are relative to. Without it, only
    /// `data:` URIs load.
    pub base_dir: Option<PathBuf>,
    /// What the parse errors call the document (the file name, for the CLI).
    pub name: Option<String>,
}

impl Document {
    pub fn new(source: impl Into<String>, kind: DocumentKind) -> Self {
        Document { source: source.into(), kind, base_dir: None, name: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutOptions {
    /// The page is as large as the content (an SVG is always its own size).
    pub crop: bool,
    /// The page size in CSS pixels; the default is A4. Ignored with `crop`.
    pub page: Option<(f32, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RenderOptions {
    pub layout: LayoutOptions,
    pub no_compression: bool,
    /// Write an untagged PDF.
    pub no_tags: bool,
    pub pdfa: Option<PdfALevel>,
}

impl RenderOptions {
    fn check(&self) -> Result<()> {
        if self.no_tags && self.pdfa.is_some_and(|level| level.needs_tags()) {
            return Err(Error::BadRequest(
                "PDF/A level `a` needs the structure tree, so it cannot be used with no_tags".into(),
            ));
        }
        Ok(())
    }
}

/// A PDF and what is known about it.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub pdf: Vec<u8>,
    pub pages: usize,
    /// The size of the first page in points.
    pub page_size_pt: Option<(f32, f32)>,
    /// What was unsupported or missing, from this call only.
    pub warnings: Vec<String>,
}

pub struct Engine {
    config: EngineConfig,
    pub(crate) measure_cache: Mutex<crate::measure::Cache>,
}

impl Engine {
    /// Installs the process-wide settings. A second engine is fine if its settings are the same;
    /// different ones are an error, because the fonts and the resource root cannot change once
    /// they were read.
    pub fn new(config: EngineConfig) -> Result<Engine> {
        let mut installed = CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match installed.as_ref() {
            Some(existing) if *existing != config => {
                return Err(Error::BadRequest(
                    "the engine was already set up with different font or resource settings".into(),
                ));
            }
            Some(_) => {}
            None => {
                layout::text::font::set_use_system_fonts(config.system_fonts);
                layout::text::font::set_font_dirs(config.font_dirs.clone());
                if let Some(root) = &config.resource_root {
                    crate::image::set_resource_root(root.clone());
                }
                *installed = Some(config.clone());
            }
        }
        Ok(Engine { config, measure_cache: Mutex::default() })
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Lays the document out.
    pub fn layout(&self, document: &Document, options: &LayoutOptions) -> Result<LayoutResult> {
        let base_dir = self.base_dir(document)?;
        let device = if options.crop {
            // dummy device. Final page size is determined by cropping
            Device { width: PixelLength(0.0), height: PixelLength(0.0) }
        } else {
            let (width, height) = options.page.unwrap_or((793.7, 1123.0));
            Device { width: PixelLength(width), height: PixelLength(height) }
        };
        let name = document.name.clone();
        // (a document with cross-references is laid out more than once, from a new tree each time)
        let parse = || -> std::result::Result<DomRef, String> {
            match document.kind {
                DocumentKind::Xml => parser::generate_render_tree_xml(&document.source)
                    .and_then(|dom| parser::into_html_document(&dom))
                    .map_err(|message| match &name {
                        Some(name) => format!("{name}: {message}"),
                        None => message,
                    }),
                DocumentKind::Html => Ok(parser::generate_render_tree(&document.source)),
            }
        };
        let started = std::time::Instant::now();
        let (dom, context) = layout::layout_resolved(&parse, device, options.crop, &base_dir).map_err(|message| {
            // the only error before the layout proper is the parse of an XML document
            if document.kind == DocumentKind::Xml { Error::Parse(message) } else { Error::Layout(message) }
        })?;
        info!("TIMED: layout {:?}", started.elapsed());
        Ok(LayoutResult { dom, context, crop: options.crop })
    }

    /// Lays the document out and writes it as a PDF to memory.
    pub fn render(&self, document: &Document, options: &RenderOptions) -> Result<Rendered> {
        options.check()?;
        let (warnings, result) = capture_warnings(|| {
            let mut layout = self.layout(document, &options.layout)?;
            let missing = layout.missing_glyphs_report();
            let mut pdf = Vec::new();
            let summary = layout.write_pdf(options, &mut pdf)?;
            Ok::<_, Error>((pdf, summary, missing))
        });
        let (pdf, (pages, page_size_pt), missing) = result?;
        let mut warnings = warnings;
        warnings.extend(missing);
        Ok(Rendered { pdf, pages, page_size_pt, warnings })
    }

    /// Where relative URLs are resolved: the directory of the document. Without one, nothing
    /// outside of `data:` URIs can be found: the path does not exist.
    fn base_dir(&self, document: &Document) -> Result<PathBuf> {
        // (the directory need not be inside of the resource root: it is the files that are checked)
        match &document.base_dir {
            Some(dir) => Ok(dir.clone()),
            None => Ok(Path::new("/nonexistent/helium-no-base-dir").to_path_buf()),
        }
    }
}

/// A laid out document.
pub struct LayoutResult {
    dom: DomRef,
    context: LayoutContext,
    crop: bool,
}

impl LayoutResult {
    pub fn dom(&self) -> &DomRef {
        &self.dom
    }

    pub fn context(&self) -> &LayoutContext {
        &self.context
    }

    /// The report of the characters that no font had, if there are any (what the CLI prints).
    pub fn missing_glyphs_report(&self) -> Option<String> {
        self.context.read_resource::<layout::text::fallback::MissingGlyphs>().report()
    }

    /// The whole layout as the JSON of `--dump-layout`.
    pub fn dump_json(&self) -> String {
        layout::dump::dump_layout(&self.dom, &self.context)
    }

    /// The boxes of the elements with these ids (`page`, `border_box`, `content_box`, `padding`,
    /// `border` and `margin`), and the ids that no element with a box has.
    pub fn boxes(&self, ids: &[String]) -> (serde_json::Map<String, serde_json::Value>, Vec<String>) {
        layout::dump::element_boxes(&self.dom, &self.context, ids)
    }

    /// Evaluates the `data-expected-*` attributes of the document against the layout.
    pub fn check(&self) -> layout::check::CheckReport {
        layout::check::check_layout(&self.dom, &self.context)
    }

    /// Writes the pages as PDF. Returns the number of pages and the size of the first one in points.
    pub fn write_pdf<W: Write>(&mut self, options: &RenderOptions, output: W) -> Result<(usize, Option<(f32, f32)>)> {
        options.check()?;
        let pages = display_list::generate_display_list(&mut self.context);
        let count = pages.pages.len();
        let size = pages.pages.first().map(|page| {
            let (w, h) = if self.crop { (page.content_rect.width, page.content_rect.height) } else { (page.width, page.height) };
            (w.0 * 0.75, h.0 * 0.75)
        });
        pdf::write_pdf(output, pages, options.no_compression, self.crop, !options.no_tags, options.pdfa)
            .map_err(|error| Error::Render(error.to_string()))?;
        Ok((count, size))
    }
}

thread_local! {
    static MUTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static WARNINGS: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Called by the logger of the server for every record at `warn` or above: the message belongs to
/// the call that [`capture_warnings`] runs, if there is one.
pub fn note_warning(message: String) -> bool {
    if MUTED.with(|muted| muted.get()) {
        return true;
    }
    WARNINGS.with(|sink| match sink.borrow_mut().as_mut() {
        Some(list) => {
            list.push(message);
            true
        }
        None => false,
    })
}

/// Runs `f` and returns what it logged at `warn` (needs the logger of [`crate::server`]) and the
/// characters that no font had, together with its result. The sink is removed again also when `f`
/// panics.
pub fn capture_warnings<R>(f: impl FnOnce() -> R) -> (Vec<String>, R) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            WARNINGS.with(|sink| *sink.borrow_mut() = None);
        }
    }
    WARNINGS.with(|sink| *sink.borrow_mut() = Some(Vec::new()));
    let _reset = Reset;
    let result = f();
    let warnings = WARNINGS.with(|sink| sink.borrow_mut().take()).unwrap_or_default();
    (warnings, result)
}

/// Runs `f` without its warnings reaching [`capture_warnings`]: they are about something that the
/// caller of the engine did not write (the built-in style sheet), so they are not its to act on.
pub(crate) fn without_warnings<R>(f: impl FnOnce() -> R) -> R {
    struct Unmute(bool);
    impl Drop for Unmute {
        fn drop(&mut self) {
            MUTED.with(|muted| muted.set(self.0));
        }
    }
    let _unmute = Unmute(MUTED.with(|muted| muted.replace(true)));
    f()
}
