# Developer console

React/TypeScript/Vite console in `web/`; build with `npm ci && npm run build`. Serve `web/dist` through the same HTTPS origin as the API (`WK_WEB_DIR`) and configure its exact `WK_CONSOLE_ORIGIN`.

Hosted sign-in uses the operator-configured OIDC provider, authorization code + PKCE, a browser-bound nonce/state transaction and an opaque HttpOnly cookie. No provider token reaches React or browser storage. `/v1/me` returns trusted identity and current permitted products; selecting another product clears the previous content and cancels pending requests. Sign out deletes this portal session and clears its cookie; it does not end the provider's own SSO session. Production without browser OIDC configuration shows setup-needed, without a token-paste fallback. Explicit development mode retains the memory-only bearer form.

Views currently include product search, authorized summary details, review queue, correction at expected revision, withdrawal, feedback, historical summaries, permission-filtered graph, current membership policy and metadata-only environment schemas. Static text rendering avoids active HTML from summaries. No conversation transcript or environment secret endpoint is provided. Native capture health remains available through local `wk-agent doctor`.

The owner-only My context store, team feed, device connection/settings and scoped administration described in the [portal roadmap](2026-10-10-hosted-developer-portal-plan.md) are subsequent increments; current review drafts are visible to authorized reviewers and must not be described as owner-only context.

Ten browser regressions cover synthetic authorization, product switching, logout, setup failures, existing workflows, and desktop/390px layout. A real server/PostgreSQL/local OIDC provider acceptance also passed; [exact evidence and limitations](../reviews/2026-10-10-hosted-identity-acceptance.md). Production-provider certification and capacity testing remain open.
