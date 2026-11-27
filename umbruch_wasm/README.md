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

The module is about 30 MB (18 MB with Brotli), mostly the bundled fallback fonts for CJK, emoji and
other scripts.

## Building

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100 --locked
umbruch_wasm/build.sh               # -> umbruch_wasm/pkg
```

The release workflow publishes the package as `umbruch-wasm-<tag>.tar.gz` with every release.
