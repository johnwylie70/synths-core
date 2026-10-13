#!/bin/sh
# Fetches the OpenMPT player test modules for MOD and XM into
# target/openmpt-tests/, where tests/replayer_openmpt.rs looks for them.
#
# The modules are listed, with what each one tests, on the OpenMPT wiki:
#   https://wiki.openmpt.org/Development:_Test_Cases/MOD
#   https://wiki.openmpt.org/Development:_Test_Cases/XM
# The wiki's text is CC BY-SA 3.0. The module files carry no licence
# statement of their own, so they are fetched for testing and never
# committed (target/ is ignored). The tests skip when they are absent.
set -eu

cd "$(dirname "$0")/.."
for kind in MOD XM; do
    lower=$(echo "$kind" | tr 'A-Z' 'a-z')
    dir="target/openmpt-tests/$lower"
    mkdir -p "$dir"
    curl -fsSL "https://wiki.openmpt.org/Development:_Test_Cases/$kind" |
        grep -o "https://resources.openmpt.org/player_tests/$lower/[^\"]*" |
        sort -u |
        while read -r url; do
            name=$(basename "$url")
            [ -s "$dir/$name" ] || curl -fsSL "$url" -o "$dir/$name"
        done
    echo "$kind: $(ls "$dir" | wc -l | tr -d ' ') files in $dir"
done
