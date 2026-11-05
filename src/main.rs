#![allow(clippy::needless_update)]

#[macro_use]
extern crate cssparser;

#[macro_use]
extern crate html5ever;

#[cfg(test)]
extern crate quickcheck;
#[cfg(test)]
#[macro_use(quickcheck)]
extern crate quickcheck_macros;

use structopt::StructOpt;

use derive_more::From;

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

pub mod default_cell_ext;
pub mod display_list;
pub mod dot;
pub mod layout;
pub mod node_data_ref;
mod parser;
mod pdf;
pub mod simple_dom;
pub mod style;
mod traverse;
mod tree;

use crate::parser::generate_render_tree;
use crate::style::device::Device;
use style::properties::computed::PixelLength;

use log::info;

#[derive(StructOpt, Debug)]
#[structopt(name = "helium")]
struct Options {
    #[structopt(long = "no-compression")]
    no_filter: bool,
    #[structopt(long = "crop")]
    crop: bool,
    /// Write the computed layout as JSON to this file (use `-` for stdout)
    #[structopt(long = "dump-layout", parse(from_os_str))]
    dump_layout: Option<PathBuf>,
    /// Evaluate the `data-expected-*` attributes (web-platform-tests `checkLayout`) of the input
    /// document against the computed layout and exit with an error if any expectation fails.
    #[structopt(long = "check-layout")]
    check_layout: bool,
    /// The engine that lays out the boxes: `taffy` or `scheme`
    #[structopt(
        long = "layout-engine",
        env = "HELIUM_LAYOUT_ENGINE",
        default_value = "taffy"
    )]
    layout_engine: layout::LayoutEngine,
    /// The input HTML file to process
    #[structopt(parse(from_os_str))]
    input: PathBuf,
    /// Path where the output PDF file will be stored
    #[structopt(parse(from_os_str))]
    output: PathBuf,
}

#[derive(Debug, From)]
pub enum Error {
    Io(std::io::Error),
    Layout(layout::lscheme::RuntimeError),
}

fn main() -> Result<(), Error> {
    env_logger::init();

    let opt = Options::from_args();

    let file = fs::read(opt.input)?;
    let html = String::from_utf8_lossy(&file);

    let before_render_tree = Instant::now();
    let dom = generate_render_tree(&html);
    info!(
        "TIMED: generate_render_tree {:?}",
        before_render_tree.elapsed()
    );

    let device = if opt.crop {
        // dummy device. Final page size is determined by cropping
        Device {
            width: PixelLength(0.0),
            height: PixelLength(0.0),
        }
    } else {
        // a4 paper
        Device {
            width: PixelLength(793.7),
            height: PixelLength(1123.0),
        }
    };
    let before_layout = Instant::now();
    let mut layout_context = layout::layout(dom.clone(), device, opt.crop, opt.layout_engine)?;
    info!("TIMED: layout {:?}", before_layout.elapsed());

    // repl(&mut layout_context);

    if let Some(path) = &opt.dump_layout {
        let json = layout::dump::dump_layout(&dom, &layout_context);
        if path.as_os_str() == "-" {
            print!("{}", json);
        } else {
            fs::write(path, json)?;
        }
    }

    // the PDF is still written for failing checks so that it can be inspected
    let check_exit_code = if opt.check_layout {
        let report = layout::check::check_layout(&dom, &layout_context);
        for line in &report.unsupported {
            eprintln!("UNSUPPORTED {}", line);
        }
        for line in &report.failures {
            eprintln!("FAIL {}", line);
        }
        println!(
            "checked {} expectations, {} failed",
            report.checked,
            report.failures.len()
        );
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

    let display_list = display_list::generate_display_list(&mut layout_context);

    pdf::write_pdf(opt.output, display_list, opt.no_filter, opt.crop)?;

    if let Some(code) = check_exit_code {
        std::process::exit(code);
    }

    Ok(())
}
