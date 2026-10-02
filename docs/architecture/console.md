# Developer console

React/TypeScript/Vite console in `web/`; build with `npm ci && npm run build`. Serve `web/dist` through the same origin as the API (`WK_WEB_DIR`). Configure the exact `WK_CONSOLE_ORIGIN`. The UI accepts an externally issued token and retains it only in React memory. It does not implement an identity provider or persist tokens in browser storage.

Views: product search, authorized summary details, review queue, correction at expected revision, withdrawal, feedback, historical summaries, permission-filtered graph, current membership policy and metadata-only environment schemas. Missing collector coverage is explicitly shown as not reported. Static text rendering avoids active HTML from summaries. Native capture health remains available through local `wk-agent doctor`, not exposed to remote peers.

Browser tests use synthetic route responses for authorization failures and empty states. These do not replace real identity, backend or client certification.
