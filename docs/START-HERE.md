> Implementation status: see [repository README](../README.md) and [build ledger](PROGRESS.md). The original planning package below predates implementation.

# Workspace Knowledge: agent build package

This package translates the product discussion into an open-source technical specification and implementation sequence. Working name and license are proposals. No application code, real-agent capture, installations, or public repository have been created.

## Read in order

1. [Technical specification](docs/superpowers/specs/2026-09-30-workspace-knowledge-design.md)
2. [Implementation plan](docs/superpowers/plans/2026-09-30-workspace-knowledge.md)
3. [Build-agent kickoff prompt](BUILD-AGENT-PROMPT.md)

4. [Conversation capture and graph provenance revision](CONVERSATION-CAPTURE-ADDENDUM.md) — official Enterprise capture, instrumented chat, and new implementation tasks.

## Recommended first deliverable

Complete Milestone A: approved workspace → synthetic capture → private encrypted local queue → sanitized card → authenticated shared storage → permission-filtered MCP retrieval. Then prove real automation with Cursor and Claude Code before expanding client coverage.

## Decisions already carried into the plan

- Native Windows/macOS/Linux clients and self-hosted server.
- Approved product/service scope with no personal-folder traversal.
- Automatic policy-approved summary publication after one-time developer/team consent.
- MCP for retrieval, per-client hooks for collection, CLI fallback.
- Explicit partial/cooperative capture when a client lacks a reliable capture interface.
- Raw conversations stay private; environment secrets use a separate on-prem vault.
- Fast bounded context, versioned evidence, conflict handling and revocation.
- Open-source packaging, contribution/security policies and release certification.

Official client documentation was checked on 2026-09-30. Compatibility remains a build-and-test requirement, not a completed claim. The technical specification links the sources for each integration.
