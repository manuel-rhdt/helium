# Scripts (A2 print)

`index.html` is a two-page A2 print of 53 writing systems, each set in a real text. Next to every specimen
stands the name of its script, small and vertical, and nothing else.

- **The opening of the Universal Declaration of Human Rights** (Article 1 and the start of Article 2; a few translations in the corpus begin at Article 2) for the 41 scripts that have a
  translation in the [UDHR in Unicode](https://www.unicode.org/udhr/) corpus (from Latin and Greek
  to Tibetan, Cherokee, Vai, Adlam, Javanese and Grantha). The texts come from the MIT-licensed `udhr`
  npm package. Each text is cut after as many lines as the tile holds. Tiles are ordered from page 1 to page 2. Mongolian is set in vertical
  columns.
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
have to be installed first. Line breaking of Thai, Khmer, Lao and Burmese needs Helium 0.2.1 or later.

```
gallery/poster/fetch-fonts.sh            # downloads about 25 Noto fonts into ~/.fonts/helium-poster
cargo run --release -- gallery/poster/index.html gallery/poster/scripts-poster.pdf
```

The print is not listed in `gallery/order.txt` because the gallery build only uses the fonts built
into Helium, and these fonts are not.
