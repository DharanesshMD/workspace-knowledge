# Versioned integration schemas

These JSON Schema documents are generated from the Rust wire types. They are checked against fresh generation by workspace tests.

- `card-submission-v1.json`: sanitized summary submission only. Caller-supplied lifecycle is restricted to proposed/in_progress. Authenticated product and author come from the server, not the submitted card. Optional `provenance` (`assistant_written`) marks a card the developer's assistant wrote and the developer reviewed; it is additive and omitted when absent.
- `private-event-v1.json`: normalized local collector event. This includes private visible messages and must never be sent to shared knowledge endpoints. Paths still require live local consent and scope validation.
- `effective-policy-v1.json`: server membership projection returned by the policy endpoint. It cannot be used to grant permissions.

Regenerate from the repository root:

```sh
./scripts/cargo run -q -p wk-contracts --example schema > docs/architecture/schemas/card-submission-v1.json
./scripts/cargo run -q -p wk-capture --example schema > docs/architecture/schemas/private-event-v1.json
./scripts/cargo run -q -p wk-knowledge --example schema > docs/architecture/schemas/effective-policy-v1.json
```

Schema validation describes shape and selected field constraints. Runtime validation is authoritative for UTF-8 byte limits, whitespace rules, retention, path boundaries, duplicate conflicts, privacy screening, permissions and trusted-evidence promotion. Passing a schema is never permission to capture or publish. The event and policy documents describe version 1; the event wire type includes `schema_version`, while policy compatibility is versioned in this document's identifier.
