#!/bin/sh
set -eu
# Never elevated. Override WK_PREFIX for a staging install.
[ "$(id -u)" != 0 ] || { echo 'Install as the developer, not root' >&2; exit 1; }
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
prefix=${WK_PREFIX:-"$HOME/.local"}
./scripts/cargo build --locked --release -p wk-cli -p wk-agent -p wk-environments -p wk-server -p wk-mcp
mkdir -p "$prefix/bin"
for binary in wk wk-agent wk-env wk-server wk-mcp; do
  install -m 755 "target/release/$binary" "$prefix/bin/$binary"
done
printf 'Installed wk, wk-agent, wk-env, wk-server and wk-mcp in %s/bin\n' "$prefix"
printf '%s\n' 'Add this directory to PATH if needed. See docs/operations/install.md for team connection.'
printf '%s\n' 'No capture consent, hooks or startup services were enabled.'
