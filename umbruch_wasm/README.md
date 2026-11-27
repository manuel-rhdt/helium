# umbruch_wasm

Umbruch in the browser: `render(html)` turns an HTML document into a PDF, entirely on the client.

```js
import init, { render } from "./umbruch_wasm.js";

await init();                       // fetches and compiles umbruch_wasm_bg.wasm
const result = render("<!DOCTYPE html><h1>Hello</h1>");
if (result.error) console.error(result.error, result.log);
else new Blob([result.pdf], { type: "application/pdf" });   // result.pages, result.log
```

`render` does not throw for a document that cannot be rendered; it returns `error`. `log` holds the
warnings (unknown CSS, missing glyphs, ...) as `[WARN module] message` lines.

What differs from the `umbruch` executable: there is no file system and no system fonts, so only the
built-in fonts are used and only `data:` URIs load (images, `@font-face`). The engine runs on the
calling thread and a render blocks it, so call it from a Web Worker to keep a page responsive.

The module is about 16 MB (6 MB with Brotli): the engine with TeX Gyre, the math fonts and DejaVu Sans.
The Noto fallback fonts for CJK, emoji and the other scripts are not built in, because they are 18 MB of what
would be a 32 MB module (19 MB with Brotli). Text in those scripts is not drawn (the log lists the missing
characters) unless the document brings a font with `@font-face` and a `data:` URI. Build with
`umbruch_wasm/build.sh --features embedded-fonts` (the cargo feature of the same name) to include them.

## Building

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100 --locked
umbruch_wasm/build.sh               # -> umbruch_wasm/pkg
```

The release workflow publishes the package as `umbruch-wasm-<tag>.tar.gz` with every release.
