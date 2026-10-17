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
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
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
        // A note's heading is its intent and its text the rationale.
        Shape::Note { title, body } => (
            title.trim(),
            clean(title, INTENT_MAX),
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
        title: format!("{MARK}{}", clean(title, TITLE_MAX)),
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

    #[test]
    fn long_text_is_clipped_on_character_boundaries() {
        let long = "é".repeat(5000);
        let e = from_shape(&exchange(&long, &long)).unwrap();
        assert_eq!(e.intent.chars().count(), INTENT_MAX + 1);
        assert_eq!(e.rationale.chars().count(), RATIONALE_MAX + 1);
        assert!(e.intent.ends_with('…') && e.rationale.ends_with('…'));
        assert!(e.title.starts_with(MARK) && e.title.ends_with('…'));
        assert_eq!(e.title.chars().count(), MARK.len() + TITLE_MAX + 1);
        assert!(e.title.chars().count() <= 256);
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
    async fn rules_mode_never_contacts_the_model() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        let c = config(Summarizer::Rules, &url);
        let shape = exchange("Add a limiter", "Used middleware");
        let e = extract(&c, &shape, "unused source").await.unwrap();
        assert_eq!(e.title, "[auto-drafted] Add a limiter");
        let accepted =
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept()).await;
        assert!(accepted.is_err(), "rules mode opened a connection");
        // An unroutable endpoint changes nothing either.
        let c = config(Summarizer::Rules, "http://127.0.0.1:1/x");
        assert!(extract(&c, &shape, "unused source").await.is_ok());
        // A held item stays held through the dispatcher.
        let bad = exchange("", "r");
        assert!(extract(&c, &bad, "").await.is_err());
    }
}
