use afm;
use glob::glob;
use pom;
use quote::quote;
use proc_macro2::{Ident, Span};

use std::error::Error;
use std::{fs, env};
use std::path::Path;
use std::io::Write;

use inflector::cases::pascalcase::to_pascal_case;

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = env::var("OUT_DIR")?;
    let dest_path = Path::new(&out_dir).join("base_14_fonts.rs");
    let mut f = fs::File::create(&dest_path)?;
    
    let parser = afm::afm();

    let mut tokens = vec![];
    let mut font_names = vec![];
    println!("cargo:rerun-if-changed=resources/font/base_14_fonts");
    for entry in glob("resources/font/base_14_fonts/*.afm").expect("Could not find font metrics specifications for base 14 fonts.") {
        let metrics = fs::read_to_string(entry.expect("Could not read afm file"))?;
        let mut input = pom::DataInput::new(metrics.as_bytes());
        let metrics = parser.parse(&mut input)?;
        let font_name = metrics.font_name;
        let ascender = metrics.ascender;
        let descender = metrics.descender;

        let char_metrics = metrics.char_metrics.into_iter().map(|metric| {
            let name = metric.name;
            let bbox = [metric.bbox.xmin, metric.bbox.xmax, metric.bbox.ymin, metric.bbox.ymax];
            let character_code = metric.character_code;
            let wx = metric.wx;
            quote!(CharMetric {
                name: #name,
                bbox: [#(FontUnit(#bbox)),*],
                character_code: #character_code,
                advance_width: FontUnit(#wx),
            })
        });

        tokens.push(quote!(
            BuiltInFontMetrics {
                font_name: #font_name,
                ascender: FontUnit(#ascender),
                descender: FontUnit(#descender),
                char_metrics: &[#(#char_metrics),*],
            }
        ));

        font_names.push(font_name);
    }

    let font_names_case_corrected = font_names.iter().map(|s| Ident::new(&to_pascal_case(&s), Span::call_site()));
    let font_names_case_corrected2 = font_names_case_corrected.clone();

    let token_stream = quote!(
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub enum BuiltInFonts {
            #(#font_names_case_corrected),*
        }

        impl BuiltInFonts {
            pub fn metrics(self) -> BuiltInFontMetrics {
                match self {
                    #( BuiltInFonts::#font_names_case_corrected2 => #tokens ),*
                }
            }
        }
    );
    
    write!(f, "{}", token_stream)?;

    Ok(())
}