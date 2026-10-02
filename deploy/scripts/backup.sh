#!/bin/sh
set -eu
umask 077
[ "$#" = 2 ] || { echo 'usage: backup.sh AGE_RECIPIENT OUTPUT.age' >&2; exit 2; }
[ ! -e "$2" ] || { echo 'Refusing existing output' >&2; exit 1; }
: "${PGSERVICE:?Configure a restricted pg_service.conf and .pgpass; do not pass credentials as arguments}"
parent=$(dirname -- "$2")
work=$(mktemp -d "$parent/.wk-backup.XXXXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM
pg_dump --format=custom --file="$work/database.dump"
printf '%s\n' 'wk-backup-v1; shared database only; restore denies all access until reprovisioned' > "$work/manifest.txt"
# age encryption completes before the destination is installed.
tar -C "$work" -cf - database.dump manifest.txt | age -r "$1" -o "$work/backup.age"
# The encrypted temporary file is on the destination filesystem. link(2) installs
# it atomically and refuses an existing destination, including a racing writer.
ln "$work/backup.age" "$2"
printf '%s\n' 'Encrypted shared backup created. Vault and latest tombstones require separate procedures.'
