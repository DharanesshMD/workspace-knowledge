#!/bin/sh
set -eu
umask 077
[ "$#" = 3 ] || { echo 'usage: restore.sh AGE_IDENTITY BACKUP.age --isolated-empty-database' >&2; exit 2; }
[ "$3" = --isolated-empty-database ] || exit 2
: "${PGSERVICE:?Use an isolated empty database service with administrator access}"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM
age -d -i "$1" -o "$work/backup.tar" "$2"
# Extract only the single known data file, ignoring arbitrary archive paths.
tar -xOf "$work/backup.tar" database.dump > "$work/database.dump"
count=$(psql -X -At -v ON_ERROR_STOP=1 -c "SELECT count(*) FROM pg_tables WHERE schemaname='public'")
[ "$count" = 0 ] || { echo 'Target is not empty' >&2; exit 1; }
# Render first, then restore and revoke in ONE transaction. No restored access
# becomes visible if revocation fails. pg_restore emits no transaction here.
pg_restore --exit-on-error --no-owner --no-privileges --file="$work/restore.sql" "$work/database.dump"
{
  printf '%s\n' 'BEGIN;'
  cat "$work/restore.sql"
  cat <<'SQL'
UPDATE public.workspace_bindings SET revoked=true;
DELETE FROM public.memberships;
UPDATE public.cards SET withdrawn=true, published=false WHERE EXISTS
 (SELECT 1 FROM public.tombstones t WHERE t.org=cards.org AND t.product=cards.product AND t.card_id=cards.id);
COMMIT;
SQL
} > "$work/transaction.sql"
psql -X -v ON_ERROR_STOP=1 -f "$work/transaction.sql"
printf '%s\n' 'Restored with ALL memberships removed. Keep API stopped: merge latest tombstones, replay, and explicitly reprovision identities/consent before serving.'
