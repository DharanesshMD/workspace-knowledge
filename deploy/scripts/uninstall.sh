#!/bin/sh
set -eu
prefix=${WK_PREFIX:-"$HOME/.local"}
for binary in wk wk-agent wk-connect wk-env wk-server wk-mcp; do
  rm -f "$prefix/bin/$binary"
done
printf '%s\n' 'Removed binaries only. Stop services first; consent, keys, client configs and data are preserved.'
