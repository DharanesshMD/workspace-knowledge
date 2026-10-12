# Restore without reopening historical access

Stop API, retention workers and client access. Restore into a new isolated empty database with an operator service credential and no application network access:

```sh
deploy/scripts/restore.sh AGE_IDENTITY BACKUP.age --isolated-empty-database
deploy/scripts/tombstones.sh merge AGE_IDENTITY SHARED_LATEST.age
deploy/scripts/personal-deletions.sh merge AGE_IDENTITY PERSONAL_LATEST.age
```

Version2 requires manifest schema16 and actual migrations1–16. Populated targets, unknown versions and incompatible dumps are rejected. Earlier version2/schema12–15 snapshots and older version1 archives need a version-specific **offline** upgrade in another isolated database; migrate with the matching trusted release, validate deletion/access metadata and create a version2 snapshot. Do not edit a manifest to bypass compatibility.

Restore runs schema/data, access quarantine and embedded deletion replay in one transaction. It removes memberships, product-admin assignments and browser sessions; revokes every installation and workspace binding; preserves credential/key tombstones; and denies PUBLIC function execution/schema access. `--no-privileges` drops the dump's original REVOKEs, so explicit denial is necessary. Any SQL failure rolls the transaction back. Never start an application after a failed operation.

The collector authentication epoch advances beyond the60-second allowed issuer clock skew (restore time+61seconds). An old upstream token cannot enroll an unseen installation after membership regrant. Wait past the recorded epoch and require **fresh native sign-in and installation consent**. Collector JWTs require an `iat` claim after restore; opaque development tokens cannot satisfy that boundary. Ordinary API/MCP token verification remains independent and still requires a currently granted membership. Validate the production issuer's issued-at behavior before release.

Merge BOTH independently retained latest journals before granting access. A snapshot cannot know later deletions; if either authoritative journal is missing or uncertain, leave data inaccessible. Merge checks exact CSV headers/types and operates in one transaction; malformed data fails without partial insertion. Personal replay clears body text and prevents a later idempotent retry from recreating a note absent from the snapshot. Shared replay withdraws matching scoped cards. Repeat merge is idempotent.

Reapply `deploy/scripts/application-role.sql` for the restricted runtime role; never serve with the restore owner credential. Review current policies and explicitly provision only presently authorized memberships/identities, then assign necessary admin roles using protected bootstrap. Do not restore membership snapshots. Restore vault separately, rotate/revoke obsolete credentials, renew local consent and test owner isolation, revoked keys and current identities before reconnecting clients.

`scripts/hosted-restore-drill.py` operates only in new synthetic databases inside the owned `wk-cursor-native-test` container. It exercises actual age-encrypted backup/restore, access quarantine, PUBLIC denial, both latest journals, post-snapshot orphan-key protection, unrelated-summary preservation, repeat merge, nonempty refusal, malformed rows, tampered ciphertext, incompatible manifests and rollback on injected pre-commit failure. It also cleans up its own databases/files. Rust collector-epoch tests independently cover unseen old JWTs, missing/future issued-at claims, fresh sign-in after modeled cooldown and temporary database failures. Production OIDC/TLS/vault and real restored-native reconnection remain separate acceptance gates.
