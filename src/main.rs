use structopt::StructOpt;

use derive_more::From;

use std::io::{Read, Write};
use std::path::PathBuf;

use umbruch::engine::{Document, DocumentKind, Engine, EngineConfig, LayoutOptions, RenderOptions};
use umbruch::layout;
use umbruch::pdf::PdfALevel;

#[derive(StructOpt, Debug)]
#[structopt(name = "umbruch")]
struct Options {
    #[structopt(long = "no-compression")]
    no_filter: bool,
    #[structopt(long = "crop")]
    crop: bool,
    /// Write an untagged PDF: without the structure tree (the logical structure of the document
    /// for assistive technology, reflow and text extraction) that is derived from the HTML.
    #[structopt(long = "no-tags")]
    no_tags: bool,
    /// Write a file that conforms to this level of PDF/A: 1b, 2a, 2b, 2u, 3a, 3b or 3u. The level `a`
    /// needs the structure tree, so it does not go with `--no-tags`. Colours are sRGB (the output
    /// intent is a profile that is made in code), so a CMYK image is an error. PDF/A-1 (1b) does not
    /// allow transparency, colour glyphs or ICC profiles of version 4: a document that needs one is an
    /// error. The other things that PDF/A forbids are not checked.
    #[structopt(long = "pdfa", value_name = "LEVEL")]
    pdfa: Option<PdfALevel>,
    /// Write the computed layout as JSON to this file (use `-` for stdout)
    #[structopt(long = "dump-layout", parse(from_os_str))]
    dump_layout: Option<PathBuf>,
    /// Write the choices of the paginator as JSON to this file (use `-` for stdout): for every page
    /// what it holds, what it costs and why (room, orphans and widows, avoided breaks, the cost of
    /// the lines), the ways to break its paragraphs that were chosen, and the ones that the
    /// paragraphs had. The values that are tried in experiments are `UMBRUCH_LINE_TOLERANCE` and
    /// `UMBRUCH_VARIANT_TOLERANCE`.
    #[structopt(long = "dump-pagination", parse(from_os_str))]
    dump_pagination: Option<PathBuf>,
    /// Evaluate the `data-expected-*` attributes (web-platform-tests `checkLayout`) of the input
    /// document against the computed layout and exit with an error if any expectation fails.
    #[structopt(long = "check-layout")]
    check_layout: bool,
    /// Use only the fonts built into Umbruch, not the fonts installed on the system, so that the
    /// output does not depend on the machine. Also enabled by setting the environment variable
    /// `UMBRUCH_NO_SYSTEM_FONTS` (to anything but `0`).
    #[structopt(long = "no-system-fonts")]
    no_system_fonts: bool,
    /// A directory with fonts to use in addition to the others (can be given several times)
    #[structopt(long = "font-dir", parse(from_os_str))]
    font_dirs: Vec<PathBuf>,
    /// Allow the document to use files (images) from anywhere inside of this directory, instead
    /// of only from the directory of the document
    #[structopt(long = "resource-root", parse(from_os_str))]
    resource_root: Option<PathBuf>,
    /// Read the input as XML (an SVG or XHTML document: well-formed, with namespaces). This is the
    /// default for the extensions `.svg`, `.xhtml`, `.xht` and `.xml`.
    #[structopt(long = "xml")]
    xml_flag: bool,
    /// Read the input with the HTML parser, which repairs what is not well-formed. This is the
    /// default for every other extension, and for the input `-` (standard input).
    #[structopt(long = "html", conflicts_with = "xml_flag")]
    html_flag: bool,
    /// Serve requests on standard input and output instead of converting one file: a request and
    /// response protocol with `Content-Length` framing (`hello`, `measure`, `layout`, `render` and
    /// `shutdown`; see the documentation of `umbruch::server`). Standard error carries the log only.
    #[structopt(long = "serve", conflicts_with_all = &["dump_layout", "dump_pagination", "check_layout"])]
    serve: bool,
    /// The input file to process: HTML, or SVG (which may hold HTML and MathML in `foreignObject`).
    /// Use `-` for standard input.
    #[structopt(parse(from_os_str), required_unless = "serve")]
    input: Option<PathBuf>,
    /// Path where the output PDF file will be stored. Use `-` for standard output.
    #[structopt(parse(from_os_str), required_unless = "serve")]
    output: Option<PathBuf>,
}

#[derive(Debug, From)]
pub enum Error {
    Io(std::io::Error),
    Layout(String),
}

fn main() -> Result<(), Error> {
    let opt = Options::from_args();
    let no_system_fonts = opt.no_system_fonts
        || std::env::var_os("UMBRUCH_NO_SYSTEM_FONTS").is_some_and(|v| !v.is_empty() && v != "0");
    let config = EngineConfig {
        system_fonts: !no_system_fonts,
        font_dirs: opt.font_dirs.clone(),
        resource_root: opt.resource_root.clone(),
    };

    if opt.serve {
        // the warnings of a request come back with its response
        umbruch::server::init_logging();
        let engine = Engine::new(config).map_err(|e| Error::Layout(e.to_string()))?;
        let status = umbruch::server::serve(&engine, std::io::stdin().lock(), std::io::stdout(), umbruch::server::Limits::default());
        std::process::exit(status);
    }

    env_logger::init();
    if opt.no_tags && opt.pdfa.is_some_and(|level| level.needs_tags()) {
        eprintln!("error: PDF/A level `a` needs the structure tree, so it cannot be used with --no-tags");
        std::process::exit(2);
    }
    let engine = Engine::new(config).map_err(|e| Error::Layout(e.to_string()))?;

    let input = opt.input.clone().expect("required unless --serve");
    let output = opt.output.clone().expect("required unless --serve");
    let from_stdin = input.as_os_str() == "-";
    let to_stdout = output.as_os_str() == "-";
    let stdout_is_dump = [&opt.dump_layout, &opt.dump_pagination].iter().any(|p| p.as_ref().is_some_and(|p| p.as_os_str() == "-"));
    if to_stdout && (stdout_is_dump || opt.check_layout) {
        eprintln!("error: the PDF is written to standard output, which the other output cannot share");
        std::process::exit(2);
    }

    let (text, input_dir) = if from_stdin {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes)?;
        (String::from_utf8_lossy(&bytes).into_owned(), PathBuf::from("."))
    } else {
        let dir = input
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        (String::from_utf8_lossy(&std::fs::read(&input)?).into_owned(), dir)
    };
    // the parser follows the kind of file: XML for SVG and XHTML, HTML for the rest
    let is_xml = if opt.xml_flag {
        true
    } else if opt.html_flag || from_stdin {
        false
    } else {
        input.extension().is_some_and(|ext| {
            ["svg", "xhtml", "xht", "xml"].iter().any(|xml| ext.eq_ignore_ascii_case(xml))
        })
    };
    let document = Document {
        source: text,
        kind: if is_xml { DocumentKind::Xml } else { DocumentKind::Html },
        base_dir: Some(input_dir),
        name: Some(if from_stdin { "<stdin>".to_owned() } else { input.display().to_string() }),
    };
    let render_options = RenderOptions {
        layout: LayoutOptions { crop: opt.crop, page: None },
        no_compression: opt.no_filter,
        no_tags: opt.no_tags,
        pdfa: opt.pdfa,
    };

    layout::report_pagination(opt.dump_pagination.is_some());
    let mut layout = engine
        .layout(&document, &render_options.layout)
        .map_err(|e| Error::Layout(e.to_string()))?;

    if let Some(report) = layout.missing_glyphs_report() {
        eprintln!("{}", report);
    }

    if let Some(path) = &opt.dump_layout {
        let json = layout.dump_json();
        if path.as_os_str() == "-" {
            print!("{}", json);
        } else {
            std::fs::write(path, json)?;
        }
    }

    if let Some(path) = &opt.dump_pagination {
        let json = layout::take_pagination_report().unwrap_or_default();
        if path.as_os_str() == "-" {
            println!("{}", json);
        } else {
            std::fs::write(path, json)?;
        }
    }

    // the PDF is still written for failing checks so that it can be inspected
    let check_exit_code = if opt.check_layout {
        let report = layout.check();
        for line in &report.unsupported {
            eprintln!("UNSUPPORTED {}", line);
        }
        for line in &report.failures {
            eprintln!("FAIL {}", line);
        }
        println!("checked {} expectations, {} failed", report.checked, report.failures.len());
        if !report.failures.is_empty() {
            Some(1)
        } else if report.checked == 0 {
            eprintln!("no checkable expectations found");
            Some(2)
        } else {
            None
        }
    } else {
        None
    };

    // (the file is made only once the PDF is complete, so there is none for a document that cannot be written)
    let mut pdf = Vec::new();
    if let Err(error) = layout.write_pdf(&render_options, &mut pdf) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
    if to_stdout {
        let mut out = std::io::stdout().lock();
        out.write_all(&pdf)?;
        out.flush()?;
    } else {
        std::fs::write(&output, &pdf)?;
    }

    if let Some(code) = check_exit_code {
        std::process::exit(code);
    }

    Ok(())
}
