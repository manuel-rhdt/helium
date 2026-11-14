# Scripts poster (A3)

`index.html` is an A3 poster of 144 writing systems, from Egyptian hieroglyphs and cuneiform to
scripts invented within living memory. Each tile shows the first characters of a script as Unicode
encodes them, in the script's own font and direction. The header has a cartouche (a CSS border
around the hieroglyphs of *Ra-ms-s*) and the Maya Long Count date 9.8.9.13.0 in Mayan numerals.
Maya glyph blocks are not encoded in Unicode, so only the numerals can be set as text.

`scripts-poster.pdf` is the result.

## Render it

Helium has no `@font-face` yet and uses the fonts installed on the system, so the extra Noto fonts
have to be installed first:

```
gallery/poster/fetch-fonts.sh            # downloads about 115 Noto fonts into ~/.fonts/helium-poster
cargo run --release -- gallery/poster/index.html gallery/poster/scripts-poster.pdf
```

The poster is not listed in `gallery/order.txt` because the gallery build only uses the fonts built
into Helium, and these fonts are not.
