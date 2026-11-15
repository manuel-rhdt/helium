# Scripts (two A2 pages)

`index.html` is a print of 123 writing systems, each set in a real text, ordered by the date of
each script's earliest use and tinted by it (umber for the oldest, teal for the newest). Every
specimen has only the name of its script, small and vertical, next to it.

The tiles are laid out as a masonry: a flex container with `flex-direction: column; flex-wrap:
wrap; justify-content: space-between` packs tiles of their natural height into six columns
per page. Han and Japanese (`vertical-rl`) and Mongolian (`vertical-lr`) are set top to bottom
with `writing-mode`.

## Texts

- The opening of the **Universal Declaration of Human Rights** (Article 1 and the start of
  Article 2 for most; a few translations in the corpus begin at Article 2) for 41 scripts, from
  the MIT-licensed `udhr` npm package (the [UDHR in Unicode](https://www.unicode.org/udhr/)).
- **Sample sentences** for 36 further scripts (Balinese, Batak, N'Ko, Osage, Warang Citi, Hanifi
  Rohingya, Toto, Nag Mundari and others), taken from a language-sample file supplied for this
  print.
- **Samples for the historic scripts** from a second research file: Wikipedia candidate strings
  (checked only for script membership) for Glagolitic, Coptic (the Bohairic Lord's Prayer),
  Lycian, Lydian, Carian, Old Italic, Avestan, Pahlavi, Manichaean, Sogdian, Mandaic, Kharoshthi,
  Old Turkic, Old Hungarian, Old Permic, Elbasan, Newa, Sharada, Modi, Kaithi, Tirhuta,
  Khudawadi, Dogra and others; real inscriptions for Cypriot (IG XV 1) and Linear A (Haghia
  Triada tablet HT 1); and the sign sequence of the Phaistos disc (side A, in the conventional
  reading order). Several are only a word or a short phrase.
- **Inscriptions and words**, transcribed from published editions: an Akkadian phrase in
  cuneiform (*šum-ma a-wi-lum*, the first words of the Code of Hammurabi), the cartouche of
  Ramesses, Cretan place names in Linear B (*ko-no-so*, *pa-i-to*, *a-mi-ni-so*, *tu-ri-so*), the
  Ugaritic abecedary, the opening of the Ahiram sarcophagus in Phoenician, the first words of
  Darius's Behistun inscription in Old Persian, the Ashoka edicts in Brahmi, the Long Count date
  7.16.3.2.13 of Chiapa de Corzo Stela 2 in Mayan numerals, *maqi mucoi* in Ogham, the Gallehus
  horn in Elder Futhark, the Lord's Prayer of Wulfila in Gothic, and a line of the UDHR in
  Braille. Maya glyph blocks are not encoded in Unicode, so only the numerals can be set.

I could not verify the text of the scripts the author cannot read. The dates used for the order
are from memory (Toto, Tangsa and Nag Mundari are guesses).

## Fonts

Helium has no `@font-face` and uses the fonts installed on the system, so the document asks for
them with `<meta name="gallery-fonts" content="system">`. Noto Serif for the Indic, Southeast
Asian, Caucasian and CJK scripts (Noto Serif CJK for Han, Japanese and Hangul), Amiri for
Arabic, Gulzar for Nastaliq, Abyssinica SIL for Ethiopic, Gentium Book Plus for Latin, Greek and
Cyrillic (one design for all three, and for the punctuation of the other scripts), and Noto Sans
for the scripts that have no serif design. All are SIL OFL licensed.

`fetch-fonts.sh` downloads them (about 115 files, 33 MB, mostly the three CJK serifs) into
`~/.fonts/helium-poster`. The Gallery workflow runs it, caches the result and renders the
poster like the other documents, so there is no PDF in the repository.

## Render it locally

```
gallery/poster/fetch-fonts.sh
cargo run --release -- gallery/poster/index.html poster.pdf
```

Line breaking of Thai, Khmer, Lao and Burmese needs Helium 0.2.1 or later, and the vertical text
needs `writing-mode`.
