//! Deterministic drafts for `summarizer: rules` and the single call-site dispatcher.
use crate::state::{Config, Summarizer};
use wk_summarize::Extract;

pub(crate) const INTENT_MAX: usize = 400;
pub(crate) const RATIONALE_MAX: usize = 1500;
/// `CardSubmission::validate` limits titles to 256 UTF-8 bytes, marker included.
const TITLE_BYTES: usize = 256 - MARK.len();
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
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Clip to `TITLE_BYTES` bytes at a character boundary, ending in `…` when clipped.
fn clip_title(s: &str) -> String {
    let s = s.trim();
    if s.len() <= TITLE_BYTES {
        return s.to_string();
    }
    let mut end = TITLE_BYTES - '…'.len_utf8();
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn first_line(s: &str) -> &str {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

fn clean(s: &str, max: usize) -> String {
    clip(&wk_adapters::cursor_history::redact(s), max)
}

/// The words of `line` (letters, digits and apostrophes), lower-cased.
fn words(line: &str) -> Vec<String> {
    line.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// True when the line has words and every one is in `allowed` (digits always count).
fn only(line: &str, allowed: &[&str]) -> bool {
    let w = words(line);
    !w.is_empty()
        && w.iter()
            .all(|x| allowed.contains(&x.as_str()) || x.chars().all(|c| c.is_ascii_digit()))
}

const GREETING_WORDS: &[&str] = &[
    "hi",
    "hey",
    "hello",
    "howdy",
    "there",
    "team",
    "all",
    "everyone",
    "claude",
    "hope",
    "you",
    "you're",
    "youre",
    "are",
    "doing",
    "having",
    "well",
    "a",
    "good",
    "great",
    "week",
    "day",
    "morning",
    "afternoon",
    "evening",
    "weekend",
    "quick",
    "one",
    "question",
    "for",
    "thanks",
    "thank",
    "so",
    "much",
    "just",
    "i",
    "have",
    "how",
    "is",
    "it",
    "going",
];

const CONFIRMATION_WORDS: &[&str] = &[
    "yes", "yeah", "yep", "yup", "ok", "okay", "sure", "continue", "go", "ahead", "proceed", "do",
    "it", "please", "thanks", "thank", "you", "great", "perfect", "good", "sounds", "lgtm",
    "option", "approach", "with", "the", "that", "this", "one", "and", "looks", "ship", "correct",
    "right", "fine", "first", "second", "a", "b", "then",
];

const FILLER_SENTENCES: &[&str] = &[
    "done",
    "sure",
    "ok",
    "okay",
    "all",
    "set",
    "great",
    "certainly",
    "absolutely",
    "here",
    "you",
    "go",
    "finished",
    "complete",
    "completed",
    "perfect",
    "got",
    "it",
    "will",
    "do",
    "yes",
    "thanks",
];

/// A line that is pasted material rather than the developer's words: indented code, a stack
/// frame or the first line of an error report.
fn looks_pasted(line: &str) -> bool {
    let t = line.trim();
    let first = t
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(':');
    line.starts_with(char::is_whitespace)
        || t.starts_with("Traceback")
        || t.starts_with("File \"")
        || t.starts_with("at ")
        || t.starts_with("$ ")
        || t.to_lowercase().starts_with("error:")
        || t.to_lowercase().starts_with("error[")
        || first.ends_with("Error")
        || first.ends_with("Exception")
}

/// The prompt from the line where the request starts: leading blank lines, greetings, code
/// fences and pasted traces are skipped. A prompt with nothing else is kept whole.
fn request(prompt: &str) -> &str {
    let p = prompt.trim();
    let (mut at, mut fenced) = (0, false);
    for line in p.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced && !t.is_empty() && !looks_pasted(line) && !only(t, GREETING_WORDS) {
            return &p[at..];
        }
        at += line.len();
    }
    p
}

/// A short reply to the agent ("ok do it", "continue") that says nothing about the work.
fn is_confirmation(prompt: &str) -> bool {
    words(prompt).len() <= 6 && only(prompt, CONFIRMATION_WORDS)
}

/// The first sentence of `reply` that says something, with where it starts.
fn headline(reply: &str) -> Option<(usize, &str)> {
    let mut at = 0;
    for line in reply.split_inclusive('\n') {
        let body = line.trim_start_matches(|c: char| "#*->_ \t".contains(c));
        let start = at + (line.len() - body.len());
        let mut from = 0;
        while from < body.trim_end().len() {
            let rest = &body[from..];
            let end = rest
                .match_indices(['.', '!', '?'])
                .find(|(i, _)| {
                    rest[i + 1..].starts_with(char::is_whitespace) || rest[i + 1..].is_empty()
                })
                .map_or(rest.trim_end().len(), |(i, _)| i + 1);
            let sentence = rest[..end].trim().trim_matches(|c| c == '*' || c == '_');
            if !sentence.is_empty() && !only(sentence, FILLER_SENTENCES) {
                return Some((start + from, sentence));
            }
            from += end;
            from += rest[end..].len() - rest[end..].trim_start().len();
        }
        at += line.len();
    }
    None
}

const CLOSERS: &[&str] = &[
    "let me know",
    "hope this helps",
    "happy to help",
    "feel free to",
    "would you like me to",
    "want me to",
    "shall i ",
    "i can also",
];
const SUMMARY_MARKERS: &[&str] = &[
    "in summary",
    "summary",
    "tl;dr",
    "tldr",
    "to summarize",
    "in short",
    "what changed",
];

/// The part of a reply worth quoting as the outcome: courtesies at its end are dropped, and a
/// reply too long to quote starts at its summary paragraph when it has one.
fn outcome(reply: &str) -> String {
    let mut paragraphs: Vec<&str> = reply.trim().split("\n\n").collect();
    let courtesy = |p: &str| {
        let lower = p.to_lowercase();
        p.chars().count() < 300 && CLOSERS.iter().any(|c| lower.contains(c))
    };
    while paragraphs.len() > 1 && paragraphs.last().is_some_and(|p| courtesy(p)) {
        paragraphs.pop();
    }
    let text = paragraphs.join("\n\n");
    if text.chars().count() <= RATIONALE_MAX {
        return text;
    }
    let marked = paragraphs.iter().position(|p| {
        let lead = p
            .trim_start_matches(|c: char| "#*_ ".contains(c))
            .to_lowercase();
        SUMMARY_MARKERS.iter().any(|m| lead.starts_with(m))
    });
    match marked {
        Some(i) => paragraphs[i..]
            .join("\n\n")
            .trim_start_matches(['#', ' '])
            .to_string(),
        None => text,
    }
}

/// Build an `Extract` without a model. Errors hold the item (nothing to quote, or sensitive).
pub(crate) fn from_shape(shape: &Shape<'_>) -> Result<Extract, String> {
    let (title, intent, rationale, whole) = match shape {
        Shape::Exchange { prompt, reply } => {
            // A bare "ok do it" says nothing about the work, so the reply names it instead.
            let said = headline(reply.trim()).filter(|_| is_confirmation(prompt));
            let (title, wording) = match said {
                Some((at, sentence)) => (sentence, &reply.trim()[at..]),
                None => (first_line(request(prompt)), request(prompt)),
            };
            (
                title,
                clean(wording, INTENT_MAX),
                clean(&outcome(reply), RATIONALE_MAX),
                format!("{prompt}\n{reply}"),
            )
        }
        // A note's heading is its intent and its text the rationale (matches the plan).
        Shape::Note { title, body } => (
            first_line(title),
            clean(first_line(title), INTENT_MAX),
            clean(body, RATIONALE_MAX),
            format!("{title}\n{body}"),
        ),
    };
    let empty = match shape {
        Shape::Exchange { .. } => false,
        Shape::Note { body, .. } => body.trim().is_empty(),
    };
    if title.is_empty() || intent.is_empty() || empty {
        return Err("nothing to draft".into());
    }
    if wk_summarize::sensitive(&whole) {
        return Err("source requires private review".into());
    }
    Ok(Extract {
        title: format!(
            "{MARK}{}",
            clip_title(&wk_adapters::cursor_history::redact(title))
        ),
        intent,
        rationale,
        decisions: vec![],
        constraints: vec![],
        open_questions: vec![],
        sensitive: false,
    })
}

/// The one place a draft's text is produced. `source` is the prompt text the model path expects.
pub(crate) async fn extract(
    c: &Config,
    shape: &Shape<'_>,
    source: &str,
) -> Result<Extract, String> {
    match c.summarizer {
        Summarizer::Local => wk_summarize::extract(&c.inference, &c.model, source).await,
        Summarizer::Rules => from_shape(shape),
    }
}

/// Rule drafts are never complete: they quote structure, not reasoning.
pub(crate) fn complete(c: &Config, complete: bool) -> bool {
    complete && c.summarizer == Summarizer::Local
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange<'a>(prompt: &'a str, reply: &'a str) -> Shape<'a> {
        Shape::Exchange { prompt, reply }
    }

    fn config(summarizer: Summarizer, inference: &str) -> Config {
        Config {
            root: "/w".into(),
            binding_file: "/b.json".into(),
            service: "backend".into(),
            server: "http://127.0.0.1:4318".into(),
            model: "m".into(),
            inference: inference.into(),
            summarizer,
            port: 4319,
            account: "workspace".into(),
            device_id: "d".into(),
            repository: None,
            device: None,
        }
    }

    #[test]
    fn exchange_uses_first_line_prompt_and_reply() {
        let e = from_shape(&exchange(
            "Make retries idempotent\nso payments are not repeated",
            "Added an idempotency key and capped retries at 3.",
        ))
        .unwrap();
        assert_eq!(e.title, "[auto-drafted] Make retries idempotent");
        assert!(e.intent.starts_with("Make retries idempotent"));
        assert_eq!(
            e.rationale,
            "Added an idempotency key and capped retries at 3."
        );
        assert!(
            e.decisions.is_empty()
                && e.constraints.is_empty()
                && e.open_questions.is_empty()
                && !e.sensitive
        );
    }

    /// A card built from the extract passes the contract's own validation.
    fn validates(e: Extract) {
        wk_summarize::card(e, "key".into(), 1, vec!["backend".into()], false).unwrap();
    }

    #[test]
    fn long_text_is_clipped_on_character_boundaries() {
        let long = "é".repeat(5000);
        let e = from_shape(&exchange(&long, &long)).unwrap();
        assert_eq!(e.intent.chars().count(), INTENT_MAX + 1);
        assert_eq!(e.rationale.chars().count(), RATIONALE_MAX + 1);
        assert!(e.intent.ends_with('…') && e.rationale.ends_with('…'));
        assert!(e.title.starts_with(MARK) && e.title.ends_with('…'));
    }

    #[test]
    fn titles_fit_the_contract_limit_in_bytes_for_any_script() {
        let samples = [
            "é".repeat(5000),
            "数据库迁移需要幂等 ".repeat(100),
            "Сделать повторы идемпотентными ".repeat(50),
            "🚀 ship it ".repeat(100),
            "retry ".repeat(300),
        ];
        for text in &samples {
            let e = from_shape(&exchange(text, "ok")).unwrap();
            assert!(e.title.starts_with(MARK));
            assert!(e.title.len() <= 256, "{} bytes", e.title.len());
            assert!(e.title.ends_with('…'));
            validates(e);
        }
        let e = from_shape(&Shape::Note {
            title: &samples[1],
            body: "body text",
        })
        .unwrap();
        assert!(e.title.len() <= 256);
        validates(e);
        // Text that already fits is kept whole, without a marker.
        let e = from_shape(&exchange("Short title", "ok")).unwrap();
        assert_eq!(e.title, "[auto-drafted] Short title");
        validates(e);
    }

    #[test]
    fn note_headings_use_only_their_first_line() {
        let e = from_shape(&Shape::Note {
            title: "\nRetries\nsecond line",
            body: "Retries are capped at 3.",
        })
        .unwrap();
        assert_eq!(e.title, "[auto-drafted] Retries");
        assert_eq!(e.intent, "Retries");
    }

    #[test]
    fn credentials_straddling_a_clip_boundary_are_fully_masked() {
        let secret = ["qzvj", "wxmk93"].concat();
        let filler = |n: usize| "abcd ".repeat(n / 5);
        let cases = [
            // Straddles the 400-character intent clip (and is past the title budget).
            (
                format!("{}token={secret} tail", filler(390)),
                "ok".to_string(),
            ),
            // Straddles the title's byte budget as well as the intent.
            (
                format!("{}token={secret} tail", filler(230)),
                "ok".to_string(),
            ),
            // Straddles the 1500-character rationale clip.
            (
                "Add a limiter".to_string(),
                format!("{}token={secret} tail", filler(1490)),
            ),
        ];
        for (prompt, reply) in &cases {
            let e = from_shape(&exchange(prompt, reply)).unwrap();
            for field in [&e.title, &e.intent, &e.rationale] {
                for len in 4..=secret.len() {
                    assert!(
                        !field.contains(&secret[..len]),
                        "{} leaked in {field}",
                        &secret[..len]
                    );
                }
            }
        }
    }

    #[test]
    fn short_text_is_not_marked_as_clipped() {
        // Spaced words: a long unbroken run would be held as an opaque token.
        let exact = format!("{}e", "abcd ".repeat(79) + "abcd");
        assert_eq!(exact.chars().count(), INTENT_MAX);
        let e = from_shape(&exchange(&exact, "ok")).unwrap();
        assert_eq!(e.intent, exact);
    }

    #[test]
    fn note_uses_heading_and_body() {
        let e = from_shape(&Shape::Note {
            title: "Retries",
            body: "Retries are capped at 3 because the gateway repeats charges.",
        })
        .unwrap();
        assert_eq!(e.title, "[auto-drafted] Retries");
        assert_eq!(e.intent, "Retries");
        assert!(e.rationale.contains("capped at 3"));
    }

    #[test]
    fn credentials_are_masked_and_sensitive_sources_are_held() {
        // `token=` is masked by `redact` but is not one of the keywords that hold a source.
        let secret = ["hunt", "er2"].concat();
        let e = from_shape(&exchange(
            &format!("set the deploy token={secret} in the config"),
            &format!("wrote DEPLOY_TOKEN={secret}"),
        ))
        .unwrap();
        for field in [&e.title, &e.intent, &e.rationale] {
            assert!(!field.contains(&secret), "masked: {field}");
        }
        let block = format!("here is {} PRIVATE KEY{} abc", "-----BEGIN", "-----");
        assert!(from_shape(&exchange(&block, "ok")).is_err());
        assert!(from_shape(&exchange("fine", &block)).is_err());
        assert!(from_shape(&exchange("my password is x", "ok")).is_err());
    }

    #[test]
    fn empty_prompts_and_notes_are_held() {
        assert!(from_shape(&exchange("  ", "reply")).is_err());
        assert!(from_shape(&exchange("\n \n", "reply")).is_err());
        assert!(from_shape(&Shape::Note {
            title: "T",
            body: " "
        })
        .is_err());
        assert!(from_shape(&Shape::Note {
            title: " ",
            body: "body"
        })
        .is_err());
    }

    #[test]
    fn complete_follows_the_summarizer() {
        let local = config(Summarizer::Local, "http://127.0.0.1:1/x");
        let rules = config(Summarizer::Rules, "");
        assert!(complete(&local, true));
        assert!(!complete(&local, false));
        assert!(!complete(&rules, true));
        assert!(!complete(&rules, false));
    }

    #[tokio::test]
    async fn rules_mode_never_reaches_the_model_path() {
        // The model path rejects anything but a loopback URL before connecting, so success
        // with an invalid endpoint shows the dispatcher did not call it.
        let c = config(Summarizer::Rules, "not a url");
        assert!(wk_summarize::extract(&c.inference, &c.model, "x")
            .await
            .is_err());
        let shape = exchange("Add a limiter", "Used middleware");
        let e = extract(&c, &shape, "unused source").await.unwrap();
        assert_eq!(e.title, "[auto-drafted] Add a limiter");
        assert_eq!(e.rationale, "Used middleware");
        // A held item stays held through the dispatcher.
        assert!(extract(&c, &exchange("", "r"), "").await.is_err());
    }

    fn title_of(prompt: &str) -> String {
        from_shape(&exchange(prompt, "Done."))
            .unwrap()
            .title
            .trim_start_matches(MARK)
            .to_string()
    }

    #[test]
    fn greetings_before_the_request_do_not_become_the_title() {
        let e = from_shape(&exchange(
            "Hey! Quick one for you.\nThe order list is slow for big customers. Add pagination.",
            "Added cursor pagination.",
        ))
        .unwrap();
        assert_eq!(
            e.title,
            "[auto-drafted] The order list is slow for big customers. Add pagination."
        );
        assert!(
            e.intent.starts_with("The order list is slow"),
            "{}",
            e.intent
        );
        assert_eq!(
            title_of("Hi there, hope you're having a good week!\nAdd an export by date range."),
            "Add an export by date range."
        );
        // A request that merely starts with a greeting word is kept whole.
        assert_eq!(
            title_of("Hey, fix the login redirect"),
            "Hey, fix the login redirect"
        );
        // Nothing but a greeting: keep what was said rather than invent a title.
        assert_eq!(title_of("Hi!"), "Hi!");
    }

    #[test]
    fn pasted_traces_and_code_fences_before_the_request_do_not_become_the_title() {
        let trace = "Traceback (most recent call last):\n  File \"auth/session.py\", line 88, in load\n    sid = cookies['session_id']\nKeyError: 'session_id'\n\nwhy does login fail after a deploy?";
        assert_eq!(title_of(trace), "why does login fail after a deploy?");
        assert_eq!(
            title_of("KeyError: 'wk_session'\n    at load_session (auth/session.py:92)\n\nthe mobile build crashes on login, can you look?"),
            "the mobile build crashes on login, can you look?"
        );
        assert_eq!(
            title_of(
                "```\n- timeout = 10\n+ timeout = 30\n```\nIs this diff OK for the gateway client?"
            ),
            "Is this diff OK for the gateway client?"
        );
        // Only a trace or only a fence: fall back to the first line as before.
        assert_eq!(title_of("KeyError: 'x'"), "KeyError: 'x'");
        assert_eq!(title_of("```\nlet a = 1;\n```"), "```");
        // Prose that mentions an error later is not skipped.
        assert_eq!(
            title_of("Handle the KeyError in load_session"),
            "Handle the KeyError in load_session"
        );
    }

    #[test]
    fn a_bare_confirmation_is_titled_from_the_reply() {
        for prompt in [
            "yes, go ahead with option 2",
            "ok do it",
            "Continue",
            "Looks good, ship it.",
        ] {
            let e = from_shape(&exchange(
                prompt,
                "Done. Webhook handling now acknowledges the gateway first and queues the event. Events are deduplicated by id.",
            ))
            .unwrap();
            assert_eq!(
                e.title,
                "[auto-drafted] Webhook handling now acknowledges the gateway first and queues the event.",
                "{prompt}"
            );
            assert!(
                e.intent.starts_with("Webhook handling now acknowledges"),
                "{prompt}: {}",
                e.intent
            );
        }
        // A short question is a real request, not a confirmation.
        assert_eq!(
            title_of("Yes or no: should retries stop at 3?"),
            "Yes or no: should retries stop at 3?"
        );
        // Nothing to take a title from in the reply: keep the prompt.
        let e = from_shape(&exchange("continue", "Done.")).unwrap();
        assert_eq!(e.title, "[auto-drafted] continue");
    }

    #[test]
    fn trailing_courtesies_are_dropped_from_the_rationale() {
        let e = from_shape(&exchange(
            "Rename the key",
            "Renamed the key everywhere.\nThe old name is no longer read.\n\nHope this helps! Let me know if you'd like a deprecation warning.",
        ))
        .unwrap();
        assert_eq!(
            e.rationale,
            "Renamed the key everywhere.\nThe old name is no longer read."
        );
        // Only the closing paragraph goes; a courtesy in the middle stays.
        let e = from_shape(&exchange(
            "Rename the key",
            "Feel free to revert this step.\n\nRenamed the key.\n\nWould you like me to also add a warning?",
        ))
        .unwrap();
        assert_eq!(
            e.rationale,
            "Feel free to revert this step.\n\nRenamed the key."
        );
        // A reply that is only a courtesy is kept rather than emptied.
        let e = from_shape(&exchange("Rename the key", "Let me know if that works.")).unwrap();
        assert_eq!(e.rationale, "Let me know if that works.");
    }

    #[test]
    fn a_long_reply_keeps_its_summary_not_just_its_opening() {
        let explore = "I read the handler and traced every call. ".repeat(60);
        let reply = format!("{explore}\n\nIn summary: the button had two handlers; I removed onClick and disabled it while a request is in flight.");
        assert!(reply.chars().count() > RATIONALE_MAX);
        let e = from_shape(&exchange("Why two submits?", &reply)).unwrap();
        assert!(
            e.rationale
                .starts_with("In summary: the button had two handlers"),
            "{}",
            e.rationale
        );
        assert!(!e.rationale.contains("traced every call"));
        for marker in [
            "TL;DR: removed onClick.",
            "## Summary\nRemoved onClick.",
            "**Summary**: removed onClick.",
        ] {
            let e = from_shape(&exchange(
                "Why two submits?",
                &format!("{explore}\n\n{marker}"),
            ))
            .unwrap();
            assert!(
                e.rationale.contains("removed onClick") || e.rationale.contains("Removed onClick"),
                "{marker}"
            );
            assert!(
                e.rationale.chars().count() < 100,
                "{marker}: {}",
                e.rationale
            );
        }
        // Without a marker, or when the reply already fits, nothing moves.
        let e = from_shape(&exchange("Why?", &explore)).unwrap();
        assert!(e.rationale.starts_with("I read the handler"));
        let e = from_shape(&exchange(
            "Why?",
            "I looked around.\n\nIn summary: it is fine.",
        ))
        .unwrap();
        assert!(e.rationale.starts_with("I looked around."));
    }

    /// Not a test: writes the three draft kinds for a synthetic fixture so
    /// `scripts/summary-quality-eval.py` can score them. Run with `--ignored` and
    /// `WK_EVAL_FIXTURE`, `WK_EVAL_OUT` (and `WK_EVAL_INFERENCE`, `WK_EVAL_MODEL` for the local model).
    #[tokio::test]
    #[ignore]
    async fn dump_drafts_for_summary_quality_eval() {
        let fixture: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(std::env::var("WK_EVAL_FIXTURE").unwrap()).unwrap(),
        )
        .unwrap();
        let endpoint = std::env::var("WK_EVAL_INFERENCE").unwrap_or_default();
        let model = std::env::var("WK_EVAL_MODEL").unwrap_or_default();
        let as_card = |e: Extract, id: &str, complete: bool| {
            wk_summarize::card(e, id.into(), 1, vec!["backend".into()], complete)
                .map(|c| serde_json::to_value(c).unwrap())
        };
        let mut out = vec![];
        for turn in fixture["turns"].as_array().unwrap() {
            let (id, prompt, reply) = (
                turn["id"].as_str().unwrap(),
                turn["prompt"].as_str().unwrap(),
                turn["reply"].as_str().unwrap(),
            );
            let show = |r: Result<serde_json::Value, String>| match r {
                Ok(card) => serde_json::json!({ "card": card }),
                Err(e) => serde_json::json!({ "held": e }),
            };
            let rules = from_shape(&exchange(prompt, reply)).and_then(|e| as_card(e, id, false));
            let local = if endpoint.is_empty() {
                serde_json::json!({ "skipped": "no endpoint" })
            } else {
                let source = format!("user_prompt: {prompt}\nassistant_response: {reply}");
                show(
                    wk_summarize::extract(&endpoint, &model, &source)
                        .await
                        .and_then(|e| as_card(e, id, true)),
                )
            };
            let given = serde_json::to_string(&turn["card"]).unwrap();
            let assistant =
                crate::assistant_draft::build(&given, "backend", 1, "eval", "claude-code")
                    .map(|(_, record)| record["card"].clone());
            out.push(serde_json::json!({
                "id": id,
                "rules": show(rules),
                "local": local,
                "assistant": show(assistant),
            }));
        }
        std::fs::write(
            std::env::var("WK_EVAL_OUT").unwrap(),
            serde_json::to_string_pretty(&out).unwrap(),
        )
        .unwrap();
    }
}
