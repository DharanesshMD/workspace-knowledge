#!/bin/sh
# Keep the latest body-free journal independently of every retained snapshot.
set -eu
umask 077
[ "$#" = 3 ] || { echo 'usage: personal-deletions.sh export AGE_RECIPIENT OUTPUT.age | merge AGE_IDENTITY INPUT.age' >&2; exit 2; }
: "${PGSERVICE:?Configure a protected PostgreSQL service}"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM
case "$1" in
export)
  [ ! -e "$3" ] || exit 1
  psql -X -v ON_ERROR_STOP=1 -c "COPY (SELECT org,product,owner,note_id,revision,key_hash,deleted_at FROM public.personal_deletions ORDER BY org,product,owner,key_hash) TO STDOUT WITH (FORMAT csv, HEADER true)" > "$work/ledger.csv"
  age -r "$2" -o "$work/ledger.age" "$work/ledger.csv"
  destwork=$(mktemp -d "$(dirname -- "$3")/.wk-personal-ledger.XXXXXXXX")
  trap 'rm -rf "$work" "$destwork"' EXIT HUP INT TERM
  cp "$work/ledger.age" "$destwork/ledger.age"
  ln "$destwork/ledger.age" "$3"
  ;;
merge)
  age -d -i "$2" -o "$work/ledger.csv" "$3"
  [ "$(head -n 1 "$work/ledger.csv")" = 'org,product,owner,note_id,revision,key_hash,deleted_at' ] || { echo 'Invalid journal header' >&2; exit 1; }
  cd "$work"
  psql -X -v ON_ERROR_STOP=1 <<'SQL'
BEGIN;
CREATE TEMP TABLE latest_personal_deletions (LIKE public.personal_deletions INCLUDING DEFAULTS INCLUDING CONSTRAINTS) ON COMMIT DROP;
\copy latest_personal_deletions(org,product,owner,note_id,revision,key_hash,deleted_at) FROM 'ledger.csv' WITH (FORMAT csv, HEADER true)
INSERT INTO public.personal_deletions SELECT * FROM latest_personal_deletions
ON CONFLICT (org,product,owner,key_hash) DO UPDATE SET
 revision=GREATEST(personal_deletions.revision,EXCLUDED.revision),
 deleted_at=GREATEST(personal_deletions.deleted_at,EXCLUDED.deleted_at);
UPDATE public.personal_summaries p SET body=NULL,deleted=true,revision=GREATEST(p.revision,d.revision)
FROM public.personal_deletions d WHERE p.org=d.org AND p.product=d.product AND p.owner=d.owner
 AND (p.id=d.note_id OR encode(sha256(convert_to(p.idempotency_key,'UTF8')),'hex')=d.key_hash)
 AND (NOT p.deleted OR p.body IS NOT NULL OR p.revision<d.revision);
COMMIT;
SQL
  ;;
*) exit 2 ;;
esac
