# Contributing

Use synthetic fixtures only. Never submit team conversations, credentials, `.env` files, customer data, or captured client histories in issues, tests, screenshots or pull requests. Explain the privacy boundary and relevant negative tests for changes.

Build from the pinned Rust toolchain and lockfiles. Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`. Run `npm ci`, `npm test`, and `npm run build` in `web`. PostgreSQL/OpenBao integration and native-client certification require explicitly configured disposable instances; report skipped integration checks honestly.

Contributions are Apache-2.0. Sign each commit with `git commit -s`, agreeing to the [Developer Certificate of Origin](https://developercertificate.org/). Do not use another person's identity. Dependency changes must preserve license notices and update lockfiles/SBOM. Public release requires separate maintainer authorization; a successful CI build is not authorization to publish.
