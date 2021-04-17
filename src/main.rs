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
    #[structopt(parse(from_os_str))]
    input: PathBuf,
}

#[derive(Debug, From)]
pub enum Error {
    Io(std::io::Error),
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
    let mut layout_context = layout::layout(dom.clone(), device, opt.crop);
    info!("TIMED: layout {:?}", before_layout.elapsed());

    let display_list = display_list::generate_display_list(&mut layout_context);

    pdf::write_pdf(display_list, opt.no_filter, opt.crop)?;

    Ok(())
}
