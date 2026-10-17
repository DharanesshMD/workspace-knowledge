//! Splits coding-agent memory files into reviewable sections. Pure: no I/O.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Instructions,
    /// VS Code Copilot `*.instructions.md`: frontmatter (`applyTo`,
    /// `description`) is dropped and `description` hints every section.
    ScopedInstructions,
    Note,
}
impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Instructions => "instructions",
            SourceKind::ScopedInstructions => "scoped_instructions",
            SourceKind::Note => "claude_note",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading path; empty for a note or for text before any heading.
    pub heading: Vec<String>,
    /// Index among earlier headings with the same path.
    pub occurrence: usize,
    /// Frontmatter `name`/`description` (notes) or `description` (scoped
    /// instructions), used only as model hints.
    pub hints: Option<String>,
    pub body: String,
}
pub fn split(kind: SourceKind, text: &str) -> Vec<Section> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    match kind {
        SourceKind::Note => note(&text),
        SourceKind::Instructions => instructions(&text),
        SourceKind::ScopedInstructions => {
            let (hints, body) = frontmatter(&text, &["description"]);
            instructions(&body)
                .into_iter()
                .map(|s| Section {
                    hints: hints.clone(),
                    ..s
                })
                .collect()
        }
    }
}
fn note(text: &str) -> Vec<Section> {
    let (hints, body) = frontmatter(text, &["name", "description"]);
    let body = body.trim();
    if body.is_empty() {
        return vec![];
    }
    vec![Section {
        heading: vec![],
        occurrence: 0,
        hints,
        body: body.to_string(),
    }]
}
/// One matching pair of surrounding `"` or `'` quotes, if present.
fn unquote(value: &str) -> &str {
    ['"', '\'']
        .into_iter()
        .find_map(|q| value.strip_prefix(q)?.strip_suffix(q))
        .unwrap_or(value)
}
/// Drops leading `---` frontmatter; top-level `keys` with plain values become
/// hints. Without a closing `---` there is no frontmatter: the text is kept.
fn frontmatter(text: &str, keys: &[&str]) -> (Option<String>, String) {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return (None, text.to_string());
    }
    let mut meta: Vec<&str> = Vec::new();
    for line in lines.by_ref() {
        if line == "---" {
            let hints: Vec<String> = meta
                .iter()
                .filter(|entry| !entry.starts_with([' ', '\t']))
                .filter_map(|entry| {
                    let (key, value) = entry.split_once(':')?;
                    let (key, value) = (key.trim(), unquote(value.trim()));
                    (keys.contains(&key) && !value.is_empty() && !value.starts_with(['>', '|']))
                        .then(|| format!("{key}: {value}"))
                })
                .collect();
            let body: Vec<&str> = lines.collect();
            return (
                (!hints.is_empty()).then(|| hints.join("; ")),
                body.join("\n"),
            );
        }
        meta.push(line);
    }
    (None, text.to_string())
}
fn heading(line: &str) -> Option<(usize, String)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) || !line[level..].starts_with(' ') {
        return None;
    }
    let title = line[level..].trim();
    // Closing hashes count only after a space, so `C#` keeps its hash.
    let title = match title.trim_end_matches('#') {
        t if t.is_empty() || t.ends_with(' ') => t.trim_end(),
        _ => title,
    };
    (!title.is_empty()).then(|| (level, title.to_string()))
}
fn instructions(text: &str) -> Vec<Section> {
    // A fence closes only with the marker that opened it.
    let mut fence: Option<&str> = None;
    let lines: Vec<(&str, Option<(usize, String)>)> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(marker) = ["```", "~~~"].into_iter().find(|m| trimmed.starts_with(m)) {
                match fence {
                    None => fence = Some(marker),
                    Some(open) if open == marker => fence = None,
                    Some(_) => {}
                }
                return (line, None);
            }
            (line, if fence.is_some() { None } else { heading(line) })
        })
        .collect();
    let mut counts = [0usize; 7];
    for (_, found) in &lines {
        if let Some((level, _)) = found {
            counts[*level] += 1;
        }
    }
    // Split at the highest level used more than once, else the highest present.
    let split_level = (1..=6)
        .find(|level| counts[*level] >= 2)
        .or_else(|| (1..=6).find(|level| counts[*level] >= 1));
    let mut sections = Vec::new();
    let mut parents: Vec<(usize, String)> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    // Every heading counts, even one with an empty body, so keys stay stable.
    let mut seen: Vec<Vec<String>> = Vec::new();
    let mut occurrence = 0;
    let mut body: Vec<&str> = Vec::new();
    for (line, found) in &lines {
        match (found, split_level) {
            (Some((level, title)), Some(at)) if *level == at => {
                push(&mut sections, &current, occurrence, &body);
                body.clear();
                current = parents.iter().map(|(_, t)| t.clone()).collect();
                current.push(title.clone());
                occurrence = seen.iter().filter(|p| **p == current).count();
                seen.push(current.clone());
            }
            (Some((level, title)), Some(at)) if *level < at => {
                push(&mut sections, &current, occurrence, &body);
                body.clear();
                parents.retain(|(l, _)| l < level);
                parents.push((*level, title.clone()));
                current = parents.iter().map(|(_, t)| t.clone()).collect();
                occurrence = seen.iter().filter(|p| **p == current).count();
                seen.push(current.clone());
            }
            _ => body.push(*line),
        }
    }
    push(&mut sections, &current, occurrence, &body);
    sections
}
fn push(sections: &mut Vec<Section>, heading: &[String], occurrence: usize, body: &[&str]) {
    let text = body.join("\n").trim().to_string();
    if text.is_empty() {
        return;
    }
    sections.push(Section {
        heading: heading.to_vec(),
        occurrence,
        hints: None,
        body: text,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn h(parts: &[&str], occurrence: usize) -> (Vec<String>, usize) {
        (parts.iter().map(|p| p.to_string()).collect(), occurrence)
    }
    fn headings(sections: &[Section]) -> Vec<(Vec<String>, usize)> {
        sections
            .iter()
            .map(|s| (s.heading.clone(), s.occurrence))
            .collect()
    }
    #[test]
    fn note_drops_frontmatter_and_keeps_name_and_description_as_hints() {
        let s = split(
            SourceKind::Note,
            "---\nname: retries\ndescription: \"Retry policy\"\nmetadata:\n  type: project\n  name: nested\n---\nPayment retries reuse the idempotency key.\n",
        );
        assert_eq!(s.len(), 1);
        assert_eq!(
            s[0].hints.as_deref(),
            Some("name: retries; description: Retry policy")
        );
        assert_eq!(s[0].body, "Payment retries reuse the idempotency key.");
        assert!(s[0].heading.is_empty());
    }
    #[test]
    fn instructions_split_at_the_repeated_level_under_a_title() {
        let s = split(
            SourceKind::Instructions,
            "# Backend\nIntro text.\n## Build\nRun cargo test.\n### Detail\nStays in build.\n## Style\nUse rustfmt.\n",
        );
        assert_eq!(
            headings(&s),
            vec![
                h(&["Backend"], 0),
                h(&["Backend", "Build"], 0),
                h(&["Backend", "Style"], 0)
            ]
        );
        assert!(s[1].body.contains("### Detail"));
    }
    #[test]
    fn headings_inside_fenced_code_do_not_split() {
        let s = split(
            SourceKind::Instructions,
            "## Build\n```sh\n## not a heading\n```\n## Test\nRun it.\n",
        );
        assert_eq!(headings(&s), vec![h(&["Build"], 0), h(&["Test"], 0)]);
        assert!(s[0].body.contains("## not a heading"));
    }
    #[test]
    fn repeated_headings_get_occurrences_and_empty_sections_are_dropped() {
        let s = split(
            SourceKind::Instructions,
            "## Notes\nOne.\n## Empty\n\n## Notes\nTwo.\n",
        );
        assert_eq!(headings(&s), vec![h(&["Notes"], 0), h(&["Notes"], 1)]);
    }
    #[test]
    fn an_empty_duplicate_heading_still_counts_toward_occurrence() {
        let s = split(SourceKind::Instructions, "## Notes\n\n## Notes\nTwo.\n");
        assert_eq!(headings(&s), vec![h(&["Notes"], 1)]);
    }
    #[test]
    fn a_fence_closes_only_with_the_marker_that_opened_it() {
        let s = split(
            SourceKind::Instructions,
            "## A\n```md\n~~~\n## fake\n~~~\n```\n## B\nText.\n",
        );
        assert_eq!(headings(&s), vec![h(&["A"], 0), h(&["B"], 0)]);
        assert!(s[0].body.contains("## fake"));
    }
    #[test]
    fn closing_hashes_are_stripped_only_after_a_space() {
        let s = split(
            SourceKind::Instructions,
            "## Using C#\nText.\n## Style ##\nMore.\n",
        );
        assert_eq!(headings(&s), vec![h(&["Using C#"], 0), h(&["Style"], 0)]);
    }
    #[test]
    fn a_leading_byte_order_mark_is_ignored() {
        let s = split(SourceKind::Note, "\u{feff}---\nname: a\n---\nBody");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].body, "Body");
        assert_eq!(s[0].hints.as_deref(), Some("name: a"));
    }
    #[test]
    fn text_without_headings_is_one_preamble_section() {
        let s = split(SourceKind::Instructions, "Always run the linter.\r\n");
        assert_eq!(headings(&s), vec![h(&[], 0)]);
        assert_eq!(s[0].body, "Always run the linter.");
    }
    #[test]
    fn scoped_instructions_drop_frontmatter_and_hint_every_section_with_the_description() {
        let s = split(
            SourceKind::ScopedInstructions,
            "---\r\napplyTo: \"**/*.rs\"\r\nname: ignored\r\ndescription: \"Rust style\"\r\n---\r\n## Errors\r\nUse thiserror.\r\n## Tests\r\nKeep them fast.\r\n",
        );
        assert_eq!(headings(&s), vec![h(&["Errors"], 0), h(&["Tests"], 0)]);
        assert!(s
            .iter()
            .all(|s| s.hints.as_deref() == Some("description: Rust style")));
        assert_eq!(s[0].body, "Use thiserror.");
        let plain = split(
            SourceKind::ScopedInstructions,
            "---\napplyTo: \"**\"\n---\nAlways lint.\n",
        );
        assert_eq!(
            (plain[0].hints.as_deref(), plain[0].body.as_str()),
            (None, "Always lint.")
        );
    }
    #[test]
    fn hint_values_lose_one_matching_pair_of_quotes() {
        let hint = |value: &str| {
            split(
                SourceKind::ScopedInstructions,
                &format!("---\ndescription: {value}\n---\nBody\n"),
            )[0]
            .hints
            .clone()
        };
        assert_eq!(
            hint("'Rust style'").as_deref(),
            Some("description: Rust style")
        );
        assert_eq!(
            hint("\"Rust style\"").as_deref(),
            Some("description: Rust style")
        );
        assert_eq!(
            hint("\"Note: errors\"").as_deref(),
            Some("description: Note: errors")
        );
        assert_eq!(
            hint("Note: errors").as_deref(),
            Some("description: Note: errors")
        );
        assert_eq!(hint("\"\"x\"\"").as_deref(), Some("description: \"x\""));
        assert_eq!(hint("'x\"").as_deref(), Some("description: 'x\""));
    }
    #[test]
    fn unclosed_scoped_frontmatter_stays_in_the_body_without_hints() {
        let text = "---\ndescription: Rust style\nUse thiserror.\n";
        let s = split(SourceKind::ScopedInstructions, text);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].hints, None);
        assert_eq!(s[0].body, text.trim());
    }
    #[test]
    fn instructions_keep_frontmatter_as_text() {
        let s = split(SourceKind::Instructions, "---\ndescription: x\n---\nBody\n");
        assert_eq!(s[0].hints, None);
        assert!(s[0].body.starts_with("---"));
    }
    #[test]
    fn empty_note_has_no_sections() {
        assert!(split(SourceKind::Note, "---\nname: x\n---\n\n").is_empty());
    }
}
