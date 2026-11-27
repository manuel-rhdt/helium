#!/usr/bin/env bash
# Builds the WebAssembly package into umbruch_wasm/pkg: umbruch_wasm.js (an ES module), the .wasm
# and the TypeScript declarations. Arguments go to `cargo build`: the module is 16 MB without the Noto
# fallback fonts (CJK, emoji and the other scripts), which are the default for the umbruch
# executable; `--features embedded-fonts` builds them in for a module of 32 MB.
# Needs the wasm32 target and a wasm-bindgen CLI of the version that Cargo.toml pins:
#
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.100 --locked
set -euo pipefail
cd "$(dirname "$0")"

version="$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' Cargo.toml)"
if [[ "$(wasm-bindgen --version 2>/dev/null | cut -d' ' -f2)" != "$version" ]]; then
  echo "wasm-bindgen $version is needed: cargo install wasm-bindgen-cli --version $version --locked" >&2
  exit 1
fi

cargo build --release --locked --target wasm32-unknown-unknown "$@"
rm -rf pkg
wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/umbruch_wasm.wasm
ls -lh pkg
