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
CARGO_PROFILE_RELEASE_STRIP=true cargo build --release --lib --target $TARGET

mkdir -p target/wasm
cp target/$TARGET/release/synths_core.wasm target/wasm/synths_core.wasm
echo "target/wasm/synths_core.wasm: $(wc -c < target/wasm/synths_core.wasm | tr -d ' ') bytes"

if command -v node >/dev/null 2>&1; then
    node scripts/check-wasm.mjs target/wasm/synths_core.wasm
else
    echo "node was not found, so the module was not checked."
fi
