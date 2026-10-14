//! Splits coding-agent memory files into reviewable sections. Pure: no I/O.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Instructions,
    Note,
}
impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Instructions => "instructions",
            SourceKind::Note => "claude_note",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Heading path; empty for a note or for text before any heading.
    pub heading: Vec<String>,
    /// Index among earlier sections with the same heading path.
    pub occurrence: usize,
    /// Note frontmatter `name`/`description`, used only as model hints.
    pub hints: Option<String>,
    pub body: String,
}
pub fn split(kind: SourceKind, text: &str) -> Vec<Section> {
    let text = text.replace("\r\n", "\n");
    match kind {
        SourceKind::Note => note(&text),
        SourceKind::Instructions => instructions(&text),
    }
}
fn note(text: &str) -> Vec<Section> {
    let (hints, body) = frontmatter(text);
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
fn frontmatter(text: &str) -> (Option<String>, String) {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return (None, text.to_string());
    }
    let mut meta = Vec::new();
    for line in lines.by_ref() {
        if line == "---" {
            let hints: Vec<String> = meta
                .iter()
                .filter_map(|line: &&str| {
                    let (key, value) = line.split_once(':')?;
                    let (key, value) = (key.trim(), value.trim().trim_matches('"'));
                    (matches!(key, "name" | "description") && !value.is_empty())
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
    let title = line[level..].trim().trim_end_matches('#').trim();
    (!title.is_empty()).then(|| (level, title.to_string()))
}
fn instructions(text: &str) -> Vec<Section> {
    let mut fenced = false;
    let lines: Vec<(&str, Option<(usize, String)>)> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                return (line, None);
            }
            (line, if fenced { None } else { heading(line) })
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
    let mut body: Vec<&str> = Vec::new();
    for (line, found) in &lines {
        match (found, split_level) {
            (Some((level, title)), Some(at)) if *level == at => {
                push(&mut sections, &current, &body);
                body.clear();
                current = parents.iter().map(|(_, t)| t.clone()).collect();
                current.push(title.clone());
            }
            (Some((level, title)), Some(at)) if *level < at => {
                push(&mut sections, &current, &body);
                body.clear();
                parents.retain(|(l, _)| l < level);
                parents.push((*level, title.clone()));
                current = parents.iter().map(|(_, t)| t.clone()).collect();
            }
            _ => body.push(*line),
        }
    }
    push(&mut sections, &current, &body);
    sections
}
fn push(sections: &mut Vec<Section>, heading: &[String], body: &[&str]) {
    let text = body.join("\n").trim().to_string();
    if text.is_empty() {
        return;
    }
    let occurrence = sections.iter().filter(|s| s.heading == heading).count();
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
            "---\nname: retries\ndescription: \"Retry policy\"\nmetadata:\n  type: project\n---\nPayment retries reuse the idempotency key.\n",
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
    fn text_without_headings_is_one_preamble_section() {
        let s = split(SourceKind::Instructions, "Always run the linter.\r\n");
        assert_eq!(headings(&s), vec![h(&[], 0)]);
        assert_eq!(s[0].body, "Always run the linter.");
    }
    #[test]
    fn empty_note_has_no_sections() {
        assert!(split(SourceKind::Note, "---\nname: x\n---\n\n").is_empty());
    }
}
