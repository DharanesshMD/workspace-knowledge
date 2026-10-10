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
tar -xOf "$work/backup.tar" manifest.txt > "$work/manifest.txt"
printf '%s\n' 'wk-backup-v2' 'schema_version=12' > "$work/expected.txt"
cmp -s "$work/manifest.txt" "$work/expected.txt" || { echo 'Incompatible backup version; use a version-specific offline migration' >&2; exit 1; }
count=$(psql -X -At -v ON_ERROR_STOP=1 -c "SELECT count(*) FROM pg_tables WHERE schemaname='public'")
[ "$count" = 0 ] || { echo 'Target is not empty' >&2; exit 1; }
# Render first, then restore and revoke in ONE transaction. No restored access
# becomes visible if revocation fails. pg_restore emits no transaction here.
pg_restore --exit-on-error --no-owner --no-privileges --file="$work/restore.sql" "$work/database.dump"
{
  printf '%s\n' 'BEGIN;'
  cat "$work/restore.sql"
  cat <<'SQL'
DO $$ BEGIN
 IF (SELECT array_agg(version ORDER BY version) FROM public.wk_schema_migrations) IS DISTINCT FROM ARRAY[1,2,3,4,5,6,7,8,9,10,11,12]::integer[] THEN
  RAISE EXCEPTION 'incompatible restored schema';
 END IF;
END $$;
UPDATE public.workspace_bindings SET revoked=true;
UPDATE public.registered_devices SET revoked_at=COALESCE(revoked_at,now()),revision=revision+CASE WHEN revoked_at IS NULL THEN 1 ELSE 0 END;
DELETE FROM public.browser_sessions;
DELETE FROM public.product_admins;
DELETE FROM public.memberships;
UPDATE public.cards SET withdrawn=true, published=false WHERE EXISTS
 (SELECT 1 FROM public.tombstones t WHERE t.org=cards.org AND t.product=cards.product AND t.card_id=cards.id);
UPDATE public.personal_summaries p SET body=NULL,deleted=true,revision=GREATEST(p.revision,d.revision)
FROM public.personal_deletions d WHERE p.org=d.org AND p.product=d.product AND p.owner=d.owner
 AND (p.id=d.note_id OR encode(sha256(convert_to(p.idempotency_key,'UTF8')),'hex')=d.key_hash)
 AND (NOT p.deleted OR p.body IS NOT NULL OR p.revision<d.revision);
-- --no-privileges also drops original REVOKEs: deny PUBLIC before commit.
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM PUBLIC;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
UPDATE public.collector_security_epoch SET not_before=GREATEST(not_before,floor(extract(epoch FROM clock_timestamp()))::bigint+61) WHERE singleton;
COMMIT;
SQL
} > "$work/transaction.sql"
psql -X -v ON_ERROR_STOP=1 -f "$work/transaction.sql"
printf '%s\n' 'Restored with memberships, admin assignments and browser sessions removed; installations revoked. Keep API stopped: merge latest shared/personal journals, reapply restricted application grants and explicitly reprovision memberships. Wait past collector epoch then require fresh upstream collector sign-in.'
