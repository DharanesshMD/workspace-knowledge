# Future plan: evaluate Graphicle for symbol-level code context

**Status: parked, not scheduled.** Revisit after the production-supported launch gates close (see the [production readiness ledger](../../operations/production-readiness.md)). Nothing here is approved work.

Source assessed on 2026-10-19: [graphicle-ai/graphicle](https://github.com/graphicle-ai/graphicle), from its README and docs only. It was not installed, run, or code-reviewed.

## What it is

Apache-2.0 Rust program for a single developer (KarLex AI): a tree-sitter code graph (a Rust port of Graphify), project memory, session capture, and one MCP server for coding agents. Its team server uses access keys; a commercial team edition is in private preview. At assessment it was five days old, at v0.1.2, with 3 contributors.

## Why parked

- **No performance benefit for this app.** Its benchmarks measure graph-build speed against Graphify. Our measured limit is database CPU under search load, which it does not touch. Its own agent tests show a tie with grep at finding code and claim no token saving.
- **Immature dependency for a launch.** Five days old, one vendor, pre-1.0, and a source build downloads parser sources. That conflicts with the supply-chain and security-review gates.
- **Overlaps our privacy model.** `graphicle setup` edits `.mcp.json`, `AGENTS.md` and Claude/Codex hooks and turns on session capture that only redacts credentials. Our design keeps capture private until reviewed.

## What could be worth having

- **Symbol-level structure** (callers, imports, "what does changing this function touch"). Our change context works at file level.
- **Staleness tied to functions.** A reviewed card goes stale when the function it describes changes. Today we track source revisions, not symbols.

## Options, cheapest first

1. **Document coexistence (docs only).** Developers can run Graphicle graph-only next to our MCP server: `graphicle graph extract . --code-only`, connect with `--graph` and without `--hooks`, with its session capture and memory left off so there is one capture path.
2. **Spike (about a day, throwaway).** Build from source in a temp directory, run it graph-only on this repository, and compare what `graph affected` finds with `knowledge.change_context` output. Needs the owner's OK to download and build third-party code.
3. **Prototype symbol anchoring.** Add symbol extraction (tree-sitter directly, or the `graphicle-graph` crate) to anchor reviewed cards to functions and flag them stale when the function changes. Needs a dependency and security review first, and a schema and privacy design: nothing from unreviewed local code may reach the team server.

## Decision gates before option 3

- Launch gates closed and CI green at the final commit.
- Graphicle has a stable release history and a public security-reporting channel.
- The spike shows a measurable improvement for developers or agents over file-level context.
- Dependency audit, license and notice review pass; parser sources are vendored or pinned for reproducible builds.
