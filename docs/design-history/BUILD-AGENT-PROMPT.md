# Copyable build-agent prompt

Build the Workspace Knowledge system described in the attached technical specification and implementation plan. Read both documents before editing. Treat their privacy and authorization rules as acceptance requirements.

Work in a dedicated repository selected for this project. Inspect existing instructions and Git status; preserve unrelated work. If no repository has been selected, ask for its location before writing application code. Copy these documents into that repository's docs directory and keep relative links valid.

Implement sequentially starting with Tasks 1–7 and the synthetic vertical slice. Do not start by building a dashboard or scanning real conversations. Use synthetic fixtures, a disposable database and a deterministic fake summarizer for initial tests. Add real local inference and client integrations only after scope, retention and permission tests pass. Continue through subsequent tasks when authorized scope includes the full implementation.

The product must automatically capture supported work in developer-approved product/service folders and automatically publish eligible summaries under explicit team/developer policy. Do not impose per-turn manual approval for policy-auto mode. Keep uncertain, mixed or sensitive sessions private. Preserve original prompts and responses only in bounded encrypted local storage; never index or publish them.

Use MCP for cross-agent retrieval and client-specific documented hooks for capture. Preserve existing client configurations. Prove each client's version/platform behavior with an actual test; distinguish registration from connectivity and connectivity from capture. Read CONVERSATION-CAPTURE-ADDENDUM.md. Implement optional Enterprise Compliance API capture for explicitly bound Desktop chats, and instrumented workspace chat for controlled non-Enterprise capture. Respect the explicitly changed broker processing boundary. Do not claim universal transcript access or complete browser-observed history.

Share approved environment values through the separate vault/broker path. Never return secret values in knowledge tools, summaries, logs or embeddings. Enforce product/service/environment permissions on the server. A source repository manifest cannot grant access without local enrollment.

Run the tests and acceptance checks for each task. Record commands, outputs, measured performance and compatibility gaps. Do not publish a repository, install hooks into real user profiles, enroll real folders, deploy to production, or collect actual team conversations without authorization for that action. Ask only for genuinely missing decisions; routine implementation choices should follow the specification.
