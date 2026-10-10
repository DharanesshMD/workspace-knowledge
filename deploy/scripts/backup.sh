#!/bin/sh
set -eu
umask 077
[ "$#" = 2 ] || { echo 'usage: backup.sh AGE_RECIPIENT OUTPUT.age' >&2; exit 2; }
[ ! -e "$2" ] || { echo 'Refusing existing output' >&2; exit 1; }
: "${PGSERVICE:?Configure a restricted pg_service.conf and .pgpass; do not pass credentials as arguments}"
parent=$(dirname -- "$2")
work=$(mktemp -d "$parent/.wk-backup.XXXXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM
version=$(psql -X -At -v ON_ERROR_STOP=1 -c "SELECT array_agg(version ORDER BY version)::text FROM public.wk_schema_migrations")
[ "$version" = '{1,2,3,4,5,6,7,8,9,10,11,12}' ] || { echo 'Unsupported source schema' >&2; exit 1; }
pg_dump --format=custom --file="$work/database.dump"
printf '%s\n' 'wk-backup-v2' 'schema_version=12' > "$work/manifest.txt"
# age encryption completes before the destination is installed.
tar -C "$work" -cf - database.dump manifest.txt | age -r "$1" -o "$work/backup.age"
# The encrypted temporary file is on the destination filesystem. link(2) installs
# it atomically and refuses an existing destination, including a racing writer.
ln "$work/backup.age" "$2"
printf '%s\n' 'Encrypted hosted database backup created, including sanitized private summaries. Local vaults and latest shared/personal deletion journals require separate procedures.'
