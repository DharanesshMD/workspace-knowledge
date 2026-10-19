# Security reporting

Do not include credentials, personal transcripts, production database dumps or real customer data in public issues or pull requests.

The latest release is the source-only GitHub prerelease [`v0.1.0-alpha.3`](https://github.com/DharanesshMD/workspace-knowledge/releases/tag/v0.1.0-alpha.3), published while this repository was private. It is an evaluation prerelease, not a production-supported release. The repository remains private until the owner changes visibility manually. GitHub private vulnerability reporting is not yet enabled; before relying on it, the owner must enable and verify the channel after changing repository visibility. If the reporting button is unavailable, do not disclose vulnerability details in a public issue; contact the repository owner through an already trusted private channel. No security email, response SLA or paid support is promised. Current readiness is tracked in the [production readiness ledger](docs/operations/production-readiness.md).

Include the affected revision, a synthetic minimal reproduction, impact and relevant platform/client versions. Avoid real accounts or customer data in proof-of-concept files. Coordinate disclosure privately with the owner.

## Supported security scope

There is no production-supported release. The published `v0.1.0-alpha.3` source prerelease targets evaluation on Linux Docker servers with macOS developers; use synthetic or disposable environments. Production OIDC/HTTPS, a real-team target-server pilot, signed/notarized macOS distribution, broader model-sensitive privacy qualification, exact-head CI, and independent security review remain gates. Automatic publication is disabled; code-context retrieval is advisory, and the optional Claude hook covers only the locally tested Edit/Write surface.

The previous **RUSTSEC-2023-0071** package was removed from the runtime graph and full lockfile. The latest local Cargo audit reports zero vulnerability findings and four informational unmaintained-dependency advisories: [`rustls-pemfile`](https://rustsec.org/advisories/RUSTSEC-2025-0134.html), [`zbus_names`](https://rustsec.org/advisories/RUSTSEC-2026-0371.html), [`zvariant`](https://rustsec.org/advisories/RUSTSEC-2026-0370.html), and [`zvariant_derive`](https://rustsec.org/advisories/RUSTSEC-2026-0372.html). This is not comprehensive security certification; resolve or explicitly accept the maintenance exposure before production support. Read the [remediation and fork maintenance assessment](docs/security/rsa-runtime-disposition.md), [threat model](docs/security/threat-model.md) and [restore boundaries](docs/operations/restore.md).

Production security certification and penetration testing are pending. Public source availability would not change these support or security boundaries.
