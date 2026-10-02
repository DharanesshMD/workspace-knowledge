# Jev decision layer

Jev evaluates bounded semantic questions over already-published, authorized summaries. This implements the user's requested decision layer while preserving the on-premises privacy boundary. It is disabled until configured. No hosted API calls were made during development.

`POST /v1/products/{product}/decisions` accepts `{"card_ids":["UUID"]}` with one to four distinct active published cards. Both stdio and remote MCP expose this as `knowledge.assess`. The server fetches summaries through current membership checks; callers cannot inject raw conversations, private prompts, secrets, arbitrary questions or unapproved service context into this endpoint.

One request batches next-step routing and review priority for each summary, plus pairwise conflict/relationship suggestions. Questions use explicit summary indices because the API does not send question-map keys to the model. Results are advisory: Jev never grants access, publishes a card, verifies code or tests, modifies a graph relationship, changes capture scope, or releases secrets. Those decisions remain deterministic or explicitly reviewer-controlled. Code correctness and ambiguous cases require evidence and human review; no claim that a decision model replaces code/test verification is made.

The request timeout is two seconds, response/input limits are 64 KiB, concurrency is four calls, redirects and inherited proxies are disabled, and failures return a review-required outcome. No immediate automatic retries amplify an outage or add latency. Successful responses are cached in bounded process memory for five minutes using caller identity, membership policy, complete selected state, model and rubric inputs. Revisions and membership are rechecked before returning even cached results. No source text, prompts, credentials or provider response bodies are logged. Unknown labels, wrong types, missing answers, malformed probability distributions and unexpected model versions are rejected.

## Configuration and data residency

- `WK_JEV_ENDPOINT`: exact endpoint ending `/v1/systemone`; no default hosted URL.
- `WK_JEV_MODEL`: pinned model ID, default `jev-1.13.0`. Aliases resolving to another ID are rejected; pin a version and recalibrate before upgrades.
- `TYPESAFE_API_KEY`: optional local-service credential; required by the hosted provider. Never put it in CLI arguments, browser storage or cards.
- `WK_JEV_ALLOW_REMOTE=1`: explicit deployment approval for any non-loopback HTTPS endpoint. Keep unset until its data-residency/privacy approval is established. HTTP is accepted only for literal loopback.
- `WK_JEV_CALIBRATED_PROBABILITY`: optional locally validated maximum-label-probability threshold. Without it all outputs route to review. A threshold allows advisory routing only, never mutation or permission grants.

The cited quickstart documents a hosted service. A self-hosted Jev distribution/private deployment has not been established from those docs. The configurable endpoint supports an operator-provided compatible private service if available; it does not imply that model weights or a self-hosted license exist. The choice between a private deployment and approved hosted processing remains an open deployment decision.

## Paper interpretation and performance evidence

The supplied paper, *JEV-as-a-Judge: Accept When Confident, Escalate When Unsure* (arXiv:2609.26550v3), studies accepting confident decisions and escalating uncertain ones, and reports weaker performance where correctness must be derived or style is adversarial. Its confidence signal is maximum label probability. The API's separate `confidence` statistic is not assumed identical: responses preserve both as `max_probability` and `api_confidence`.

Use `scripts/jev-calibrate.py INPUT.jsonl OUTPUT.json` on independently labeled local data, one model/rubric/task at a time. Rows contain `split` (calibration/evaluation), `model`, `rubric_version`, `task`, `correct` (boolean), `max_probability`, and optional `latency_ms`. The script requires at least 100 rows per split, chooses a candidate threshold using a Wilson lower bound on calibration data, and reports held-out accuracy/coverage without auto-enabling anything. It never uploads samples. Operator review, representative data and evaluation of high-confidence mistakes are still necessary.

Batching and cache behavior are verified against synthetic HTTP responses. Actual Jev accuracy, latency, and speedup versus the existing local workflow are unmeasured until an approved endpoint and representative corpus are available. Keep lexical retrieval direct and fast; invoke semantic assessment when the developer/agent needs it rather than inserting a network judgment into every authorization or search operation.

Sources: [Quickstart](https://docs.typesafe.ai/introduction/quickstart), [API](https://docs.typesafe.ai/api), [Confidence](https://docs.typesafe.ai/confidence), [Models](https://docs.typesafe.ai/models). Reviewed 2026-10-02.
