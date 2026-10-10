# Encrypted hosted backup

Install PostgreSQL client tools, `age` and `tar`. Configure protected `PGSERVICE` / `.pgpass`; never pass passwords as arguments. Run `deploy/scripts/backup.sh AGE_PUBLIC_RECIPIENT /private/backup.age`. Version2 snapshots require exactly schema migrations1–12 and include the hosted database: shared cards, **sanitized owner-private summaries**, and access metadata. Raw conversations and local encrypted vaults are never part of this database backup. `pg_dump` supplies its consistent snapshot; coordinate the snapshot and independently retained journal recovery point operationally.

Export the latest journals independently after deletions or at the documented recovery point:

```sh
deploy/scripts/tombstones.sh export AGE_PUBLIC_RECIPIENT /private/shared-latest.age
deploy/scripts/personal-deletions.sh export AGE_PUBLIC_RECIPIENT /private/personal-latest.age
```

All three scripts use private temporary files, age encryption and atomic no-overwrite publication. Keep the private age identity offline separately. Age public-key encryption protects confidentiality and detects ciphertext modification; it does not authenticate who created a snapshot. Restore only trusted operator archives. Use an encrypted temporary filesystem to protect crash remnants and filesystem snapshots.

Keep both newest journals beyond the oldest retained snapshot. Journals contain scoped IDs/revisions/timestamps and, for personal deletions, hashed idempotency keys; no summary text or raw messages. Current implementation retains deletion and installation tombstones indefinitely. There is no external durable journal replication daemon. The deployment operator must enforce backup expiry (initial recommendation30days) and a recovery point that includes post-snapshot deletions. Physical SQL cleanup does not erase old WAL, pages or snapshots.

OpenBao, its recovery/unseal keys, local vaults and OIDC state require separate storage-specific backup and recovery. This script does not back them up. Application recovery measurements are synthetic evidence, not a production RPO/RTO guarantee.
