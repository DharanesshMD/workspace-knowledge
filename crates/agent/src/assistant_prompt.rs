//! Instruction texts that make a developer's assistant write a card for `wk-agent draft`.
use serde_json::{json, Value};

fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-:@%+=".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

const BODY: &str = "Summarise this working session for my team as one knowledge card. Use only what actually happened in this session.\n\nWrite a JSON object with exactly these fields:\n- title: short, specific (under 120 characters)\n- intent: what we were trying to achieve, 1-2 sentences\n- rationale: why the code ended up this way, including approaches that failed\n- decisions: array of short strings\n- constraints: array of short strings (things a future change must respect)\n- open_questions: array of short strings\n\nRules: never include secrets, credentials, tokens, personal data or customer data; do not paste code, file contents or raw conversation text; describe in your own words. Leave an array empty rather than guessing.\n\nThen run this command, with the JSON between the two marker lines, and show me the result:\n\n    {CMD} draft --client {CLIENT} <<'WK_JSON'\n    { ...the JSON object... }\n    WK_JSON\n\nNever include the text WK_JSON in the content. If the command refuses the card, tell me why instead of rewording it to get past the screening.\n\nThe draft stays private on my machine until I review and submit it. Do not publish anything.\n";

const CODEX_GATE: &str = "When I ask you to summarise this session for Workspace Knowledge (wk-summarize), do the following. Otherwise ignore this section.\n\n";

const DESCRIPTION: &str = "Summarise this session as a private Workspace Knowledge draft";

pub(crate) fn text(client: &str, exe: &str, state: &str) -> Result<String, String> {
    if exe.chars().chain(state.chars()).any(char::is_control) {
        return Err("paths must not contain control characters".into());
    }
    let cmd = format!("{} --state-dir {}", quote(exe), quote(state));
    let body = BODY.replace("{CMD}", &cmd).replace("{CLIENT}", client);
    match client {
        "claude-code" => Ok(format!("---\ndescription: {DESCRIPTION}\n---\n{body}")),
        "copilot" => Ok(format!(
            "---\ndescription: {DESCRIPTION}\nagent: agent\n---\n{body}"
        )),
        "codex" => Ok(format!("{CODEX_GATE}{body}")),
        _ => Err("client must be claude-code, copilot or codex".into()),
    }
}

/// What the `prompt` command prints. It writes no files. Normally JSON whose `save_as` hints where
/// to put the text; with `raw`, a bare string that `main` prints as-is so it can be redirected to a file.
pub(crate) fn output(
    client: &str,
    exe: &std::path::Path,
    state: &std::path::Path,
    raw: bool,
) -> Result<Value, String> {
    let text = text(client, &exe.to_string_lossy(), &state.to_string_lossy())?;
    if raw {
        return Ok(Value::String(text));
    }
    let save_as = match client {
        "claude-code" => ".claude/commands/wk-summarize.md",
        "copilot" => ".github/prompts/wk-summarize.prompt.md",
        _ => "add this text to AGENTS.md",
    };
    Ok(json!({"client": client, "text": text, "save_as": save_as}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn every_client_text_names_the_command_the_fields_and_the_rules() {
        for client in ["claude-code", "copilot", "codex"] {
            let t = text(client, "/usr/local/bin/wk-agent", "/private/state").unwrap();
            assert!(
                t.contains("/usr/local/bin/wk-agent --state-dir /private/state draft --client"),
                "{client}"
            );
            for f in [
                "title",
                "intent",
                "rationale",
                "decisions",
                "constraints",
                "open_questions",
            ] {
                assert!(t.contains(f), "{client} {f}");
            }
            assert!(
                t.to_lowercase().contains("no secrets")
                    || t.to_lowercase().contains("never include"),
                "{client}"
            );
        }
        assert!(
            text("claude-code", "/x", "/y")
                .unwrap()
                .starts_with("---\ndescription:"),
            "slash command frontmatter"
        );
        assert!(
            text("copilot", "/x", "/y")
                .unwrap()
                .starts_with("---\ndescription:"),
            "prompt file frontmatter"
        );
        assert!(
            !text("codex", "/x", "/y").unwrap().starts_with("---"),
            "codex text has no frontmatter"
        );
        let copilot = text("copilot", "/x", "/y").unwrap();
        let frontmatter = copilot.split("---\n").nth(1).unwrap();
        assert!(frontmatter.contains("agent: agent\n"));
        assert!(!frontmatter.contains("mode:"), "mode is the retired field");
        assert!(text("vim", "/x", "/y").is_err());
    }

    #[test]
    fn each_text_names_its_own_client() {
        for client in ["claude-code", "copilot", "codex"] {
            let t = text(client, "/x", "/y").unwrap();
            assert!(
                t.contains(&format!("draft --client {client} <<'WK_JSON'")),
                "{client}"
            );
        }
    }

    #[test]
    fn codex_text_only_applies_when_asked() {
        let t = text("codex", "/x", "/y").unwrap();
        assert!(t.starts_with("When I ask you to summarise this session for Workspace Knowledge"));
        assert!(t.contains("Otherwise ignore this section."));
        let gate = t.find("Otherwise ignore").unwrap();
        assert!(gate < t.find("Write a JSON object").unwrap());
        for client in ["claude-code", "copilot"] {
            assert!(!text(client, "/x", "/y")
                .unwrap()
                .contains("Otherwise ignore"));
        }
    }

    #[test]
    fn the_command_reads_the_card_from_a_quoted_heredoc_and_refusals_are_reported() {
        for client in ["claude-code", "copilot", "codex"] {
            let t = text(client, "/bin/wk", "/s").unwrap();
            let cmd = format!("/bin/wk --state-dir /s draft --client {client} <<'WK_JSON'\n");
            let start = t.find(&cmd).expect(client);
            assert!(t[start..].contains("\n    WK_JSON\n"), "{client}");
            assert!(t.contains("Never include the text WK_JSON in the content."));
            assert!(t.contains(
                "If the command refuses the card, tell me why instead of rewording it to get past the screening."
            ));
        }
    }

    #[test]
    fn control_characters_in_paths_are_rejected() {
        for bad in ["/a\nb", "/a\rb", "/a\tb", "/a\u{1b}[0m"] {
            assert!(text("codex", bad, "/s").is_err(), "{bad:?}");
            assert!(text("codex", "/x", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn raw_output_is_the_bare_text() {
        let raw = output("claude-code", Path::new("/b"), Path::new("/s"), true).unwrap();
        assert_eq!(raw, Value::String(text("claude-code", "/b", "/s").unwrap()));
        assert!(output("vim", Path::new("/b"), Path::new("/s"), true).is_err());
    }

    #[test]
    fn paths_with_spaces_or_shell_characters_are_quoted() {
        let t = text("codex", "/opt/my tools/wk-agent", "/p/state dir").unwrap();
        assert!(t.contains("'/opt/my tools/wk-agent' --state-dir '/p/state dir'"));
        assert_eq!(quote("/a/it's"), r"'/a/it'\''s'");
        assert_eq!(quote("/a/$(x)"), "'/a/$(x)'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn output_has_client_text_and_a_save_hint_per_client() {
        let hints = [
            ("claude-code", ".claude/commands/wk-summarize.md"),
            ("copilot", ".github/prompts/wk-summarize.prompt.md"),
            ("codex", "add this text to AGENTS.md"),
        ];
        for (client, hint) in hints {
            let v = output(client, Path::new("/bin/wk-agent"), Path::new("/s"), false).unwrap();
            assert_eq!(v["client"], client);
            assert_eq!(v["save_as"], hint);
            assert_eq!(
                v["text"],
                text(client, "/bin/wk-agent", "/s").unwrap().as_str()
            );
            assert_eq!(v.as_object().unwrap().len(), 3);
        }
        assert!(output("vim", Path::new("/x"), Path::new("/y"), false).is_err());
    }
}
