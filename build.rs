//! Builds the fonts that are embedded into the binary.
//!
//! The full Noto fonts in `resources/fonts/src` are far too large to ship (the CJK fonts alone
//! are tens of megabytes), so they are subset here with harfbuzz: the CJK fonts to the characters
//! of the legacy national standards (GB 2312, Big5 level 1, JIS X 0208, KS X 1001; see
//! `resources/fonts/cjk-charset.txt`) plus kana and punctuation, all other fonts only lose their
//! hinting. The result is written to `OUT_DIR` and listed in `bundled_fonts.rs`, which is
//! included by `src/layout/text/font.rs`.

use harfbuzz_rs::subset::{SubsetFlags, SubsetInput};
use harfbuzz_rs::Face;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Which characters of the source font to keep.
#[derive(Clone, Copy)]
enum Keep {
    /// Everything, only the hinting is removed.
    All,
    /// The characters for the CJK fonts: the legacy standards' charset and common blocks.
    Cjk,
}

struct Spec {
    /// File in `resources/fonts/src`.
    source: &'static str,
    /// Face index in a font collection.
    face: u32,
    keep: Keep,
}

const fn all(source: &'static str) -> Spec {
    Spec { source, face: 0, keep: Keep::All }
}

const fn cjk(source: &'static str, face: u32) -> Spec {
    Spec { source, face, keep: Keep::Cjk }
}

/// The first face of the Noto CJK collections (Japanese).
const JP: u32 = 0;

const FONTS: &[Spec] = &[
    // Arabic script: Naskh for serif text, Sans for the rest, Nastaliq for Urdu.
    all("NotoNaskhArabic-Regular.ttf"),
    all("NotoNaskhArabic-Bold.ttf"),
    all("NotoSansArabic-Regular.ttf"),
    all("NotoSansArabic-Bold.ttf"),
    all("NotoNastaliqUrdu-Regular.ttf"),
    all("NotoSansHebrew-Regular.ttf"),
    all("NotoSansHebrew-Bold.ttf"),
    all("NotoSerifHebrew-Regular.ttf"),
    all("NotoSerifHebrew-Bold.ttf"),
    all("NotoSansSyriac-Regular.ttf"),
    all("NotoSansThaana-Regular.ttf"),
    // Indic scripts
    all("NotoSansDevanagari-Regular.ttf"),
    all("NotoSansDevanagari-Bold.ttf"),
    all("NotoSansBengali-Regular.ttf"),
    all("NotoSansBengali-Bold.ttf"),
    all("NotoSansGurmukhi-Regular.ttf"),
    all("NotoSansGurmukhi-Bold.ttf"),
    all("NotoSansGujarati-Regular.ttf"),
    all("NotoSansGujarati-Bold.ttf"),
    all("NotoSansOriya-Regular.ttf"),
    all("NotoSansTamil-Regular.ttf"),
    all("NotoSansTamil-Bold.ttf"),
    all("NotoSansTelugu-Regular.ttf"),
    all("NotoSansTelugu-Bold.ttf"),
    all("NotoSansKannada-Regular.ttf"),
    all("NotoSansKannada-Bold.ttf"),
    all("NotoSansMalayalam-Regular.ttf"),
    all("NotoSansMalayalam-Bold.ttf"),
    all("NotoSansSinhala-Regular.ttf"),
    // South-East Asia
    all("NotoSansThai-Regular.ttf"),
    all("NotoSansThai-Bold.ttf"),
    all("NotoSansLao-Regular.ttf"),
    all("NotoSansKhmer-Regular.ttf"),
    all("NotoSansMyanmar-Regular.ttf"),
    // Others
    all("NotoSansGeorgian-Regular.ttf"),
    all("NotoSansArmenian-Regular.ttf"),
    all("NotoSansEthiopic-Regular.ttf"),
    // Symbols and mathematics
    all("NotoSansSymbols-Regular.ttf"),
    all("NotoSansSymbols2-Regular.ttf"),
    all("NotoSansMath-Regular.ttf"),
    // CJK
    // One face per style suffices: all faces of the collections have the same glyphs and the
    // `locl` feature picks the variants of the language (ja, ko, zh-Hans, zh-Hant), so this
    // face is a stand-in for all of the regional Noto CJK families.
    cjk("NotoSansCJK-Regular.ttc", JP),
    cjk("NotoSansCJK-Bold.ttc", JP),
];

/// Blocks that are kept in the CJK fonts in addition to the characters of the charset file.
const CJK_BLOCKS: &[(u32, u32)] = &[
    (0x20, 0x7E),     // ASCII
    (0xA0, 0x17F),    // Latin-1 and Latin Extended-A
    (0x2000, 0x206F), // general punctuation
    (0x2190, 0x21FF), // arrows
    (0x2460, 0x24FF), // enclosed alphanumerics
    (0x25A0, 0x25FF), // geometric shapes
    (0x2E80, 0x2FDF), // radicals
    (0x3000, 0x30FF), // CJK punctuation, hiragana, katakana
    (0x3100, 0x312F), // bopomofo
    (0x3130, 0x318F), // hangul compatibility jamo
    (0x31F0, 0x31FF), // katakana extension
    (0x3200, 0x33FF), // enclosed CJK, compatibility
    (0x1100, 0x11FF), // hangul jamo
    (0xFE30, 0xFE4F), // CJK compatibility forms
    (0xFF00, 0xFFEF), // halfwidth and fullwidth forms
];

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let src = root.join("resources/fonts/src");
    let charset_path = root.join("resources/fonts/cjk-charset.txt");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("fonts");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).unwrap();

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", charset_path.display());

    let charset: Vec<char> = fs::read_to_string(&charset_path).unwrap().chars().filter(|c| !c.is_whitespace()).collect();

    let mut list = String::from("pub(crate) const EMBEDDED_FONTS: &[&[u8]] = &[\n");
    for spec in FONTS {
        let stem = Path::new(spec.source).file_stem().unwrap().to_str().unwrap();
        let name = format!("{stem}-{}.otf", spec.face);
        let target = out.join(&name);
        let data = fs::read(src.join(spec.source)).unwrap_or_else(|e| panic!("{}: {e}", spec.source));
        let subset = subset(&data, spec, &charset).unwrap_or_else(|| panic!("cannot subset {}", spec.source));
        fs::write(&target, subset).unwrap();
        writeln!(list, "    include_bytes!({:?}),", target.display().to_string()).unwrap();
    }
    list.push_str("];\n");
    fs::write(out.join("bundled_fonts.rs"), list).unwrap();
}

fn subset(data: &[u8], spec: &Spec, charset: &[char]) -> Option<Vec<u8>> {
    let face = Face::from_bytes(data, spec.face);
    let mut input = SubsetInput::new().ok()?;
    match spec.keep {
        Keep::All => input.keep_everything(),
        Keep::Cjk => {
            let mut unicodes = input.unicode_set_mut();
            for &(first, last) in CJK_BLOCKS {
                unicodes.insert_range(char::from_u32(first).unwrap()..=char::from_u32(last).unwrap());
            }
            for &c in charset {
                unicodes.insert(c);
            }
        }
    }
    // Keep the layout features: they shape the scripts and select the regional glyph variants.
    let mut flags = SubsetFlags::NO_HINTING | SubsetFlags::NOTDEF_OUTLINE;
    if matches!(spec.keep, Keep::All) {
        flags |= SubsetFlags::PASSTHROUGH_UNRECOGNIZED;
    }
    input.set_flags(flags);
    if face.table_with_tag(b"fvar").is_some() {
        input.pin_all_axes_to_default(&face).ok()?;
    }
    let subset = face.subset(&input).ok()?;
    Some(subset.face_data().as_ref().to_vec())
}
