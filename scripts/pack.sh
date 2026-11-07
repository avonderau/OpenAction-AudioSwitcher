#!/bin/sh
# Builds dist/com.avonderau.audioswitcher.streamDeckPlugin: the plugin folder with
# static Linux binaries for x86_64 and aarch64.
# Needs: rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
set -eu
cd "$(dirname "$0")/.."
plugin=com.avonderau.audioswitcher.sdPlugin
rm -rf "$plugin/bin"
for pair in x86_64-unknown-linux-musl:x86_64-unknown-linux-gnu aarch64-unknown-linux-musl:aarch64-unknown-linux-gnu; do
  build=${pair%%:*}
  host=${pair##*:}
  cargo build --release --locked --target "$build"
  mkdir -p "$plugin/bin/$host"
  cp "target/$build/release/audioswitcher" "$plugin/bin/$host/audioswitcher"
done
mkdir -p dist
out="dist/com.avonderau.audioswitcher.streamDeckPlugin"
rm -f "$out"
zip -qr -X "$out" "$plugin" -x '*.DS_Store'
echo "$out"
