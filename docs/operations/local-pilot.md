# Local MCP pilot

This protocol is for the current single-Mac evaluation with the existing local Docker stack. It uses synthetic projects and two developer personas, not a real-team or clean-machine pilot. Do not use a successful local session to claim production support.

## Starting state

The established local setup uses `https://wk.localhost:8444`, product `demo`, Codex as Ravi and Antigravity as Anika. Their synthetic projects are `~/wk-demo/codex-ravi` and `~/wk-demo/antigravity-anika`. The configured MCP entry is `workspace-knowledge-local`.

Check both connections:

```sh
wk-connect check --client codex --name workspace-knowledge-local
wk-connect check --client antigravity --name workspace-knowledge-local
```

Both should report authenticated discovery of 12 tools. Start a new client session or refresh MCP connections after replacing binaries or credentials. For a fresh setup, follow [client onboarding](client-onboarding.md); its default entry name is `workspace-knowledge`, so omit the custom `--name` unless deliberately reusing the local trial name. Use a private token file or the hidden prompt, never a token pasted into a chat or command argument.

## Working session

1. In Codex, ask: “Look up Anika's `WK-LIVE-20261107` work on `backend/retry.py`. Explain the recorded decisions and evidence. Distinguish publication from implementation lifecycle. Do not change or share anything.” Verify the answer calls `knowledge.team_work` and agrees with the returned card and reported files.
2. In Antigravity, ask the equivalent question about Ravi. Verify attribution and constraints against the returned card. An assistant's reported test result is not a substitute for examining or rerunning the test.
3. Make a small change only in a synthetic project. Ask explicitly to share its summary with a unique `WK-PILOT-` marker and the actual changed files. Do not claim tests that were not run. Verify the other client retrieves that exact card and its correct publication state.
4. Ask about an unknown developer and a directory-prefix lookalike. Expect an explicit unknown-name response and no fabricated file matches. Verify service isolation with a deliberately scoped disposable account through the automated lifecycle test below.
5. Record the tested commit, OS/client versions, returned card IDs, commands/check results and any issue. Leave unexecuted steps pending. Withdraw disposable shared cards through the normal reviewer/author workflow; do not remove audit or deletion history.

Automatic conversation capture is outside this pilot. Keep the exact-client capture holds and approval boundaries intact.

## Automated failure and recovery checks

With the existing local evaluation stack and protected operator state:

```sh
python3 scripts/client-onboarding-acceptance.py
```

This creates a private HOME/prefix and disposable backend-scoped developer. It checks installation, invalid trust, direct native proxy startup, sharing/retrieval, service isolation, server outage/restart, token revocation/expiry/replacement, repeatable disconnect, uninstall/reinstall and cleanup. It briefly stops the local app and restarts it even on the tested failure path. Run it when a short interruption is acceptable.

For operators, `scripts/keycloak-acceptance.py` creates and removes a separate synthetic HTTPS identity stack. `scripts/hosted-restore-drill.py` requires a separately owned disposable PostgreSQL `wk_test` service with `age`; it never targets the normal application database. Read the script prerequisites and [restore guide](restore.md) before using it. Never point these drills at valued data.

## Exit criteria

For a local evaluation session, both clients must connect, retrieve the correct cross-persona work and preserve publication/implementation distinctions. Explicit sharing must remain scoped. Automated outage, credential and recovery checks must pass with their disposable resources cleaned up. Any leaked credential, unauthorized data, successful revoked-token use, incorrect attribution, lost settings or failed restore is a stop condition to investigate.

The local pilot can then be recorded as passed for its exact environment and executed cases. The next public-release gates still require a clean physical Mac, signing/notarization, a chosen hosted identity/deployment environment, independent security review and a consented real team. Those gates cannot be replaced by additional synthetic personas on this Mac.
