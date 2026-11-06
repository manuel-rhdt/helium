//! Static data for font fallback (`fallback`): the font names and families that it knows.
//!
//! Keeping the tables here leaves `fallback` with the logic of choosing among them.

use super::fallback::Class;
use unicode_script::Script;

/// Families with a broad character coverage that are tried, in this order, for characters that
/// none of the families of `font-family` cover. Families that are not installed are skipped.
pub(super) const FALLBACK_FAMILIES: &[&str] = &[
    // Linux
    "DejaVu Sans",
    "Noto Sans",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    // the name of the font built into Helium
    "Noto Sans Symbols2",
    "Noto Sans Math",
    "DejaVu Serif",
    "FreeSerif",
    "FreeSans",
    "Symbola",
    // macOS
    "Apple Symbols",
    "Arial Unicode MS",
    "STIX Two Text",
    // Windows
    "Segoe UI Symbol",
    "Cambria Math",
];

/// General-purpose serif families that are tried for text in a serif font before the
/// (mostly sans-serif) `FALLBACK_FAMILIES`, so that e.g. a dash missing from a font comes from a
/// serif font as well.
pub(super) const SERIF_FAMILIES: &[&str] = &[
    "Noto Serif",
    "DejaVu Serif",
    "Liberation Serif",
    "Times New Roman",
    "Times",
];

/// A font whose `OS/2` metadata gives the wrong class (or none), recorded for that one font.
///
/// `fallback::classify` trusts the metadata (PANOSE and `sFamilyClass`) unless the PostScript
/// name of the font is listed here. Add an entry only for a specific font file that was found to
/// be classified wrongly, with the class that its metadata should have said, and say in a
/// comment what is wrong with the metadata. The metadata describes Latin typefaces only, so it
/// often fails for other scripts, and many fonts leave it zero.
pub(super) struct ClassCorrection {
    /// The exact PostScript name (`Font::postscript_name`), which names one face of a family.
    pub(super) postscript_name: &'static str,
    /// The class that the metadata of the font should have given.
    pub(super) class: Class,
}

pub(super) const CLASS_CORRECTIONS: &[ClassCorrection] = &[
    // TeX Gyre Termes: no OS/2 classification
    ClassCorrection { postscript_name: "TeXGyreTermes-Regular", class: Class::Serif },
    ClassCorrection { postscript_name: "TeXGyreTermes-Bold", class: Class::Serif },
    ClassCorrection { postscript_name: "TeXGyreTermes-Italic", class: Class::Serif },
    ClassCorrection { postscript_name: "TeXGyreTermes-BoldItalic", class: Class::Serif },
    // TeX Gyre Cursor: no OS/2 classification
    ClassCorrection { postscript_name: "TeXGyreCursor-Regular", class: Class::Mono },
    ClassCorrection { postscript_name: "TeXGyreCursor-Bold", class: Class::Mono },
    ClassCorrection { postscript_name: "TeXGyreCursor-Italic", class: Class::Mono },
    ClassCorrection { postscript_name: "TeXGyreCursor-BoldItalic", class: Class::Mono },
    // Noto Naskh Arabic: PANOSE says normal sans, but it is a Naskh design, which the Latin
    // classification does not describe
    ClassCorrection { postscript_name: "NotoNaskhArabic-Regular", class: Class::Serif },
    ClassCorrection { postscript_name: "NotoNaskhArabic-Medium", class: Class::Serif },
    ClassCorrection { postscript_name: "NotoNaskhArabic-SemiBold", class: Class::Serif },
    ClassCorrection { postscript_name: "NotoNaskhArabic-Bold", class: Class::Serif },
    ClassCorrection { postscript_name: "NotoNaskhArabicUI-Regular", class: Class::Serif },
    ClassCorrection { postscript_name: "NotoNaskhArabicUI-Medium", class: Class::Serif },
    ClassCorrection { postscript_name: "NotoNaskhArabicUI-SemiBold", class: Class::Serif },
    ClassCorrection { postscript_name: "NotoNaskhArabicUI-Bold", class: Class::Serif },
    // Noto Serif Khojki: sFamilyClass says sans (regular) or is empty (bold), but it is a serif
    // design
    ClassCorrection { postscript_name: "NotoSerifKhojki-Regular", class: Class::Serif },
    ClassCorrection { postscript_name: "NotoSerifKhojki-Bold", class: Class::Serif },
    // Noto Sans Medefaidrin: PANOSE says serif, but it is a sans-serif design
    ClassCorrection { postscript_name: "NotoSansMedefaidrin-Regular", class: Class::Sans },
    ClassCorrection { postscript_name: "NotoSansMedefaidrin-Medium", class: Class::Sans },
    ClassCorrection { postscript_name: "NotoSansMedefaidrin-SemiBold", class: Class::Sans },
    ClassCorrection { postscript_name: "NotoSansMedefaidrin-Bold", class: Class::Sans },
];

/// The CJK fonts of one regional variant, for serif, sans-serif and monospace text.
pub(super) struct CjkVariant {
    pub(super) serif: &'static [&'static str],
    pub(super) sans: &'static [&'static str],
    pub(super) mono: &'static [&'static str],
}

impl CjkVariant {
    /// The fonts of the variant for text of the design `class`.
    pub(super) fn fonts(&self, class: Class) -> &'static [&'static str] {
        match class {
            Class::Serif => self.serif,
            Class::Sans => self.sans,
            Class::Mono => self.mono,
        }
    }
}

pub(super) const JP: CjkVariant = CjkVariant {
    serif: &["Noto Serif CJK JP", "Source Han Serif JP", "Hiragino Mincho ProN", "Yu Mincho", "MS Mincho"],
    sans: &["Noto Sans CJK JP", "Source Han Sans JP", "Hiragino Sans", "Yu Gothic", "Meiryo"],
    mono: &["Noto Sans Mono CJK JP", "Source Han Code JP"],
};
pub(super) const KR: CjkVariant = CjkVariant {
    serif: &["Noto Serif CJK KR", "Source Han Serif K", "AppleMyungjo", "Batang"],
    sans: &["Noto Sans CJK KR", "Source Han Sans K", "Apple SD Gothic Neo", "Malgun Gothic"],
    mono: &["Noto Sans Mono CJK KR"],
};
pub(super) const SC: CjkVariant = CjkVariant {
    serif: &["Noto Serif CJK SC", "Source Han Serif SC", "Songti SC", "SimSun"],
    sans: &["Noto Sans CJK SC", "Source Han Sans SC", "PingFang SC", "Microsoft YaHei"],
    mono: &["Noto Sans Mono CJK SC"],
};
pub(super) const TC: CjkVariant = CjkVariant {
    serif: &["Noto Serif CJK TC", "Source Han Serif TC", "Songti TC", "PMingLiU"],
    sans: &["Noto Sans CJK TC", "Source Han Sans TC", "PingFang TC", "Microsoft JhengHei"],
    mono: &["Noto Sans Mono CJK TC"],
};
pub(super) const HK: CjkVariant = CjkVariant {
    serif: &["Noto Serif CJK HK", "Source Han Serif TC"],
    sans: &["Noto Sans CJK HK", "Source Han Sans HC"],
    mono: &["Noto Sans Mono CJK HK"],
};

/// Families for scripts that have no regional variants (those of CJK, `CjkVariant`) and no
/// languages with fonts of their own (Arabic, below): `(script, serif families, sans-serif
/// families)`. The sans-serif list is also used for monospace text.
pub(super) const SCRIPT_FAMILIES: &[(Script, &[&str], &[&str])] = &[
    (Script::Hebrew, &["Noto Serif Hebrew", "Frank Ruehl CLM", "David CLM", "Noto Sans Hebrew", "Times New Roman"], &["Noto Sans Hebrew", "Noto Serif Hebrew", "Arial Hebrew", "Arial"]),
    (Script::Syriac, &["Noto Sans Syriac"], &["Noto Sans Syriac"]),
    (Script::Thaana, &["Noto Sans Thaana"], &["Noto Sans Thaana"]),
    (Script::Devanagari, &["Noto Serif Devanagari", "Noto Sans Devanagari", "Kohinoor Devanagari", "Mangal"], &["Noto Sans Devanagari", "Noto Serif Devanagari", "Kohinoor Devanagari", "Nirmala UI"]),
    (Script::Bengali, &["Noto Serif Bengali", "Noto Sans Bengali"], &["Noto Sans Bengali", "Noto Serif Bengali", "Nirmala UI"]),
    (Script::Gurmukhi, &["Noto Serif Gurmukhi", "Noto Sans Gurmukhi"], &["Noto Sans Gurmukhi", "Noto Serif Gurmukhi", "Nirmala UI"]),
    (Script::Gujarati, &["Noto Serif Gujarati", "Noto Sans Gujarati"], &["Noto Sans Gujarati", "Noto Serif Gujarati", "Nirmala UI"]),
    (Script::Tamil, &["Noto Serif Tamil", "Noto Sans Tamil"], &["Noto Sans Tamil", "Noto Serif Tamil", "Nirmala UI"]),
    (Script::Telugu, &["Noto Serif Telugu", "Noto Sans Telugu"], &["Noto Sans Telugu", "Noto Serif Telugu", "Nirmala UI"]),
    (Script::Kannada, &["Noto Serif Kannada", "Noto Sans Kannada"], &["Noto Sans Kannada", "Noto Serif Kannada", "Nirmala UI"]),
    (Script::Malayalam, &["Noto Serif Malayalam", "Noto Sans Malayalam"], &["Noto Sans Malayalam", "Noto Serif Malayalam", "Nirmala UI"]),
    (Script::Oriya, &["Noto Serif Oriya", "Noto Sans Oriya"], &["Noto Sans Oriya", "Noto Serif Oriya"]),
    (Script::Sinhala, &["Noto Serif Sinhala", "Noto Sans Sinhala"], &["Noto Sans Sinhala", "Noto Serif Sinhala"]),
    (Script::Thai, &["Noto Serif Thai", "Noto Sans Thai", "Thonburi", "Tahoma"], &["Noto Sans Thai", "Noto Serif Thai", "Thonburi", "Tahoma"]),
    (Script::Lao, &["Noto Serif Lao", "Noto Sans Lao"], &["Noto Sans Lao", "Noto Serif Lao"]),
    (Script::Tibetan, &["Noto Serif Tibetan"], &["Noto Serif Tibetan"]),
    (Script::Myanmar, &["Noto Serif Myanmar", "Noto Sans Myanmar"], &["Noto Sans Myanmar", "Noto Serif Myanmar"]),
    (Script::Khmer, &["Noto Serif Khmer", "Noto Sans Khmer"], &["Noto Sans Khmer", "Noto Serif Khmer"]),
    (Script::Armenian, &["Noto Serif Armenian", "Noto Sans Armenian"], &["Noto Sans Armenian", "Noto Serif Armenian"]),
    (Script::Georgian, &["Noto Serif Georgian", "Noto Sans Georgian"], &["Noto Sans Georgian", "Noto Serif Georgian"]),
    (Script::Ethiopic, &["Noto Serif Ethiopic", "Noto Sans Ethiopic"], &["Noto Sans Ethiopic", "Noto Serif Ethiopic"]),
];

/// The families for Arabic script as `(serif, sans-serif)` lists, in general and for the
/// languages that prefer another style (Nastaliq for Urdu).
pub(super) const ARABIC: (&[&str], &[&str]) = (
    &[
        "Noto Naskh Arabic",
        "Amiri",
        "Scheherazade New",
        "Noto Sans Arabic",
        "Geeza Pro",
        "Traditional Arabic",
        "Times New Roman",
    ],
    &["Noto Sans Arabic", "Noto Naskh Arabic", "Geeza Pro", "Segoe UI", "Arial"],
);
pub(super) const URDU: (&[&str], &[&str]) = (
    &["Noto Nastaliq Urdu", "Jameel Noori Nastaleeq", "Urdu Typesetting"],
    &["Noto Nastaliq Urdu", "Noto Sans Arabic"],
);
pub(super) const PERSIAN: (&[&str], &[&str]) = (&["Noto Naskh Arabic"], &["Vazirmatn", "Noto Sans Arabic"]);
pub(super) const ARABIC_NONE: (&[&str], &[&str]) = (&[], &[]);
