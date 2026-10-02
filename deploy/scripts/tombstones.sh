#!/bin/sh
# Separately retain the newest deletion ledger; only operate on isolated restore targets.
set -eu
umask 077
[ "$#" = 3 ] || { echo 'usage: tombstones.sh export AGE_RECIPIENT OUTPUT.age | merge AGE_IDENTITY INPUT.age' >&2; exit 2; }
: "${PGSERVICE:?Configure a protected PostgreSQL service}"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM
case "$1" in
export)
  [ ! -e "$3" ] || exit 1
  psql -X -v ON_ERROR_STOP=1 -c "COPY (SELECT org,product,card_id,revision,created_at FROM tombstones ORDER BY org,product,card_id) TO STDOUT WITH (FORMAT csv, HEADER true)" > "$work/ledger.csv"
  age -r "$2" -o "$work/ledger.age" "$work/ledger.csv"
  destwork=$(mktemp -d "$(dirname -- "$3")/.wk-ledger.XXXXXXXX")
  trap 'rm -rf "$work" "$destwork"' EXIT HUP INT TERM
  cp "$work/ledger.age" "$destwork/ledger.age"
  ln "$destwork/ledger.age" "$3"
  ;;
merge)
  age -d -i "$2" -o "$work/ledger.csv" "$3"
  # COPY reads a file, not SQL interpolation. A malicious CSV field cannot execute SQL.
  # Keep this fixed relative name in a private working directory.
  cd "$work"
  psql -X -v ON_ERROR_STOP=1 <<'SQL'
BEGIN;
CREATE TEMP TABLE latest_tombstones (LIKE tombstones INCLUDING DEFAULTS) ON COMMIT DROP;
\copy latest_tombstones(org,product,card_id,revision,created_at) FROM 'ledger.csv' WITH (FORMAT csv, HEADER true)
INSERT INTO tombstones SELECT * FROM latest_tombstones
ON CONFLICT (org,product,card_id) DO UPDATE SET
 revision=GREATEST(tombstones.revision,EXCLUDED.revision),
 created_at=GREATEST(tombstones.created_at,EXCLUDED.created_at);
UPDATE cards SET withdrawn=true,published=false WHERE EXISTS
 (SELECT 1 FROM tombstones t WHERE t.org=cards.org AND t.product=cards.product AND t.card_id=cards.id);
COMMIT;
SQL
  ;;
*) exit 2 ;;
esac
