use structopt::StructOpt;

use derive_more::From;

use std::fs;
use std::path::PathBuf;

pub mod default_cell_ext;
pub mod display_list;
pub mod dot;
pub mod layout;
pub mod node_data_ref;
mod parser;
mod pdf;
pub mod style;
mod traverse;
mod tree;

use crate::parser::generate_render_tree;
use crate::style::device::Device;
use display_list::DisplayListBuilder;

use specs::RunNow;

#[derive(StructOpt, Debug)]
#[structopt(name = "helium")]
struct Options {
    #[structopt(parse(from_os_str))]
    input: PathBuf,
}

#[derive(Debug, From)]
pub enum Error {
    PdfWriteError(printpdf::Error),
    Io(std::io::Error),
}

fn main() -> Result<(), Error> {
    simple_logger::init_with_level(log::Level::Info).unwrap();

    let opt = Options::from_args();

    let file = fs::read(opt.input)?;
    let html = String::from_utf8_lossy(&file);

    let dom = generate_render_tree(&html);

    // a4 paper
    let device = Device {
        width: 793.7,
        height: 1123.0,
    };

    let layout_context = layout::layout(dom.clone(), device);

    let mut dotfile = fs::File::create("graph.dot")?;
    dom.serialize_dot_graph(&mut dotfile)?;

    let mut display_list_builder = DisplayListBuilder::default();
    display_list_builder.run_now(&layout_context.res);
    let display_list = dbg!(display_list_builder.build());

    pdf::write_pdf(display_list, device)?;

    Ok(())
}
