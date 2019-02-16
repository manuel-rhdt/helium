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

use crate::parser::generate_render_tree;
pub use layout::flow::FlowRef;

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

    let dom = dbg!(generate_render_tree(&html));

    dom.contributed_flows().map(|flow| pdf::write_pdf(flow));

    Ok(())
}
