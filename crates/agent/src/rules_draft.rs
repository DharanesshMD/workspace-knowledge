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

/// Build an `Extract` without a model. Errors hold the item (nothing to quote, or sensitive).
pub(crate) fn from_shape(shape: &Shape<'_>) -> Result<Extract, String> {
    let (title, intent, rationale, whole) = match shape {
        Shape::Exchange { prompt, reply } => (
            first_line(prompt),
            clean(prompt, INTENT_MAX),
            clean(reply, RATIONALE_MAX),
            format!("{prompt}\n{reply}"),
        ),
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
}
