# Upgrade and failed migration recovery

Read schema/release notes, pin the proposed artifact, verify its checksum/signature, and record current application/database versions. Stop application writes and take encrypted backups plus a current deletion ledger. Rehearse against an isolated restore with all memberships removed.

Run `wk-server migrate` using the migration role, then start the upgraded app with its ordinary application role. Never give application processes migration-owner credentials. Migrations must complete before traffic is enabled. Check authorization, withdrawal, private drafts, schema metadata and local queue behavior with synthetic users.

If migration fails, keep traffic stopped. Investigate its transaction and recorded migration version; do not blindly retry destructive DDL or run an old executable against a newer schema. Roll back the binary only when backward compatibility is established. Otherwise restore to a fresh database using the restore runbook, reapply latest tombstones and current policies, then reprovision. No automatic downgrade/zero-downtime guarantee is made.
