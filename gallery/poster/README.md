# Scripts (A2 print)

`index.html` is a two-page A2 print of 123 writing systems, each set in a real text. Next to every specimen
stands the name of its script, small and vertical, and nothing else.

- **The opening of the Universal Declaration of Human Rights** (Article 1 and the start of Article 2; a few translations in the corpus begin at Article 2) for 41 scripts that have a
  translation in the [UDHR in Unicode](https://www.unicode.org/udhr/) corpus (from Latin and Greek
  to Tibetan, Cherokee, Vai, Adlam, Javanese and Grantha). The texts come from the MIT-licensed `udhr`
  npm package. Each text is cut after as many lines as the tile holds. Tiles are ordered from page 1 to page 2. Mongolian (`vertical-lr`), Han and Japanese
  (`vertical-rl`) are set top to bottom with `writing-mode`.
- **Sample sentences** for 36 further scripts (Balinese, Batak, N'Ko, Osage, Warang Citi, Hanifi Rohingya, Toto, Nag Mundari and others), taken from a language-sample file supplied for this print. They are short translations in the style of the UDHR.
- **Samples for the historic scripts** from a second research file supplied for this print: Wikipedia candidate strings (checked only for script membership) for Glagolitic, Coptic (the Bohairic Lord's Prayer), Lycian, Lydian, Carian, Old Italic, Avestan, Pahlavi, Manichaean, Sogdian, Mandaic, Kharoshthi, Old Turkic, Old Hungarian, Old Permic, Elbasan, Newa, Sharada, Modi, Kaithi, Tirhuta, Khudawadi, Dogra and others, real inscriptions for Cypriot (IG XV 1) and Linear A (Haghia Triada tablet HT 1), and the sign sequence of the Phaistos disc (side A, with the conventional reading order). Several are only a word or a short phrase.
- **Inscriptions and words** for the historic scripts, transcribed from published editions:
  an Akkadian phrase in cuneiform (*šum-ma a-wi-lum*, the first words of the Code of Hammurabi),
  the cartouche of Ramesses, Cretan place names in Linear B (*ko-no-so*, *pa-i-to*, *a-mi-ni-so*,
  *tu-ri-so*), the Ugaritic abecedary, the opening of the Ahiram sarcophagus in Phoenician, the
  first words of Darius's Behistun inscription in Old Persian, the Ashoka edicts in Brahmi, the Long
  Count date 7.16.3.2.13 of Chiapa de Corzo Stela 2 in Mayan numerals, *maqi mucoi* in Ogham, the
  Gallehus horn in Elder Futhark, the Lord's Prayer of Wulfila in Gothic, and a line of the UDHR
  in Braille. Maya glyph blocks are not encoded in Unicode, so only the numerals can be set as text.

The specimens are ordered by the date of each script's earliest use, from the top left to the bottom
right. Nothing on the page says so.

`scripts-poster.pdf` is the result.

## Render it

Helium has no `@font-face` yet and uses the fonts installed on the system, so the extra Noto fonts
have to be installed first. Line breaking of Thai, Khmer, Lao and Burmese needs Helium 0.2.1 or later, and the vertical Han and Japanese text needs the `writing-mode` support (branch `claude/writing-mode`).

```
gallery/poster/fetch-fonts.sh            # downloads about 110 fonts into ~/.fonts/helium-poster
cargo run --release -- gallery/poster/index.html gallery/poster/scripts-poster.pdf
```

Fonts: Noto Serif for the Indic, Southeast Asian, Caucasian and CJK scripts (Noto Serif CJK for Han, Japanese and Hangul), Amiri for Arabic, Gulzar for Nastaliq, Abyssinica SIL for Ethiopic, TeX Gyre Termes for Latin, Greek and Cyrillic, and Noto Sans for the scripts that have no serif design (all SIL OFL or GUST licensed).

The print is not listed in `gallery/order.txt` because the gallery build only uses the fonts built
into Helium, and these fonts are not.

## Masonry variant

`masonry.html` sets the same specimens as tiles of their natural height, packed into columns with
a flex container (`flex-direction: column; flex-wrap: wrap; justify-content: space-between`), so
that there is no empty space under the short ones. There are six columns on each page, so the
tiles are wider and show more text than in the grid. The order runs down the columns, from the left.
The page break between the two pages is chosen from the measured heights of the tiles.
The text of each specimen and its label are tinted by the date of the script, from umber for the
oldest through brick, plum and indigo to teal for the newest, darkened towards the ink so that
the text stays as dark as before. `scripts-poster-masonry.pdf` is the result.
