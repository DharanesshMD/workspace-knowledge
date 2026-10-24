# v0.1.0-alpha.4: draft release notes and owner checklist

**Status: draft. No tag or GitHub release exists for alpha.4.** The latest published release is the source-only `v0.1.0-alpha.3` prerelease. Cut alpha.4 from the final commit only after the checklist below; the notes must then be updated with that commit's SHA. Nothing here makes the project production-supported: see the [production readiness ledger](production-readiness.md).

## Draft release notes

**Workspace Knowledge v0.1.0-alpha.4: source-only evaluation prerelease**

A self-hostable alpha for small teams: a Linux Docker server (PostgreSQL, an OIDC provider, an HTTPS reverse proxy) and macOS developer machines. Built from source; no binaries are attached and nothing is signed or notarized. Not production-certified. Use disposable or synthetic data. Automatic publication is off: everything a developer captures stays private until they review and submit it.

### What changed since alpha.3

- **Log in once to the hosted MCP.** `wk-mcp login` signs a developer in (authorization code with PKCE) and renews tokens silently from the OS credential store, so no access token has to be copied. A prefilled Keycloak realm and a [quick start](quick-start-keycloak.md) cover the server side. Claude Code and the Cursor agent CLI sign in to the server's MCP endpoint themselves once the realm allows it; the Cursor editor and VS Code 1.142.0 also passed sign-in and silent renewal ([results](../reviews/2026-10-19-remote-mcp-signin-results.md)); VS Code needs a realm access-token lifespan above 5 minutes. No client was tested with a real model calling a tool.
- **Identity providers behind a private CA.** `WK_OIDC_CA_FILE` for the server and `--ca-file` for `wk-agent device connect` and `wk-mcp login`. Keycloak 26.4 is the one provider qualified so far ([guide](oidc-keycloak.md)).
- **Publish without review, on by default for new setups.** A developer's own assistant-written cards go live right after `wk-agent draft`, with no reviewer, once they pass screening. New collectors have it on (`--no-auto-publish` opts out), the server enables every product (`WK_AUTO_PUBLISH_PRODUCTS=none` turns it off), and collectors set up before alpha.4 stay off until the developer runs `wk-agent auto-publish enable --yes`. Cards can be withdrawn and each publication is audited ([setup](../adapters/setup.md#publishing-without-review-on-by-default-for-new-setups)). This is a change from alpha.3, where every card waited for a reviewer.
- **Better default summaries.** The assistant-written flow (`/wk-summarize`) is the recommended path and a reviewer can publish such cards. Rules-mode drafts are the zero-setup fallback and now avoid greetings and pasted traces as titles.
- **Faster search.** Schema migration 18 stores the card search vector and adds a missing graph index. Five-developer search p95 fell from about 0.9 s to 34 ms in a local synthetic test ([measurements and limits](../reviews/2026-10-21-capacity-and-search.md)).
- **One capture support table.** [`docs/adapters/support.md`](../adapters/support.md) lists tested client versions, what is captured and what is held. Unsupported versions are refused with a message that points to it.
- **Fixes.** `knowledge.record` no longer shows Claude Code an unusable tool schema; contact-detail screening no longer flags UUIDs and dates as phone numbers; `rustls-webpki` and `webpki-roots` are updated for two certificate-trust advisories (RUSTSEC-2026-0382 and RUSTSEC-2026-0381).

### Upgrading from alpha.3

- **Review before publish is no longer the default for new setups.** If you want a reviewer on every card, set `WK_AUTO_PUBLISH_PRODUCTS=none` on the server before onboarding developers. Existing collectors keep their old behavior.
- The server now needs **schema 18**. Run `wk-server migrate` with the operator connection before starting the new binary.
- Backup snapshots taken at schema 17 are refused by the schema-18 restore script; follow the offline upgrade in the [restore guide](restore.md) and take a new snapshot after migrating.
- New collectors default to `rules` summaries. Existing `local` configurations are unchanged, but a `rules` configuration cannot be read by older binaries.

### Known limits

- Capture is partial and pinned to specific client versions; Codex and Antigravity are experimental and Windows is unsupported ([table](../adapters/support.md)).
- No hosted or public-CA identity provider, real-team pilot, signed macOS binaries or independent security review yet. Capacity figures are from a local Docker VM with synthetic text and are not a forecast.
- GitHub Actions had not run on the final commit when these notes were written (account billing block, still in effect on 2026-10-24); local verification results are in the [readiness ledger](production-readiness.md): the full local suite, web tests, `npm audit` and the 85-check Keycloak run passed on 2026-10-24, and `cargo audit` reports no vulnerabilities (five informational advisories). The Gitleaks history scan found no leaks on the pushed branches and tags after two exact-line allowlist entries for false positives were added to `.gitleaks.toml`.

## Owner checklist before publishing

Agents prepare; only the owner does these.

1. **Billing and CI.** Resolve the GitHub billing block (or make the repository public, which gives free minutes), then run both workflows on the final commit and the weekly client-release check once by hand.
2. **Security contact.** `SECURITY.md` currently says private vulnerability reporting is not enabled and names no channel, which is accurate while the repository is private. After going public, enable *Settings → Code security → Private vulnerability reporting*, confirm the "Report a vulnerability" button works, then replace the first paragraph's channel sentences in `SECURITY.md` with: "Report vulnerabilities privately with GitHub's *Report a vulnerability* button on the Security tab." Do not add an email address you do not monitor.
3. **Git history.** About 94 commits carry a personal email address and earlier reviews noted local path metadata. Decide whether that is acceptable. Rewriting history would invalidate the existing alpha tags.
4. **Final verification.** Done locally on 2026-10-24 (see the ledger); repeat on the exact commit to tag, and let CI run once billing allows: formatting, strict Clippy, the full Rust test suite against a disposable PostgreSQL with `--include-ignored`, the Python tests, `scripts/keycloak-acceptance.py`, the dependency audit and a secret scan of the published history. Record the SHA in the ledger.
5. **Cut the prerelease** (marked *pre-release*, source only):

   ```sh
   git tag -a v0.1.0-alpha.4 -m "Source-only evaluation prerelease" <final-sha>
   git push origin v0.1.0-alpha.4
   gh release create v0.1.0-alpha.4 --prerelease --title "v0.1.0-alpha.4" --notes-file <edited-notes.md>
   ```

   Before running it, rewrite the notes above with the SHA and final results, and move the `Unreleased` changelog section under an `alpha.4` heading.
6. **Visibility.** Change repository visibility manually, then re-check the README, SECURITY and release page from a logged-out browser.
