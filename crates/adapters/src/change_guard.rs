//! Version-gated Claude Code pre-edit decision adapter.
//! Only explicit file tools are covered; shell and other write surfaces remain bypasses.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn claude_edit_tools_are_parsed_and_unsupported_shapes_are_rejected() {
        let edit = parse_edit("claude-code", "2.1.295", &json!({"hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"file_path":"/repo/src/retry.rs","old_string":"old","new_string":"new"}})).unwrap();
        assert_eq!(edit.operation_class, "edit");
        assert_eq!(edit.targets.len(), 1);
        assert_eq!(edit.targets[0].to_string_lossy(), "/repo/src/retry.rs");
        let write = parse_edit("claude-code", "2.1.295", &json!({"hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{"file_path":"/repo/src/new.rs","content":"private"}})).unwrap();
        assert_eq!(write.operation_class, "write");
        assert!(matches!(
            parse_edit("claude-code", "2.1.15", &json!({})),
            Err(GuardError::UnsupportedVersion)
        ));
        assert!(matches!(
            parse_edit("cursor", "3.23.23", &json!({})),
            Err(GuardError::UnsupportedVersion)
        ));
        assert!(matches!(
            parse_edit(
                "claude-code",
                "2.1.295",
                &json!({"hook_event_name":"PreToolUse","tool_name":"MultiEdit","tool_input":{"edits":[]}})
            ),
            Err(GuardError::InvalidInput)
        ));
    }

    #[test]
    fn multi_edit_and_versioned_block_output_are_exact() {
        let edit = parse_edit("claude-code", "2.1.295", &json!({"hook_event_name":"PreToolUse","tool_name":"MultiEdit","tool_input":{"edits":[{"file_path":"/repo/a.rs","old_string":"a","new_string":"b"},{"file_path":"/repo/b.rs","old_string":"c","new_string":"d"}]}})).unwrap();
        assert_eq!(edit.targets.len(), 2);
        assert_eq!(
            guard_output(
                "claude-code",
                GuardDecision::Stop {
                    reason: "review required".into(),
                    brief_url: "https://host/?brief=opaque".into()
                }
            ),
            json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Developer review required: review required. Open https://host/?brief=opaque"}})
        );
        assert_eq!(
            guard_output("claude-code", GuardDecision::Proceed),
            json!({})
        );
        let fragment = hook_config("/opt/wk-agent", "/private/wk", "2.1.295").unwrap();
        assert_eq!(
            fragment["hooks"]["PreToolUse"][0]["hooks"][0]["onFailure"],
            "block"
        );
        assert_eq!(
            fragment["hooks"]["PreToolUse"][0]["matcher"],
            "Edit|Write|MultiEdit"
        );
        assert!(hook_config("/opt/wk-agent", "/private/wk", "2.1.15").is_err());
        assert!(!supports_fail_closed("2.1.294"));
    }
}
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardError {
    UnsupportedVersion,
    UnsupportedTool,
    InvalidInput,
    OutOfScope,
}

#[derive(Clone, Debug)]
pub struct GuardEdit {
    pub targets: Vec<PathBuf>,
    pub operation_class: String,
    /// Private local input used only to fingerprint the exact retry.
    pub tool_input: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardDecision {
    Stop { reason: String, brief_url: String },
    Proceed,
}

pub fn supports_fail_closed(version: &str) -> bool {
    let version = version.trim().trim_start_matches('v');
    let mut parts = version.split('.');
    let parsed = (parts.next(), parts.next(), parts.next());
    if parts.next().is_some() {
        return false;
    }
    let (Some(major), Some(minor), Some(patch)) = parsed else {
        return false;
    };
    match (
        major.parse::<u32>(),
        minor.parse::<u32>(),
        patch.parse::<u32>(),
    ) {
        (Ok(2), Ok(minor), Ok(patch)) => minor > 1 || minor == 1 && patch >= 295,
        (Ok(major), Ok(_), Ok(_)) => major > 2,
        _ => false,
    }
}

/// Parse only the documented Claude Code Edit, Write and MultiEdit file shapes.
pub fn parse_edit(client: &str, version: &str, payload: &Value) -> Result<GuardEdit, GuardError> {
    if client != "claude-code" || !supports_fail_closed(version) {
        return Err(GuardError::UnsupportedVersion);
    }
    if payload["hook_event_name"] != "PreToolUse" {
        return Err(GuardError::InvalidInput);
    }
    let tool = payload["tool_name"]
        .as_str()
        .ok_or(GuardError::InvalidInput)?;
    let input = payload.get("tool_input").ok_or(GuardError::InvalidInput)?;
    let mut targets = Vec::new();
    let operation_class = match tool {
        "Edit" | "Write" => {
            let path = input["file_path"]
                .as_str()
                .ok_or(GuardError::InvalidInput)?;
            if path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control) {
                return Err(GuardError::InvalidInput);
            }
            if tool == "Edit"
                && (input["old_string"].as_str().is_none()
                    || input["new_string"].as_str().is_none())
            {
                return Err(GuardError::InvalidInput);
            }
            if tool == "Write" && input["content"].as_str().is_none() {
                return Err(GuardError::InvalidInput);
            }
            targets.push(PathBuf::from(path));
            if tool == "Write" {
                "write"
            } else {
                "edit"
            }
        }
        "MultiEdit" => {
            let edits = input["edits"].as_array().ok_or(GuardError::InvalidInput)?;
            if edits.is_empty() || edits.len() > 20 {
                return Err(GuardError::InvalidInput);
            }
            for edit in edits {
                let path = edit["file_path"].as_str().ok_or(GuardError::InvalidInput)?;
                if path.is_empty()
                    || path.len() > 4096
                    || path.chars().any(char::is_control)
                    || edit["old_string"].as_str().is_none()
                    || edit["new_string"].as_str().is_none()
                {
                    return Err(GuardError::InvalidInput);
                }
                targets.push(PathBuf::from(path));
            }
            "multi_edit"
        }
        _ => return Err(GuardError::UnsupportedTool),
    };
    let unique: std::collections::BTreeSet<_> = targets.iter().collect();
    if unique.len() != targets.len() {
        return Err(GuardError::InvalidInput);
    }
    Ok(GuardEdit {
        targets,
        operation_class: operation_class.into(),
        tool_input: input.clone(),
    })
}

/// Claude Code interprets this JSON decision before its ordinary permission flow.
pub fn guard_output(client: &str, decision: GuardDecision) -> Value {
    match (client, decision) {
        ("claude-code", GuardDecision::Stop { reason, brief_url }) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": format!("Developer review required: {}. Open {}", reason.chars().take(220).collect::<String>(), brief_url)
            }
        }),
        _ => json!({}),
    }
}

fn quoted(path: &str) -> Result<String, String> {
    if path.is_empty()
        || path.chars().any(char::is_control)
        || path.contains(['"', '\'', '`', '$', '%'])
    {
        return Err("unsafe hook command path".into());
    }
    Ok(format!("\"{path}\""))
}

/// Separate config ownership makes protection removable without touching capture hooks.
pub fn hook_config(executable: &str, state: &str, version: &str) -> Result<Value, String> {
    if !supports_fail_closed(version) {
        return Err(
            "Claude Code 2.1.295 or later is required for fail-closed hook behavior".into(),
        );
    }
    let command = format!(
        "{} --state-dir {} protect-hook --client claude-code --version {}",
        quoted(executable)?,
        quoted(state)?,
        version
    );
    Ok(
        json!({"hooks":{"PreToolUse":[{"matcher":"Edit|Write|MultiEdit","hooks":[{"type":"command","command":command,"timeout":5,"onFailure":"block"}]}]}}),
    )
}
