#!/bin/sh
# Builds the engine as a WebAssembly module: target/wasm/synths_core.wasm
#
# It is built for the wasm32-wasip1 target, which needs that target's standard
# library. If the cargo on your PATH does not have it, this looks for a rustup
# toolchain that does. It installs nothing.
set -e
cd "$(dirname "$0")/.."

TARGET=wasm32-wasip1

has_target() {
    [ -d "$("$1" --print target-libdir --target $TARGET 2>/dev/null)" ]
}

if ! has_target rustc; then
    for dir in "$HOME"/.rustup/toolchains/*/bin; do
        if [ -x "$dir/rustc" ] && has_target "$dir/rustc"; then
            PATH="$dir:$PATH"
            export PATH
            break
        fi
    done
fi
if ! has_target rustc; then
    echo "No Rust toolchain with the $TARGET target was found." >&2
    echo "With rustup: rustup target add $TARGET" >&2
    exit 1
fi

# Leave out the names of functions: they are a third of the file and the
# browser does not need them.
# The JSON song reader (the "json" feature) is left in, though it is about
# 35 KB: one reader then decides what a song's JSON means, for the page, for
# Python and for a native app alike. Two smaller modules can be asked for:
#   --no-json   reads MOD, XM and packed songs only: synths_core.no-json.wasm
#   --no-songs  no song player at all, only the synth: synths_core.synth-only.wasm
case "${1:-}" in
    --no-songs) FEATURES=""; OUT=target/wasm/synths_core.synth-only.wasm ;;
    --no-json) FEATURES="songs"; OUT=target/wasm/synths_core.no-json.wasm ;;
    "") FEATURES="songs json"; OUT=target/wasm/synths_core.wasm ;;
    *) echo "usage: $0 [--no-json | --no-songs]" >&2; exit 2 ;;
esac
CARGO_PROFILE_RELEASE_STRIP=true cargo build --release --lib --target $TARGET --no-default-features --features "$FEATURES"

mkdir -p target/wasm
cp target/$TARGET/release/synths_core.wasm "$OUT"
echo "$OUT: $(wc -c < "$OUT" | tr -d ' ') bytes"

if command -v node >/dev/null 2>&1; then
    node scripts/check-wasm.mjs "$OUT"
else
    echo "node was not found, so the module was not checked."
fi
