# Model-free Summaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Developers can use Workspace Knowledge with no local model: `summarizer: rules` builds deterministic drafts, `wk-agent draft` stores assistant-written cards as private drafts, and `wk-agent prompt` prints the instructions that make an assistant write them. `local` remains available and is the default for existing setups.

**Architecture:** A `Summarizer` setting in `state::Config` (serde default `local`). One dispatcher, `rules_draft::extract`, replaces the four `wk_summarize::extract` call sites in `wk-agent` (the CLI's `cursor-history cards` evaluation tool is untouched): in `local` mode it calls the model as today; in `rules` mode it builds an `Extract` from the prompt/reply or note structure. Assistant drafts are stored with an `assistant` marker and submitted through a new `deliver` branch that mirrors the memory branch.

**Tech Stack:** Rust (clap, serde_json, sha2), existing encrypted local store, `wk_adapters::cursor_history::redact`, `wk_summarize::sensitive`.

**Spec:** `docs/superpowers/specs/2026-10-17-model-free-summaries-design.md`.

**Commands:** `.superpowers/sdd/2026-10-17-model-free/run.sh` wraps the pinned toolchain and a disposable PostgreSQL URL (copy it from `.superpowers/sdd/2026-10-15-history/run.sh`). Gate before each commit: `run.sh cargo fmt --all -- --check` and `run.sh cargo clippy --workspace --all-targets --locked -- -D warnings`. Commit with `git commit -s` and the trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

**Note for implementers:** the code below was written against the real APIs but not compiled. Read the surrounding code first, adapt minimally, keep the tested behaviour and report deviations.

---

## File structure

| File | Responsibility |
|---|---|
| Modify `crates/agent/src/state.rs` | `Summarizer` enum and `Config.summarizer` |
| Modify `crates/agent/src/main.rs` | `init` flags, `draft`, `prompt` commands, `doctor` line |
| Modify `crates/agent/src/device.rs` | `ConnectArgs` summarizer handling |
| Modify `crates/agent/src/chat.rs` | Managed chat requires `local` |
| Create `crates/agent/src/rules_draft.rs` | Rule builder and the `extract` dispatcher |
| Modify `crates/agent/src/process.rs`, `native_process.rs`, `memory_import.rs` | Use the dispatcher; rule drafts are partial; `assistant` branch in `deliver`; `list` rows |
| Create `crates/agent/src/assistant_draft.rs` | `draft` input validation, card building, store write |
| Create `crates/agent/src/assistant_prompt.rs` | Instruction texts for `prompt` |
| Modify docs, README, CHANGELOG, scripts | See Task 7 |

All 12 existing `state::Config { … }` literals must gain `summarizer`: `personal_sync.rs:215`, `device.rs:461` and `:580`, `memory_import.rs:536`, `native_enrollment.rs:258`, `main.rs:264`, `process.rs:610`, `cursor_source.rs:262`, `chat.rs:578`, `native_process.rs:159`, `repository.rs:451` (a `Config` in that module — check which type), plus `main.rs:139` (a different `Config`, likely the clap fragment — check).

---

### Task 1: Summarizer setting, init, connect and chat gate

**Files:** `state.rs`, `main.rs`, `device.rs`, `chat.rs`, and every `Config` literal listed above.

- [ ] **Step 1: Failing tests** in `state.rs` tests (create a `#[cfg(test)] mod tests` if absent):

```rust
#[test]
fn old_configs_without_a_summarizer_are_local() {
    let old = r#"{"root":"/w","binding_file":"/b.json","service":"backend","server":"http://127.0.0.1:4318","model":"m","inference":"http://127.0.0.1:11434/v1/chat/completions","port":4319,"account":"workspace","device_id":"d"}"#;
    let c: Config = serde_json::from_str(old).unwrap();
    assert_eq!(c.summarizer, Summarizer::Local);
}
#[test]
fn summarizer_choice_follows_the_flags() {
    use Summarizer::*;
    // (flag, inference given) -> (mode, inference to store)
    assert_eq!(resolve_summarizer(None, None).unwrap(), (Rules, String::new()));
    assert_eq!(resolve_summarizer(None, Some("http://127.0.0.1:11434/v1/chat/completions".into())).unwrap().0, Local);
    assert_eq!(resolve_summarizer(Some(Rules), None).unwrap().0, Rules);
    assert!(resolve_summarizer(Some(Local), None).is_err(), "local needs an inference URL");
    assert!(resolve_summarizer(Some(Rules), Some("http://127.0.0.1:1/x".into())).is_err(), "rules must not carry an inference URL");
}
```

And in `main.rs` `cli_tests`: `wk-agent --state-dir /s init --binding /b --service backend --server http://127.0.0.1:4318 --yes` parses with no `--inference`; `--summarizer bogus` fails to parse.

- [ ] **Step 2:** Run `run.sh cargo test -p wk-agent --locked summarizer` → compile errors.
- [ ] **Step 3: Implement.** In `state.rs`:

```rust
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Summarizer {
    /// Drafts come from a model on this machine.
    #[default]
    Local,
    /// Drafts are built from the conversation's structure; no model.
    Rules,
}
/// Pick the mode and the inference URL to store. Local needs a URL; rules must not have one.
pub fn resolve_summarizer(flag: Option<Summarizer>, inference: Option<String>) -> Result<(Summarizer, String), String> {
    match (flag, inference) {
        (None, None) | (Some(Summarizer::Rules), None) => Ok((Summarizer::Rules, String::new())),
        (None, Some(url)) | (Some(Summarizer::Local), Some(url)) => {
            wk_summarize::local_url(&url)?;
            Ok((Summarizer::Local, url))
        }
        (Some(Summarizer::Local), None) => Err("--summarizer local needs --inference".into()),
        (Some(Summarizer::Rules), Some(_)) => Err("--summarizer rules cannot be combined with --inference".into()),
    }
}
```

  and in `Config` add `#[serde(default)] pub summarizer: Summarizer,` (it derives `PartialEq, Eq`; `Summarizer` does too). Add `summarizer: Summarizer::Local` to every existing `Config` literal that is a test fixture (they pass a loopback inference URL); `main.rs:264` (init) and `device.rs:461` (connect) use the resolved value. Init flags: change `inference` to `Option<String>` (no default), `model` stays `default_value = "local-model"` (only meaningful for local; store `String::new()` for rules), add `#[arg(long, value_enum)] summarizer: Option<Summarizer>`; replace `wk_summarize::local_url(&inference)?;` with `let (summarizer, inference) = state::resolve_summarizer(summarizer, inference)?; let model = if summarizer == Summarizer::Local { model } else { String::new() };`. Same in `device::ConnectArgs` (fields `inference: Option<String>`, `summarizer: Option<Summarizer>`) and `connect`. Update the init `--yes` message to say "product, service and summarizer scope".
- [ ] **Step 4: Chat gate.** At the top of `chat::run` (before `local_url`): `if c.summarizer != state::Summarizer::Local { return Err("managed chat needs a local model; set up with --summarizer local --inference URL".into()); }` (read how `c` is loaded there). Test it with a rules `Config` fixture.
- [ ] **Step 5: `doctor`** adds `"summarizer": "local" | "rules"` to its output JSON. Test via a unit on a small helper or extend an existing doctor test if one exists; otherwise assert the helper.
- [ ] **Step 6: Scripts.** `scripts/cursor-native-integration.py:36` already passes `--inference`; add `'--summarizer','local'` there for clarity. `grep -rn "'connect'\|\"connect\"" scripts` and for any `device connect` call that relied on the default inference URL, add `--inference <url>` (or `--summarizer rules` if the script never uses a model); report each change.
- [ ] **Step 7:** `run.sh cargo test -p wk-agent --locked`, gate, commit `feat: make the summarizer a setting so a local model is optional`.

---

### Task 2: Rule builder and dispatcher

**Files:** create `crates/agent/src/rules_draft.rs`; `mod rules_draft;` in `main.rs` (with `#[allow(dead_code)] // Wired in by the call-site task.` until Task 3).

- [ ] **Step 1: Failing tests** (in the new file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn exchange(p: &str, r: &str) -> Shape<'static> {
        Shape::Exchange { prompt: Box::leak(p.to_string().into_boxed_str()), reply: Box::leak(r.to_string().into_boxed_str()) }
    }
    #[test]
    fn exchange_uses_first_line_prompt_and_reply() {
        let e = from_shape(&exchange("Make retries idempotent\nso payments are not repeated", "Added an idempotency key and capped retries at 3.")).unwrap();
        assert_eq!(e.title, "[auto-drafted] Make retries idempotent");
        assert!(e.intent.starts_with("Make retries idempotent"));
        assert_eq!(e.rationale, "Added an idempotency key and capped retries at 3.");
        assert!(e.decisions.is_empty() && e.constraints.is_empty() && e.open_questions.is_empty() && !e.sensitive);
    }
    #[test]
    fn long_text_is_clipped_on_character_boundaries() {
        let long = "é".repeat(5000);
        let e = from_shape(&exchange(&long, &long)).unwrap();
        assert!(e.intent.chars().count() <= INTENT_MAX + 1 && e.rationale.chars().count() <= RATIONALE_MAX + 1);
        assert!(e.title.chars().count() <= 256);
    }
    #[test]
    fn note_uses_heading_and_body() {
        let e = from_shape(&Shape::Note { title: "Retries", body: "Retries are capped at 3 because the gateway repeats charges." }).unwrap();
        assert_eq!(e.title, "[auto-drafted] Retries");
        assert!(e.rationale.contains("capped at 3"));
    }
    #[test]
    fn credentials_are_masked_and_sensitive_sources_are_held() {
        let e = from_shape(&exchange("set password=hunter2 in the config", "done")).unwrap();
        assert!(!e.intent.contains("hunter2"), "masked: {}", e.intent);
        assert!(from_shape(&exchange("here is -----BEGIN PRIVATE KEY----- abc", "ok")).is_err(), "sensitive source is held");
    }
    #[test]
    fn empty_prompts_and_notes_are_held() {
        assert!(from_shape(&exchange("  ", "reply")).is_err());
        assert!(from_shape(&Shape::Note { title: "T", body: " " }).is_err());
    }
}
```

- [ ] **Step 2:** run `run.sh cargo test -p wk-agent --locked rules_draft` → compile errors.
- [ ] **Step 3: Implement.**

```rust
//! Deterministic drafts for `summarizer: rules` and the single call-site dispatcher.
use crate::state::{Config, Summarizer};
use wk_summarize::Extract;

pub(crate) const INTENT_MAX: usize = 400;
pub(crate) const RATIONALE_MAX: usize = 1500;
const TITLE_MAX: usize = 200;
pub(crate) const MARK: &str = "[auto-drafted] ";

/// What a draft is built from. Borrowed text is the visible conversation or note only.
pub(crate) enum Shape<'a> {
    /// First prompt and last reply of a turn or part.
    Exchange { prompt: &'a str, reply: &'a str },
    /// A memory note or instruction section.
    Note { title: &'a str, body: &'a str },
}
fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max { return s.to_string(); }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}
fn first_line(s: &str) -> &str {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}
fn clean(s: &str, max: usize) -> String { clip(&wk_adapters::cursor_history::redact(s), max) }

/// Build an `Extract` without a model. Errors hold the item (nothing to quote, or sensitive).
pub(crate) fn from_shape(shape: &Shape<'_>) -> Result<Extract, String> {
    let (title, intent, rationale, whole) = match shape {
        Shape::Exchange { prompt, reply } => (first_line(prompt), prompt.trim(), reply.trim(), format!("{prompt}\n{reply}")),
        Shape::Note { title, body } => (title.trim(), body.trim(), body.trim(), format!("{title}\n{body}")),
    };
    if title.is_empty() || intent.is_empty() { return Err("nothing to draft".into()); }
    if wk_summarize::sensitive(&whole) { return Err("source requires private review".into()); }
    let (intent, rationale) = match shape {
        // A note's text is the rationale; its intent is its heading.
        Shape::Note { .. } => (clean(title, INTENT_MAX), clean(rationale, RATIONALE_MAX)),
        Shape::Exchange { .. } => (clean(intent, INTENT_MAX), clean(rationale, RATIONALE_MAX)),
    };
    let title = format!("{MARK}{}", clean(title, TITLE_MAX));
    Ok(Extract { title, intent, rationale, decisions: vec![], constraints: vec![], open_questions: vec![], sensitive: false })
}

/// The one place a draft's text is produced. `source` is the prompt text the model path expects.
pub(crate) async fn extract(c: &Config, shape: &Shape<'_>, source: &str) -> Result<Extract, String> {
    match c.summarizer {
        Summarizer::Local => wk_summarize::extract(&c.inference, &c.model, source).await,
        Summarizer::Rules => from_shape(shape),
    }
}
/// Rule drafts are never complete: they quote structure, not reasoning.
pub(crate) fn complete(c: &Config, complete: bool) -> bool { complete && c.summarizer == Summarizer::Local }
```

  (Fix the test's `Shape::Exchange` helper if `Box::leak` is awkward: tests may build owned strings and pass `&str` directly.)
- [ ] **Step 4:** tests pass; fix any failing masking expectation by checking `redact`'s actual output for `password=hunter2` (it must not contain `hunter2`).
- [ ] **Step 5:** gate, commit `feat: build deterministic drafts without a model`.

---

### Task 3: Use the dispatcher at the call sites

**Files:** `process.rs` (journal turns ~line 125, native turns ~line 416), `native_process.rs` (~line 112), `memory_import.rs` (~line 397); tests alongside.

- [ ] **Step 1: Failing tests.** For each site, a test with `summarizer: Rules` and an `inference` pointing at a loopback server that counts hits and fails the test if called:
  - `journal_turns` (use the existing loopback fixture pattern in `process.rs` / `chat.rs` tests): record a 2-exchange session, run `journal_turns` with a rules `Config` → exactly 1 draft per turn, title starts with `[auto-drafted] `, `capture_completeness` is `partial`, and the counting server saw 0 requests.
  - `memory_import::draft_all`: import a note, run `draft_all` with rules → drafts created without a model request.
  - native Cursor/Codex turn sites: use the existing fixtures in `native_process.rs` / `process.rs` tests; assert the same.
  Add one shared test helper `fn rules_config(mut c: Config) -> Config { c.summarizer = Summarizer::Rules; c.inference = String::new(); c.model = String::new(); c }`.
- [ ] **Step 2:** run them → they fail (the model path is still used).
- [ ] **Step 3: Implement.** Replace each call:
  - `process.rs` journal turns: build `Shape::Exchange { prompt: first user_prompt text in the turn, reply: last assistant_response text }` from `events` (these are `serde_json::Value` events with `kind` and `text`); `match crate::rules_draft::extract(c, &shape, &source).await {`. Use `crate::rules_draft::complete(c, complete)` where `complete` is passed to `wk_summarize::card`.
  - `process.rs` Cursor native turns and `native_process.rs`: `Shape::Exchange { prompt: &turn.prompt, reply: &turn.response }`; keep the `budget.attempt(...)` wrapper around the dispatcher future; replace the `true` passed to `card(...)` with `rules_draft::complete(c, true)`.
  - `memory_import.rs` `draft_all`: `Shape::Note { title: <heading path joined by " > ", or label if empty>, body: &record.body }` (the hint, if any, is prepended to the body as `"{hint}\n{body}"` only in rules mode) and `complete = rules_draft::complete(c, true)`.
  - Remove the `#[allow(dead_code)]` on `mod rules_draft;`.
- [ ] **Step 4:** run the new tests and the whole agent suite (`run.sh cargo test -p wk-agent --locked`); existing model-path tests must be unchanged (their fixtures use `Summarizer::Local`).
- [ ] **Step 5:** gate, commit `feat: draft without a model when the summarizer is rules`.

---

### Task 4: `wk-agent draft` and the assistant branch in `deliver`

**Files:** create `crates/agent/src/assistant_draft.rs`; `main.rs` (`Draft` command); `process.rs` (`deliver` branch, `list` row via `memory_import::draft_item`).

- [ ] **Step 1: Failing tests** in `assistant_draft.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    const OK: &str = r#"{"title":"Cap retries at 3","intent":"Avoid repeated charges","rationale":"The gateway charges again on duplicate calls; an idempotency key plus a cap keeps retries safe.","decisions":["Retries capped at 3"],"constraints":["Gateway repeats charges"],"open_questions":[]}"#;
    #[test]
    fn builds_a_partial_proposed_card_with_a_stable_key() {
        let (key, draft) = build(OK, "backend", 7, "hash1", "claude-code").unwrap();
        assert!(key.starts_with("draft:") && key.len() == 70);
        assert_eq!(draft["card"]["capture_completeness"], "partial");
        assert_eq!(draft["card"]["lifecycle"], "proposed");
        assert_eq!(draft["card"]["service_ids"], serde_json::json!(["backend"]));
        assert_eq!(draft["card"]["policy_revision"], 7);
        assert_eq!(draft["assistant"]["client"], "claude-code");
        assert_eq!(draft["model"], "assistant");
        assert_eq!(build(OK, "backend", 7, "hash1", "claude-code").unwrap().0, key, "same card, same id");
    }
    #[test]
    fn strict_fields_and_size_limits() {
        assert!(build(r#"{"title":"t","intent":"i","rationale":"r","decisions":[],"constraints":[],"open_questions":[],"extra":1}"#, "b", 1, "h", "x").is_err());
        assert!(build("{}", "b", 1, "h", "x").is_err());
        assert!(build(&"x".repeat(65 * 1024), "b", 1, "h", "x").is_err());
    }
    #[test]
    fn sensitive_cards_are_refused_without_echo() {
        let bad = OK.replace("Avoid repeated charges", "key sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789");
        let err = build(&bad, "backend", 1, "h", "x").unwrap_err();
        assert_eq!(err, "card requires private review (possible credential)");
        assert!(!err.contains("sk-proj"));
    }
}
```

  plus a store test: `store_draft(&mut store, &key, &draft, now)` writes with a 24-hour ttl and `read_record` returns it; a second identical write is a no-op (`store.update` keeps the old).
- [ ] **Step 2:** run → compile errors.
- [ ] **Step 3: Implement** `assistant_draft.rs`:

```rust
//! `wk-agent draft`: a card written by the developer's own assistant, kept as a private local draft.
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use wk_contracts::{CardSubmission, Completeness, Lifecycle};

const MAX_INPUT: usize = 64 * 1024;
pub(crate) const TTL: i64 = 86_400; // the local store's maximum

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    title: String,
    intent: String,
    rationale: String,
    decisions: Vec<String>,
    constraints: Vec<String>,
    open_questions: Vec<String>,
}
/// Validate the assistant's JSON and build the private draft record: `(draft key, record)`.
pub(crate) fn build(text: &str, service: &str, revision: u64, policy_hash: &str, client: &str) -> Result<(String, Value), String> {
    if text.len() > MAX_INPUT { return Err("card input too large".into()); }
    let i: Input = serde_json::from_str(text).map_err(|_| "invalid card JSON; expected title, intent, rationale, decisions, constraints, open_questions")?;
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&(&i.title, &i.intent, &i.rationale, &i.decisions, &i.constraints, &i.open_questions, service, policy_hash, revision, "assistant-v1")).map_err(|_| "invalid card")?));
    let card = CardSubmission {
        schema_version: 1,
        idempotency_key: digest.clone(),
        policy_revision: revision,
        title: i.title,
        intent: i.intent,
        rationale: i.rationale,
        decisions: wk_summarize::tidy(i.decisions),
        constraints: wk_summarize::tidy(i.constraints),
        open_questions: wk_summarize::tidy(i.open_questions),
        service_ids: vec![service.to_string()],
        evidence: vec![],
        lifecycle: Lifecycle::Proposed,
        capture_completeness: Completeness::Partial,
        base_revision: None,
        observed_at_ms: None,
    };
    card.validate()?;
    if wk_summarize::sensitive(&serde_json::to_string(&card).map_err(|_| "invalid card")?) {
        return Err("card requires private review (possible credential)".into());
    }
    let record = json!({"card": card, "assistant": {"client": client}, "policy_hash": policy_hash, "model": "assistant"});
    Ok((format!("draft:{digest}"), record))
}
/// Write the draft once; an identical card keeps its first copy.
pub(crate) fn store_draft(store: &mut wk_local::LocalStore, key: &str, record: &Value, now: i64) -> Result<(), String> {
    let bytes = serde_json::to_vec(record).map_err(|_| "invalid draft")?;
    store.update(key, now, TTL, |old| Ok(old.map_or(bytes.clone(), |b| b.to_vec())))
}
/// `wk-agent draft`: read stdin, check approval and policy, store a private draft.
pub(crate) async fn run(dir: &std::path::Path, client: &str, input: &str) -> Result<Value, String> {
    let c = crate::state::load(dir)?;
    let b = crate::state::binding(&c)?;
    let token = crate::state::token(dir, "server-token")?;
    let policy = crate::network::policy(&c, &b, &token).await?;
    let revision = policy["policy_revision"].as_u64().ok_or("invalid policy")?;
    let (key, record) = build(input, &c.service, revision, &b.policy_hash, client)?;
    let mut store = crate::state::store(dir)?;
    store_draft(&mut store, &key, &record, crate::daemon::now()?)?;
    Ok(json!({"draft_id": key, "private": true, "published": false, "next": "review with `wk-agent drafts`, then `wk-agent submit --id ID --reviewed`"}))
}
```

  (If `wk_summarize::tidy` is not `pub`, it is: `pub fn tidy`. If the card's `title` exceeds 256, `validate()` rejects it.)
- [ ] **Step 4: CLI.** In `main.rs` add `mod assistant_draft;` and
```rust
/// Store a card written by your assistant as a private draft (reads JSON on standard input).
Draft {
    /// Which assistant wrote it (label only): claude-code, copilot, codex or other.
    #[arg(long, default_value = "other")]
    client: String,
},
```
  dispatch: `Command::Draft { client } => { let text = stdin_limited()?; assistant_draft::run(&state, &client, &text).await }` (`stdin_limited` exists, limit 256 KiB; the 64 KiB check still applies). Validate `client` against `[a-z0-9-]{1,32}` and return an error otherwise. CLI test: `Command::Draft` parses with and without `--client`.
- [ ] **Step 5: `deliver` branch** in `process.rs`, directly after the `memory` branch:

```rust
    if v.get("assistant").is_some() {
        let revision = p["policy_revision"].as_u64().ok_or("invalid policy")?;
        if v["policy_hash"] != b.policy_hash || v["card"]["policy_revision"] != revision {
            s.ack(id)?; // a draft for an older approval can never become valid
            return Err("policy changed; ask your assistant to draft again".into());
        }
        if wk_summarize::sensitive(&v["card"].to_string()) {
            return Err("draft requires sensitive-content review".into());
        }
        let response = post_draft(dir, &c, &b, &t, &v, owner_only).await?;
        if !owner_only { s.ack(id)?; }
        return Ok(response);
    }
```
  Test the decision logic through a small pure helper extracted from this branch (`fn assistant_ready(v, policy_hash, revision) -> Result<(), AssistantHold>`) because `deliver` needs the keyring: policy mismatch → `Consume`, sensitive → `Hold`, ok → `Ready`; unit-test all three.
- [ ] **Step 6: `list` rows.** In `memory_import::draft_item`, add `item["assistant"] = json!({"client": v["assistant"]["client"]})` when `v.get("assistant").is_some()`; test.
- [ ] **Step 7:** run agent tests, gate, commit `feat: store assistant-written cards as private drafts`.

---

### Task 5: `wk-agent prompt`

**Files:** create `crates/agent/src/assistant_prompt.rs`; `main.rs`.

- [ ] **Step 1: Failing tests:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_client_text_names_the_command_the_fields_and_the_rules() {
        for client in ["claude-code", "copilot", "codex"] {
            let t = text(client, "/usr/local/bin/wk-agent", "/private/state").unwrap();
            assert!(t.contains("/usr/local/bin/wk-agent --state-dir /private/state draft --client"), "{client}");
            for f in ["title", "intent", "rationale", "decisions", "constraints", "open_questions"] { assert!(t.contains(f), "{client} {f}"); }
            assert!(t.to_lowercase().contains("no secrets") || t.to_lowercase().contains("never include"), "{client}");
        }
        assert!(text("claude-code", "/x", "/y").unwrap().starts_with("---\ndescription:"), "slash command frontmatter");
        assert!(text("copilot", "/x", "/y").unwrap().starts_with("---\nmode: agent\n"), "prompt file frontmatter");
        assert!(text("vim", "/x", "/y").is_err());
    }
    #[test]
    fn paths_with_spaces_or_shell_characters_are_quoted() {
        let t = text("codex", "/opt/my tools/wk-agent", "/p/state dir").unwrap();
        assert!(t.contains("'/opt/my tools/wk-agent' --state-dir '/p/state dir'"));
    }
}
```

- [ ] **Step 2:** run → compile errors.
- [ ] **Step 3: Implement.**

```rust
//! Instruction texts that make a developer's assistant write a card for `wk-agent draft`.
fn quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:@%+=".contains(c)) { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}
const BODY: &str = "Summarise this working session for my team as one knowledge card. Use only what actually happened in this session.\n\nWrite a JSON object with exactly these fields:\n- title: short, specific (under 120 characters)\n- intent: what we were trying to achieve, 1-2 sentences\n- rationale: why the code ended up this way, including approaches that failed\n- decisions: array of short strings\n- constraints: array of short strings (things a future change must respect)\n- open_questions: array of short strings\n\nRules: never include secrets, credentials, tokens, personal data or customer data; do not paste code, file contents or raw conversation text; describe in your own words. Leave an array empty rather than guessing.\n\nThen run this command, giving the JSON on standard input, and show me the result:\n\n    {CMD} draft --client {CLIENT}\n\nThe draft stays private on my machine until I review and submit it. Do not publish anything.\n";
pub(crate) fn text(client: &str, exe: &str, state: &str) -> Result<String, String> {
    let cmd = format!("{} --state-dir {}", quote(exe), quote(state));
    let body = BODY.replace("{CMD}", &cmd).replace("{CLIENT}", client);
    match client {
        "claude-code" => Ok(format!("---\ndescription: Summarise this session as a private Workspace Knowledge draft\n---\n{body}")),
        "copilot" => Ok(format!("---\nmode: agent\ndescription: Summarise this session as a private Workspace Knowledge draft\n---\n{body}")),
        "codex" => Ok(body),
        _ => Err("client must be claude-code, copilot or codex".into()),
    }
}
```

  CLI: `Prompt { #[arg(long)] client: String }` printing the text to stdout; because main prints JSON for results, return `Ok(json!({"client":client,"text":…,"save_as": "…"}))` with a `save_as` hint (`.claude/commands/wk-summarize.md`, `.github/prompts/wk-summarize.prompt.md`, or "add to AGENTS.md") rather than raw text. Test the JSON shape. The command writes no files.
- [ ] **Step 4:** run tests, gate, commit `feat: print assistant instructions for private draft summaries`.

---

### Task 6: End-to-end test

**Files:** a test in `crates/agent/src/process.rs` tests (or a new `crates/agent/tests` file if integration tests can reach the needed functions; otherwise an in-crate test).

- [ ] **Step 1:** With a rules `Config` and no model server running: record a synthetic session through `chat::import_into` (see `chat.rs` tests), run `journal_turns` → one `[auto-drafted]` draft per part; build an assistant draft through `assistant_draft::build` + `store_draft`; list both via `process::list`; assert the rows carry `assistant`/`model` markers and no raw prompt text is in `list` output beyond the card; `assistant_ready` logic accepts the fresh draft. (`submit` itself needs a server and keyring; its logic is covered by Task 4's helper test.)
- [ ] **Step 2:** commit `test: cover model-free drafting end to end`.

---

### Task 7: Documentation and scripts

- [ ] `docs/adapters/setup.md`: model-free setup (`init` without `--inference`), `wk-agent prompt --client` and where to save it per client, how `/wk-summarize` works, rule drafts and their limits (quote the conversation; review and edit; no files for Claude Code/Codex/Copilot imports), choosing `--summarizer local` for automatic drafts; managed chat needs `local`. Update every example that passes `--inference` to say it is optional.
- [ ] README quickstart and features: model optional; privacy wording stays accurate (describe the assistant path honestly). CHANGELOG bullet(s) including the `init`/`device connect` behaviour change (no `--inference` now means `rules`).
- [ ] `docs/architecture` pages that say a local model is required: grep for "local model", "inference" and update.
- [ ] Spec check: confirm the spec matches what was built; note deviations.
- [ ] Link check and `python3 -m unittest discover -s scripts/tests`; commit `docs: document setup without a local model`.

---

### Task 8: Verification

- [ ] fmt, workspace clippy `-D warnings`, `cargo test --workspace --locked -- --include-ignored` (disposable pgvector DB), build, Python tests, demo, plain-PostgreSQL acceptance, integration, code-context check, cargo and npm audits, console tests, full-history secret scan (note rule-draft tests use synthetic credential-shaped strings: build them at runtime or allowlist exact lines per the repo precedent).
- [ ] Manual smoke test in a disposable state directory: `wk-agent init` without `--inference` against a test server; `process` produces `[auto-drafted]` drafts from a synthetic session; `draft` stores an assistant card; `drafts` lists both; `prompt --client claude-code` prints the text. No model process running at any point.
