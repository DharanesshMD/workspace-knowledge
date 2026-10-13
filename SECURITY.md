# Security reporting

Do not include credentials, personal transcripts, production database dumps or real customer data in public issues or pull requests.

After the public alpha launch, the repository owner, [DharanesshMD](https://github.com/DharanesshMD), will monitor GitHub private vulnerability reports. Use **Security → Report a vulnerability** on this repository. Private reporting is not yet enabled: this candidate repository remains private, and enabling and verifying that channel is part of the [public launch checklist](docs/operations/public-alpha.md). If the button is unavailable, request a private reporting channel without disclosing vulnerability details in a public issue. No security email, response SLA or paid support is promised.

Include the affected revision, a synthetic minimal reproduction, impact and relevant platform/client versions. Avoid real accounts or customer data in proof-of-concept files. Coordinate disclosure privately with the owner.

## Supported security scope

There is no published supported release yet. The first candidate is a source-only alpha for Linux Docker servers and macOS developers; use synthetic or disposable environments while evaluating it. Production OIDC/TLS, a real-server/team pilot, signed macOS distribution and a broader model-sensitive privacy corpus remain pending. Automatic publication is disabled; code-context retrieval is advisory, and the optional Claude hook covers only the locally tested Edit/Write surface.

The previous **RUSTSEC-2023-0071** package was removed from the runtime graph and full lockfile. Unsuppressed Cargo audit reports zero vulnerabilities and one informational unmaintained-PEM-parser warning; that is not comprehensive security certification. Read the [remediation and fork maintenance assessment](docs/security/rsa-runtime-disposition.md), [threat model](docs/security/threat-model.md) and [restore boundaries](docs/operations/restore.md).

Production security certification and penetration testing are pending. Public source availability would not change these support or security boundaries.
