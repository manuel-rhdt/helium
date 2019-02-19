use structopt::StructOpt;

use derive_more::From;

use std::fs;
use std::path::PathBuf;

pub mod layout;
mod parser;
mod pdf;
pub mod style;
mod traverse;
mod tree;
pub mod default_cell_ext;
pub mod display_list;

use crate::parser::generate_render_tree;
pub use layout::flow::FlowRef;
use traverse::Traverse;

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
    simple_logger::init().unwrap();

    let opt = Options::from_args();

    let file = fs::read(opt.input)?;
    let html = String::from_utf8_lossy(&file);

    let dom = generate_render_tree(&html);

    let mut layouter = layout::inline_size::LayoutSizeComputation {
        inline_context: layout::inline_size::InlineSizeContext {
            page_size: 400.0
        },
        block_context: layout::block_size::BlockSizeContext {}
    };

    let root_flow = dom.contributed_flows().expect("The root dom node must have a flow.");

    root_flow.traverse(&mut layouter);

    dom.contributed_flows().map(|flow| pdf::write_pdf(flow));

    Ok(())
}
