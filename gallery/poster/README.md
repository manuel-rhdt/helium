# Scripts (A3 print)

`index.html` is an A3 print of 140 writing systems, set only by their shapes: six characters of each
script, ordered by the date of its earliest use, with its name and date underneath. The band at the top
holds an Egyptian cartouche (a CSS border around the signs of *Ra-ms-s*), the Maya numerals of the Long
Count date 9.8.9.13.0, and a vermilion seal. Maya glyph blocks are not encoded in Unicode, so only the
numerals can be set as text.

`scripts-poster.pdf` is the result.

## Render it

Helium has no `@font-face` yet and uses the fonts installed on the system, so the extra Noto fonts
have to be installed first:

```
gallery/poster/fetch-fonts.sh            # downloads about 115 Noto fonts into ~/.fonts/helium-poster
cargo run --release -- gallery/poster/index.html gallery/poster/scripts-poster.pdf
```

The print is not listed in `gallery/order.txt` because the gallery build only uses the fonts built
into Helium, and these fonts are not.
