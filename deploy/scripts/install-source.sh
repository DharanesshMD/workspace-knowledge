#!/bin/sh
set -eu
# Run from repository root. Never elevated. Override WK_PREFIX for a staging install.
[ "$(id -u)" != 0 ] || { echo 'Install as the developer, not root' >&2; exit 1; }
prefix=${WK_PREFIX:-"$HOME/.local"}
mkdir -p "$prefix/bin"
cargo build --locked --release -p wk-cli -p wk-agent -p wk-environments -p wk-server
for binary in wk wk-agent wk-env wk-server; do
  install -m 755 "target/release/$binary" "$prefix/bin/$binary"
done
printf '%s\n' 'Installed user binaries. Enroll explicitly before enabling a service.'
