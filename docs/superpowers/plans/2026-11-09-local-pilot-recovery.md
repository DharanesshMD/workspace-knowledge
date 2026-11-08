# Local pilot recovery and handoff

Approved scope: continue release preparation locally. No second Mac, remote pilot server, paid resource, signing identity or publication is authorized by this step.

Use the existing authentication/recovery harnesses against new disposable infrastructure. Keep the normal HTTPS stack, original user credentials and database volumes intact. Correct reproduced failures; refresh the draft release notes and provide a repeatable local pilot protocol with explicit exit criteria.

- [x] Inspect the current candidate, release gates, backup/restore scripts and existing acceptance tools.
- [x] Reproduce current-schema encrypted backup/restore failures, fix their cause, and extend the real drill to cover restored personal-token invalidation.
- [x] Run the complete disposable restore drill and production-mode private-CA Keycloak acceptance; retain results and clean up only their resources.
- [x] Add the restore drill to the existing disposable PostgreSQL CI job so schema drift is caught automatically.
- [ ] Refresh alpha.4 draft notes and local pilot instructions around schema 21, current MCP onboarding, qualified candidate and known limits.
- [ ] Obtain focused review, run relevant checks and remote gates, record evidence, and commit/push the reviewable result without merging or publishing.

The real-team, hosted-provider, signing/notarization and physical clean-Mac gates remain open. Local synthetic drills do not close them.
