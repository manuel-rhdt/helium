//! Builds the fonts that are embedded into the binary.
//!
//! The full Noto fonts in `resources/fonts/src` are far too large to ship (the CJK fonts alone
//! are tens of megabytes), so they are subset here with skera (a port of harfbuzz's subsetter): the CJK fonts to the characters
//! of the legacy national standards (GB 2312, Big5 level 1, JIS X 0208, KS X 1001; see
//! `resources/fonts/cjk-charset.txt`) plus kana and punctuation, all other fonts only lose their
//! hinting. The same goes for the Latin, Greek, Cyrillic and math fonts of `resources/fonts`.
//!
//! Every subset is deflated and written to `OUT_DIR`; `bundled_fonts.rs` lists the compressed
//! fonts together with the metadata that `fontdb` would read from them (families, weight, style),
//! so that registering the fonts at start-up does not have to inflate any of them. The file is
//! included by `src/layout/text/font.rs`, which inflates a font when it is first used.

use flate2::write::DeflateEncoder;
use read_fonts::collections::IntSet;
use read_fonts::types::{GlyphId, NameId, Tag};
use read_fonts::FontRef;
use skera::{Plan, SubsetFlags, subset_font};
use flate2::Compression;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
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
    /// File in `resources/fonts` (or in `resources/fonts/src` for the Noto fonts).
    source: &'static str,
    /// The name of the constant with the index of this font in `BUNDLED_FONTS`, if any.
    constant: Option<&'static str>,
    /// Face index in a font collection.
    face: u32,
    keep: Keep,
}

const fn all(source: &'static str) -> Spec {
    Spec { source, constant: None, face: 0, keep: Keep::All }
}

const fn named(source: &'static str, constant: &'static str) -> Spec {
    Spec { source, constant: Some(constant), face: 0, keep: Keep::All }
}

const fn cjk(source: &'static str, face: u32) -> Spec {
    Spec { source, constant: None, face, keep: Keep::Cjk }
}

/// The first face of the Noto CJK collections (Japanese).
const JP: u32 = 0;

const FONTS: &[Spec] = &[
    // Free, metric-compatible stand-ins for Helvetica, Times and Courier (TeX Gyre Heros,
    // Termes and Cursor) and the OpenType math fonts. The order is the order in which the fonts
    // are registered: the fonts registered first win ties in font matching.
    all("texgyreheros-regular.otf"),
    all("texgyreheros-bold.otf"),
    all("texgyreheros-italic.otf"),
    all("texgyreheros-bolditalic.otf"),
    all("texgyretermes-regular.otf"),
    all("texgyretermes-bold.otf"),
    all("texgyretermes-italic.otf"),
    all("texgyretermes-bolditalic.otf"),
    all("texgyrecursor-regular.otf"),
    all("texgyrecursor-bold.otf"),
    all("texgyrecursor-italic.otf"),
    all("texgyrecursor-bolditalic.otf"),
    all("latinmodern-math.otf"),
    all("STIXTwoMath-Regular.ttf"),
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
    // Colour emoji, painted from the `COLR` table (the SVG table is stripped from the source).
    all("NotoColorEmoji-COLRv1.ttf"),
    // CJK
    // One face per style suffices: all faces of the collections have the same glyphs and the
    // `locl` feature picks the variants of the language (ja, ko, zh-Hans, zh-Hant), so this
    // face is a stand-in for all of the regional Noto CJK families.
    cjk("NotoSansCJK-Regular.ttc", JP),
    cjk("NotoSansCJK-Bold.ttc", JP),
    // The fallback for everything the other fonts do not cover.
    named("DejaVuSans.ttf", "DEJAVU_REGULAR"),
    named("DejaVuSans-Bold.ttf", "DEJAVU_BOLD"),
];

/// Fonts whose deflated size is more than this fraction of their size are not compressed.
const MAX_RATIO: f64 = 0.75;

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
    let fonts = root.join("resources/fonts");
    let charset_path = fonts.join("cjk-charset.txt");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("fonts");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).unwrap();

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", fonts.display());
    println!("cargo:rerun-if-changed={}", charset_path.display());

    let charset: Vec<char> = fs::read_to_string(&charset_path).unwrap().chars().filter(|c| !c.is_whitespace()).collect();

    let mut list = String::from("pub(crate) static BUNDLED_FONTS: [EmbeddedFont; FONTS] = [\n");
    let mut constants = String::new();
    for (i, spec) in FONTS.iter().enumerate() {
        let stem = Path::new(spec.source).file_stem().unwrap().to_str().unwrap();
        let target = out.join(format!("{stem}-{}.z", spec.face));
        let path = [fonts.join(spec.source), fonts.join("src").join(spec.source)]
            .into_iter()
            .find(|path| path.exists())
            .unwrap_or_else(|| panic!("{}: not found", spec.source));
        let data = fs::read(path).unwrap();
        let subset = subset(&data, spec, &charset).unwrap_or_else(|| panic!("cannot subset {}", spec.source));
        let info = face_info(&subset).unwrap_or_else(|| panic!("cannot read {}", spec.source));
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&subset).unwrap();
        let deflated = encoder.finish().unwrap();
        // Inflating takes time (about 25 ms for 5 MB) that is only worth spending where it saves
        // much: fonts that hardly shrink (the CJK fonts) are stored as they are.
        let deflate = (deflated.len() as f64) < MAX_RATIO * subset.len() as f64;
        fs::write(&target, if deflate { &deflated } else { &subset }).unwrap();
        writeln!(
            list,
            "    EmbeddedFont::new(include_bytes!({:?}), {}, {deflate}, {info}),",
            target.display().to_string(),
            subset.len()
        )
        .unwrap();
        if let Some(name) = spec.constant {
            writeln!(constants, "pub(crate) const {name}: usize = {i};").unwrap();
        }
    }
    list.push_str("];\n");
    writeln!(list, "const FONTS: usize = {};", FONTS.len()).unwrap();
    list.push_str(&constants);
    fs::write(out.join("bundled_fonts.rs"), list).unwrap();
}

/// The metadata of a font as `fontdb` reads it, as the source of an `EmbeddedFaceInfo`.
fn face_info(data: &[u8]) -> Option<String> {
    let mut db = fontdb::Database::new();
    db.load_font_data(data.to_vec());
    let mut faces = db.faces();
    let face = faces.next()?;
    if faces.next().is_some() {
        return None;
    }
    let families: Vec<String> =
        face.families.iter().map(|(name, language)| format!("({name:?}, fontdb::Language::{language:?})")).collect();
    Some(format!(
        "EmbeddedFaceInfo {{ families: &[{}], post_script_name: {:?}, style: fontdb::Style::{:?}, weight: {}, stretch: fontdb::Stretch::{:?}, monospaced: {} }}",
        families.join(", "),
        face.post_script_name,
        face.style,
        face.weight.0,
        face.stretch,
        face.monospaced,
    ))
}

/// The layout features that the CJK fonts keep: the ones that harfbuzz's subsetter keeps by
/// default, as far as the fonts have them. The glyph closure of a feature adds glyphs, so
/// keeping all features makes the fonts bigger and numbers their glyphs differently.
const CJK_FEATURES: &[&[u8; 4]] = &[
    b"calt", b"ccmp", b"halt", b"kern", b"liga", b"ljmo", b"locl", b"mark", b"palt", b"tjmo", b"vert", b"vhal",
    b"vjmo", b"vkrn", b"vpal", b"vrt2",
];

fn subset(data: &[u8], spec: &Spec, charset: &[char]) -> Option<Vec<u8>> {
    let font = FontRef::from_index(data, spec.face).ok()?;
    // skera does not instantiate variable fonts: their variation tables would be kept, and the
    // font would be bigger and still variable. The bundled fonts are static; a variable one
    // has to be pinned to its default instance first.
    assert!(
        font.table_data(Tag::new(b"fvar")).is_none(),
        "{} is a variable font; it has to be instantiated before it is bundled",
        spec.source
    );
    let mut gids = IntSet::<GlyphId>::empty();
    let mut unicodes = IntSet::<u32>::empty();
    let mut features = IntSet::<Tag>::all();
    let mut names = IntSet::<NameId>::empty();
    let mut languages = IntSet::<u16>::empty();
    // Hinting is removed, and the layout features are kept: they shape the scripts and select the
    // regional glyph variants.
    let mut flags = SubsetFlags::SUBSET_FLAGS_NO_HINTING | SubsetFlags::SUBSET_FLAGS_NOTDEF_OUTLINE;
    match spec.keep {
        Keep::All => {
            gids = IntSet::all();
            unicodes = IntSet::all();
            names = IntSet::all();
            languages = IntSet::all();
            flags |= SubsetFlags::SUBSET_FLAGS_PASSTHROUGH_UNRECOGNIZED;
        }
        Keep::Cjk => {
            for &(first, last) in CJK_BLOCKS {
                unicodes.insert_range(first..=last);
            }
            for &c in charset {
                unicodes.insert(c as u32);
            }
            features = CJK_FEATURES.iter().map(|tag| Tag::new(tag)).collect();
            // the copyright, family, subfamily, unique, full and PostScript names, in English
            for id in 0..=6 {
                names.insert(NameId::new(id));
            }
            languages.insert(0x0409);
        }
    }
    let plan = Plan::new(&gids, &unicodes, &font, flags, &IntSet::empty(), &IntSet::all(), &features, &names, &languages);
    subset_font(&font, &plan).ok()
}
