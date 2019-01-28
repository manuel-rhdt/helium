use structopt::StructOpt;

use derive_more::From;

use std::fs;
use std::path::PathBuf;

mod flow;
pub mod layout;
mod parser;
mod pdf;
pub mod style;

pub use crate::flow::FlowTree;
use crate::parser::generate_render_tree;

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
    let opt = Options::from_args();

    let file = fs::read(opt.input)?;
    let html = String::from_utf8_lossy(&file);

    let render_tree = generate_render_tree(&html);

    pdf::write_pdf(render_tree)?;

    Ok(())
}
