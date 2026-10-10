# Native capture follow-up — 2026-10-10

The latest `wk-agent` and `wk-server` development binaries built successfully. Fourteen focused native agent regressions passed, including cancellation/restart replay, fixed expiry, session holds, changed enrollment, emitter identity, selective cleanup, state locking and fresh evidence validation after a policy wait. These are automated regression results, not a completed live restart/revocation matrix.

## Installed client drift

The installed `codex --version` now reports `0.162.0-alpha.17.2`. The native adapter's accepted version remains `0.162.0-alpha.2`. Actual enrollment rejected the updated executable with `native_enrollment_changed`; no capture or compatibility claim is made for the newer version. The successful October 9 handoff remains evidence only for the previously tested version. A new hook-format and trust probe is required before widening the version gate.

Antigravity still reports desktop version `2.19.1`. A new isolated synthetic workspace, consent binding and collector state were created under the existing plan workspace with the `oct10-` prefix. Native Antigravity enrollment and project-local hook installation succeeded. The collector doctor reported available, membership current, scope approved, zero received events and zero capture gaps. Installation alone does not establish hook delivery.

## Live Antigravity collector acceptance

After the user opened the exact synthetic `oct10-workspace/backend` folder, three real no-tool turns ran in Antigravity desktop 2.19.1, conversation `b0a507c0-3bb2-49ea-b448-82127b72ba31` (Synthetic Collector Acceptance Test). Each exact response was verified in the desktop UI.

1. `VIOLET-COLLECTOR-OCT10-ONE`: actual project hooks reached the collector through its normal emitter identity and authenticated callback checks. Doctor reported one held turn, `terminal_attribution_unverified`, and no capture gap. Processing created zero drafts and reported no publication.
2. The collector process was stopped and restarted using the same encrypted state. `VIOLET-COLLECTOR-OCT10-TWO` then completed in the same chat. Doctor reported two held turns with the same reason and no capture gap; no private drafts existed. The prior hold survived restart and the later callback was accepted.
3. `native-disable --client antigravity` cleared both retained turns. With the project hooks still installed, `VIOLET-COLLECTOR-OCT10-DISABLED` completed. Doctor after completion reported zero retained turns, zero last callback and no capture gap; drafts remained empty.

These results establish bounded live project-hook delivery, emitter acceptance, callback-bound private capture, retained holds across restart, and disabled-capture behavior. They do not establish reliable terminal attribution, eligible summarization/publication, file-tool coverage, or the complete outage/revocation race matrix. Antigravity publication remains blocked.

## Cleanup

The owned project hook registration was uninstalled. The isolated API and collector were stopped; synthetic Keychain items and private collector/consent state were removed. No global hooks were installed and no inference was requested. The user's Claude subscription login was preserved. Sanitized evidence is in the plan workspace's `oct10-antigravity-result.json`; the synthetic project and its chat remain available for inspection.
