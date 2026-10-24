//! Natural-language keyword retrieval. `plainto_tsquery` ANDs every word, so an agent that
//! phrases its request as a sentence matched nothing. The caller's text becomes an OR query
//! over its meaningful words instead, ranked so cards matching more of them come first.
//!
//! The query text reaches `to_tsquery` only as a bind parameter and only after every token has
//! been checked to be alphanumeric, so no operator, quote or prefix marker can reach it.

const MAX_TOKENS: usize = 12;
/// Far below the tsquery word limit, so `to_tsquery` can never reject a token for its length.
const MAX_TOKEN_BYTES: usize = 255;
const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "how", "i", "if", "in",
    "into", "is", "it", "its", "my", "of", "on", "or", "so", "that", "the", "their", "then",
    "there", "these", "they", "this", "to", "us", "was", "we", "what", "when", "where", "which",
    "who", "will", "with", "you", "your", "about", "before", "any", "anything", "should", "know",
    "need", "add", "do", "does",
];

/// The OR tsquery text for `text`: distinct lowercase alphanumeric words of at least two
/// characters that are not stopwords, in order of first appearance, at most twelve.
/// `None` when no word remains; callers then keep `plainto_tsquery` on the original text.
pub(crate) fn or_query(text: &str) -> Option<String> {
    let lowered = text.to_lowercase();
    let mut tokens: Vec<&str> = Vec::new();
    for token in lowered.split(|c: char| !c.is_alphanumeric()) {
        if token.chars().nth(1).is_none()
            || token.len() > MAX_TOKEN_BYTES
            || STOP.contains(&token)
            || tokens.contains(&token)
            // Splitting already guarantees this; the check is what the safety claim rests on.
            || !token.chars().all(char::is_alphanumeric)
        {
            continue;
        }
        tokens.push(token);
        if tokens.len() == MAX_TOKENS {
            break;
        }
    }
    (!tokens.is_empty()).then(|| tokens.join(" | "))
}

/// What to bind for a search: the text and whether it is an OR query for `to_tsquery`
/// (`true`) or the original text for the `plainto_tsquery` fallback (`false`).
pub(crate) fn bind(text: &str) -> (String, bool) {
    match or_query(text) {
        Some(or) => (or, true),
        None => (text.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_phrasings_become_or_queries_over_content_words() {
        assert_eq!(
            or_query("rate limiting backend API").unwrap(),
            "rate | limiting | backend | api"
        );
        assert_eq!(
            or_query("I'm about to add rate limiting to the backend API").unwrap(),
            "rate | limiting | backend | api"
        );
        assert_eq!(or_query("Token Bucket").unwrap(), "token | bucket");
    }

    #[test]
    fn short_words_stopwords_and_repeats_are_dropped_in_first_seen_order() {
        assert_eq!(
            or_query("a x Retry retry RETRY b2 b2").unwrap(),
            "retry | b2"
        );
        assert_eq!(or_query("naïve ÉCOLE école").unwrap(), "naïve | école");
    }

    #[test]
    fn nothing_usable_means_fallback() {
        for text in [
            "",
            "   ",
            "the and of",
            "I'm about to",
            "a b c",
            "!!! & | :",
            "---",
        ] {
            assert_eq!(or_query(text), None, "{text:?}");
            assert_eq!(bind(text), (text.to_string(), false));
        }
        assert_eq!(bind("rate limiting"), ("rate | limiting".to_string(), true));
    }

    #[test]
    fn at_most_twelve_tokens_are_kept() {
        let text = (1..=30)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let q = or_query(&text).unwrap();
        assert_eq!(q.split(" | ").count(), 12);
        assert!(q.starts_with("word1 | word2 | "));
        assert!(q.ends_with("word12"));
    }

    #[test]
    fn oversized_tokens_are_ignored() {
        assert_eq!(or_query(&"x".repeat(300)), None);
        assert_eq!(
            or_query(&format!("{} keep", "x".repeat(300))).unwrap(),
            "keep"
        );
    }

    #[test]
    fn operators_quotes_and_punctuation_cannot_change_the_query_structure() {
        let hostile = [
            "a & b | !c",
            "foo:* & !bar",
            "foo <-> bar <2> baz",
            "'quoted' \"double\" \\back\\ (paren) (alpha | beta) & !gamma",
            "x'); DROP TABLE cards;--",
            "foo|bar&baz!qux:ab*cd",
            "rate-limiting: token_bucket; 100 req/min",
            "\u{0}null\u{1}ctl alpha",
            "İstanbul ǅ ß ½ ① \u{301}combining alpha\u{301}",
            "emoji 🚀 rocket 日本語 テスト",
        ];
        for text in hostile {
            if let Some(q) = or_query(text) {
                let parts: Vec<&str> = q.split(" | ").collect();
                assert!((1..=MAX_TOKENS).contains(&parts.len()), "{text:?}");
                for part in parts {
                    assert!(
                        part.chars().count() >= 2 && part.chars().all(char::is_alphanumeric),
                        "{text:?} produced {part:?}"
                    );
                }
            }
        }
        // Exhaustive over every ASCII punctuation character mixed with words.
        for c in (0u8..128)
            .map(char::from)
            .filter(|c| !c.is_ascii_alphanumeric())
        {
            let q = or_query(&format!("alpha{c}beta {c}{c} gamma{c}")).unwrap();
            assert!(
                q.split(" | ").all(|t| t.chars().all(char::is_alphanumeric)),
                "{c:?} produced {q:?}"
            );
        }
    }
}
