//! Automatic "team context before editing, a shared summary after", per coding client.
//!
//! Nothing here happens until the developer ticks it on the approval page and clicks Approve. What
//! is written into the project (hook entries, a rule file) is recorded in a receipt beside the
//! folder's binding in the consent directory, so it can be taken out again exactly. A small ledger
//! next to it remembers which files were edited since the last share. The hooks are the bridge's
//! own (`wk-mcp hook`): they capture no conversation and do nothing once the folder's approval is
//! paused, revoked or changed.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use wk_adapters::install;
use wk_contracts::agent_setup;

/// The file tools whose use counts as an edit, as a Claude Code matcher.
const CLAUDE_EDITS: &str = "Edit|Write|MultiEdit|NotebookEdit";
/// Seconds a client waits for a hook. The lookup inside gives up sooner.
const HOOK_TIMEOUT: u64 = 10;
const BLOCK_START: &str =
    "<!-- workspace-knowledge:start (written by wk-mcp; `wk-mcp engage remove` takes it out) -->";
const BLOCK_END: &str = "<!-- workspace-knowledge:end -->";
/// Edited files remembered per folder; more than this and the oldest names stay.
const MAX_EDITED: usize = 200;
const MAX_STATE_BYTES: u64 = 256 * 1024;

/// How a hook process finds its way back to this bridge's server and consent records.
pub struct HookCommand {
    pub exe: PathBuf,
    pub server: String,
    pub product: String,
    /// Set only when the bridge was pointed at a consent directory other than the default.
    pub consent: Option<PathBuf>,
    pub ca_file: Option<PathBuf>,
}
/// Quote for a hook command line. A path that could end the quoting is refused, never escaped.
fn quoted(text: &str) -> Result<String, String> {
    if text.is_empty() || text.contains(['\'', '"', '\n', '\r', '`', '$', '%']) {
        return Err("a path or address needed for the hook command cannot be quoted safely".into());
    }
    Ok(format!("\"{text}\""))
}
impl HookCommand {
    fn line(&self, client: &str, event: &str) -> Result<String, String> {
        let path = |p: &Path| quoted(p.to_str().ok_or("non-UTF8 path in hook command")?);
        let mut line = format!(
            "{} hook --client {client} --event {event} --server {} --product {}",
            path(&self.exe)?,
            quoted(&self.server)?,
            quoted(&self.product)?
        );
        if let Some(dir) = &self.consent {
            line.push_str(&format!(" --consent-dir {}", path(dir)?));
        }
        if let Some(file) = &self.ca_file {
            line.push_str(&format!(" --ca-file {}", path(file)?));
        }
        Ok(line)
    }
    fn claude(&self) -> Result<Value, String> {
        let handler = |event: &str| -> Result<Value, String> {
            Ok(
                json!({"type":"command","command":self.line("claude-code", event)?,"timeout":HOOK_TIMEOUT}),
            )
        };
        Ok(json!({"hooks":{
            "UserPromptSubmit":[{"hooks":[handler("UserPromptSubmit")?]}],
            "PreToolUse":[{"matcher":CLAUDE_EDITS,"hooks":[handler("PreToolUse")?]}],
            "PostToolUse":[{"matcher":CLAUDE_EDITS,"hooks":[handler("PostToolUse")?]}],
            "Stop":[{"hooks":[handler("Stop")?]}],
        }}))
    }
    fn cursor(&self) -> Result<Value, String> {
        let mut hooks = serde_json::Map::new();
        for event in ["postToolUse", "afterFileEdit", "stop"] {
            hooks.insert(
                event.into(),
                json!([{"command":self.line("cursor", event)?,"timeout":HOOK_TIMEOUT}]),
            );
        }
        Ok(json!({"version":1,"hooks":hooks}))
    }
}

/// One thing written into the project for one client.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// Hook entries merged into the client's settings file; other entries are left alone.
    Hooks { fragment: Value },
    /// A marked block in a rule file that may hold the project's own text.
    Block { text: String },
    /// A rule file of our own, written only when no such file exists.
    File { text: String },
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Install {
    pub client: String,
    pub file: PathBuf,
    #[serde(flatten)]
    pub change: Change,
}
impl Install {
    /// One line for the approval page and `workspace.folder`.
    pub fn describe(&self) -> &'static str {
        match self.change {
            Change::Hooks { .. } => "hooks that bring in team context and remind to share",
            Change::Block { .. } => "a marked block of instructions; the rest of the file is kept",
            Change::File { .. } => "a rule file of instructions, written only if none exists",
        }
    }
}
/// What approving would write for `client` in `folder`, strongest form first. Empty for a client
/// with neither hooks nor a rule file: it is steered by instructions and tool results alone.
pub fn plan(
    client: Option<&str>,
    folder: &Path,
    command: &HookCommand,
) -> Result<Vec<Install>, String> {
    let Some(client) = client else {
        return Ok(Vec::new());
    };
    let mut installs = Vec::new();
    let hooks = match client {
        "claude-code" => Some((".claude/settings.local.json", command.claude()?)),
        "cursor" => Some((".cursor/hooks.json", command.cursor()?)),
        _ => None,
    };
    if let Some((file, fragment)) = hooks {
        installs.push(Install {
            client: client.into(),
            file: folder.join(file),
            change: Change::Hooks { fragment },
        });
    }
    // Claude Code's hooks cover both halves, so it needs no rule file. Cursor's cannot put
    // context in front of the agent before an edit, so there the rule file does that half.
    if client != "claude-code" {
        if let Some(rule) = agent_setup::rule_file(client) {
            let change = if rule.path.ends_with(".md") {
                Change::Block { text: rule.content }
            } else {
                Change::File { text: rule.content }
            };
            installs.push(Install {
                client: client.into(),
                file: folder.join(rule.path),
                change,
            });
        }
    }
    Ok(installs)
}

fn read(path: &Path) -> Result<Option<String>, String> {
    match fs::metadata(path) {
        Err(_) => Ok(None),
        Ok(meta) if !meta.is_file() || meta.len() > 4 * 1024 * 1024 => Err(format!(
            "{} is not a file that can be edited",
            path.display()
        )),
        Ok(_) => fs::read_to_string(path)
            .map(Some)
            .map_err(|_| format!("{} cannot be read", path.display())),
    }
}
/// Write through a sibling temporary file, so a reader never sees half a file.
fn write(path: &Path, text: &str) -> Result<(), String> {
    let failed = || format!("{} cannot be written", path.display());
    let name = path.file_name().ok_or_else(failed)?.to_string_lossy();
    let temp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    fs::write(&temp, text).map_err(|_| failed())?;
    fs::rename(&temp, path).map_err(|_| {
        let _ = fs::remove_file(&temp);
        failed()
    })
}
/// Make the file's directory and check it is really inside `folder`, so a symlinked `.claude` or
/// `.cursor` cannot send a write somewhere else.
fn inside(folder: &Path, file: &Path) -> Result<(), String> {
    let parent = file.parent().ok_or("invalid setup path")?;
    fs::create_dir_all(parent).map_err(|_| format!("{} cannot be created", parent.display()))?;
    let real = parent
        .canonicalize()
        .map_err(|_| "setup path unavailable")?;
    if !real.starts_with(folder) {
        return Err(format!(
            "{} leads outside the project folder",
            parent.display()
        ));
    }
    Ok(())
}
fn json_file(path: &Path) -> Result<Value, String> {
    match read(path)? {
        None => Ok(json!({})),
        Some(text) if text.trim().is_empty() => Ok(json!({})),
        // Comments would be lost on rewrite, so a file that is not strict JSON is left alone.
        Some(text) => serde_json::from_str(&text).map_err(|_| {
            format!(
                "{} is not strict JSON; it was left unchanged",
                path.display()
            )
        }),
    }
}
fn pretty(value: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(value).expect("json"))
}
/// `existing` with our marked block replaced, or added at the end.
fn with_block(existing: &str, text: &str) -> String {
    let block = format!("{BLOCK_START}\n{}\n{BLOCK_END}\n", text.trim_end());
    let kept = without_block(existing);
    if kept.trim().is_empty() {
        block
    } else {
        format!("{}\n\n{block}", kept.trim_end())
    }
}
/// `existing` without our marked block. Text outside the markers is never touched.
fn without_block(existing: &str) -> String {
    let (Some(start), Some(end)) = (existing.find(BLOCK_START), existing.find(BLOCK_END)) else {
        return existing.to_string();
    };
    if end < start {
        return existing.to_string();
    }
    let after = existing[end + BLOCK_END.len()..].trim_start_matches(['\r', '\n']);
    let before = existing[..start].trim_end();
    match (before.is_empty(), after.is_empty()) {
        (true, _) => after.to_string(),
        (false, true) => format!("{before}\n"),
        (false, false) => format!("{before}\n\n{after}"),
    }
}
/// Drop event lists and the `hooks` object that removal left empty.
fn pruned(mut config: Value) -> Value {
    if let Some(hooks) = config.get_mut("hooks").and_then(Value::as_object_mut) {
        hooks.retain(|_, list| list.as_array().is_none_or(|l| !l.is_empty()));
        if hooks.is_empty() {
            config.as_object_mut().expect("object").remove("hooks");
        }
    }
    config
}
/// Write one change. Returns whether anything is now ours to remove later.
fn apply(folder: &Path, install: &Install) -> Result<bool, String> {
    inside(folder, &install.file)?;
    match &install.change {
        Change::Hooks { fragment } => {
            let merged = install::merge(&json_file(&install.file)?, fragment)?;
            write(&install.file, &pretty(&merged))?;
            Ok(true)
        }
        Change::Block { text } => {
            let existing = read(&install.file)?.unwrap_or_default();
            write(&install.file, &with_block(&existing, text))?;
            Ok(true)
        }
        Change::File { text } => match read(&install.file)? {
            // The developer's own rule file wins; it is neither changed nor recorded as ours.
            Some(existing) => Ok(existing == *text),
            None => write(&install.file, text).map(|()| true),
        },
    }
}
/// Take one change out again, leaving anything the developer added or edited.
fn undo(install: &Install) -> Result<(), String> {
    let gone = |path: &Path| {
        fs::remove_file(path).map_err(|_| format!("{} cannot be removed", path.display()))
    };
    match &install.change {
        Change::Hooks { fragment } => {
            if read(&install.file)?.is_none() {
                return Ok(());
            }
            let left = pruned(install::remove(&json_file(&install.file)?, fragment)?);
            let only_version = left
                .as_object()
                .is_some_and(|o| o.keys().all(|k| k == "version"));
            if only_version {
                gone(&install.file)
            } else {
                write(&install.file, &pretty(&left))
            }
        }
        Change::Block { .. } => match read(&install.file)? {
            None => Ok(()),
            Some(existing) => {
                let left = without_block(&existing);
                if left.trim().is_empty() {
                    gone(&install.file)
                } else if left != existing {
                    write(&install.file, &left)
                } else {
                    Ok(())
                }
            }
        },
        Change::File { text } => match read(&install.file)? {
            Some(existing) if existing == *text => gone(&install.file),
            _ => Ok(()),
        },
    }
}

/// What was written for a folder. Its presence is the developer's "automatic" choice.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Receipt {
    pub installs: Vec<Install>,
}
impl Receipt {
    pub fn hooks_for(&self, client: Option<&str>) -> bool {
        client.is_some_and(|c| {
            self.installs
                .iter()
                .any(|i| i.client == c && matches!(i.change, Change::Hooks { .. }))
        })
    }
    pub fn covers(&self, client: Option<&str>) -> bool {
        client.is_some_and(|c| self.installs.iter().any(|i| i.client == c))
    }
}
fn state_file(consent: &Path, folder: &Path, kind: &str) -> Result<PathBuf, String> {
    Ok(wk_scope::consent::binding_file(consent, folder)?.with_extension(format!("{kind}.json")))
}
fn load<T: serde::de::DeserializeOwned>(consent: &Path, folder: &Path, kind: &str) -> Option<T> {
    let path = state_file(consent, folder, kind).ok()?;
    if fs::metadata(&path).ok()?.len() > MAX_STATE_BYTES {
        return None;
    }
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}
fn save<T: Serialize>(consent: &Path, folder: &Path, kind: &str, value: &T) -> Result<(), String> {
    write(
        &state_file(consent, folder, kind)?,
        &serde_json::to_string(value).map_err(|_| "invalid state")?,
    )
}
/// The receipt for `folder`, if the developer turned automatic team context on there.
pub fn receipt(consent: &Path, folder: &Path) -> Option<Receipt> {
    load(consent, folder, "engage")
}
/// Turn automatic team context on for `folder` and write `planned`. A change that replaces an
/// earlier one for the same client and file takes the earlier one out first. Changes that fail
/// are returned as messages; the rest still apply, and the receipt is saved either way.
pub fn turn_on(
    consent: &Path,
    folder: &Path,
    planned: Vec<Install>,
) -> Result<Vec<String>, String> {
    let mut receipt = receipt(consent, folder).unwrap_or_default();
    let mut problems = Vec::new();
    for install in planned {
        let same = |i: &Install| i.client == install.client && i.file == install.file;
        if let Some(old) = receipt.installs.iter().find(|i| same(i)) {
            if let Err(problem) = undo(old) {
                problems.push(problem);
                continue;
            }
        }
        receipt.installs.retain(|i| !same(i));
        match apply(folder, &install) {
            Ok(true) => receipt.installs.push(install),
            Ok(false) => {}
            Err(problem) => problems.push(problem),
        }
    }
    save(consent, folder, "engage", &receipt)?;
    Ok(problems)
}
/// Turn it off: take out everything the receipt lists, then forget the receipt and the ledger.
/// Returns what could not be taken out; the receipt is kept in that case so it can be retried.
pub fn turn_off(consent: &Path, folder: &Path) -> Result<Vec<String>, String> {
    let Some(mut receipt) = receipt(consent, folder) else {
        return Ok(Vec::new());
    };
    let mut problems = Vec::new();
    receipt.installs.retain(|install| match undo(install) {
        Ok(()) => false,
        Err(problem) => {
            problems.push(problem);
            true
        }
    });
    if problems.is_empty() {
        for kind in ["engage", "ledger"] {
            let _ = fs::remove_file(state_file(consent, folder, kind)?);
        }
    } else {
        save(consent, folder, "engage", &receipt)?;
    }
    Ok(problems)
}

/// What has happened in a folder since the last share. Kept per folder, not per conversation: the
/// bridge and the client's hooks are separate processes and share no conversation identity.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Ledger {
    /// The client conversation the rest belongs to. A new one starts clean.
    session: String,
    /// Workspace-relative files edited and not yet shared.
    pub edited: BTreeSet<String>,
    /// The agent has been sent back once for the current `edited`.
    nudged: bool,
    /// Notices this conversation has already been shown.
    pub seen: BTreeSet<String>,
    /// Files already looked up in this conversation.
    looked_up: BTreeSet<String>,
}
impl Ledger {
    pub fn load(consent: &Path, folder: &Path) -> Self {
        load(consent, folder, "ledger").unwrap_or_default()
    }
    pub fn save(&self, consent: &Path, folder: &Path) {
        // Best effort: a ledger that cannot be written only costs a reminder.
        let _ = save(consent, folder, "ledger", self);
    }
    /// Start clean for a conversation other than the one recorded. Edits an earlier conversation
    /// left unshared are dropped: a new conversation could not describe them truthfully.
    pub fn conversation(&mut self, session: Option<&str>) {
        let session = session.unwrap_or_default();
        if self.session != session {
            *self = Self {
                session: session.to_string(),
                ..Self::default()
            };
        }
    }
    pub fn edit(&mut self, file: String) {
        if self.edited.len() < MAX_EDITED {
            self.edited.insert(file);
        }
        self.nudged = false;
    }
    /// Whether `file` still needs a lookup in this conversation.
    pub fn first_look(&mut self, file: &str) -> bool {
        self.looked_up.len() < MAX_EDITED && self.looked_up.insert(file.to_string())
    }
    pub fn shared(&mut self) {
        self.edited.clear();
        self.nudged = false;
    }
    fn files(&self) -> String {
        let shown: Vec<&str> = self.edited.iter().take(8).map(String::as_str).collect();
        let more = self.edited.len() - shown.len();
        let mut text = shown.join(", ");
        if more > 0 {
            text.push_str(&format!(", and {more} more"));
        }
        text
    }
    /// The `next` hint for a tool result while edits are waiting to be shared.
    pub fn reminder(&self) -> Option<String> {
        (!self.edited.is_empty()).then(|| {
            format!(
                "Files edited in this folder have not been shared with the team yet: {}. When the task is finished, call knowledge.share_session with what was done and these files.",
                self.files()
            )
        })
    }
    /// What a stop hook answers. `Some` sends the agent back to share, at most once for the
    /// edits made so far and never when the client says this stop already follows a hook.
    pub fn stop(&mut self, client: &str, payload: &Value) -> Option<Value> {
        let follows_a_hook = match client {
            "claude-code" => payload["stop_hook_active"] != false,
            "cursor" => {
                payload["status"] != "completed" || payload["loop_count"].as_u64() != Some(0)
            }
            _ => true,
        };
        if self.edited.is_empty() || self.nudged || follows_a_hook {
            return None;
        }
        self.nudged = true;
        let text = format!(
            "Workspace Knowledge: automatic sharing is on for this project folder, and these edited files have not been shared with the team yet: {}. If the task is finished, call the Workspace Knowledge tool knowledge.share_session now with a faithful, secret-free summary (goal, decisions and why, constraints, open questions) and these files, then stop. If the task is not finished, or the developer asked to keep this work private, say so in one line and stop. You will not be reminded again until more files change.",
            self.files()
        );
        Some(match client {
            "claude-code" => json!({"decision":"block","reason":text}),
            _ => json!({"followup_message":text}),
        })
    }
}
/// Whether a workspace-relative `file` lies in one of the folder's approved service folders.
pub fn in_services(file: &str, services: &[String]) -> bool {
    services.iter().any(|s| {
        file.strip_prefix(s.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
    })
}
/// Where the client says a hook event happened, most specific first.
pub fn hook_folders(payload: &Value, project_dir: Option<String>) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = project_dir.into_iter().map(PathBuf::from).collect();
    folders.extend(
        payload["workspace_roots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(PathBuf::from),
    );
    folders.extend(payload["cwd"].as_str().map(PathBuf::from));
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(dir: &Path) -> HookCommand {
        HookCommand {
            exe: dir.join("bin/wk-mcp"),
            server: "https://wk.example".into(),
            product: "demo".into(),
            consent: None,
            ca_file: None,
        }
    }
    struct World {
        _dir: tempfile::TempDir,
        folder: PathBuf,
        consent: PathBuf,
    }
    fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (folder, consent) = (root.join("product"), root.join("consent"));
        fs::create_dir_all(folder.join("backend")).unwrap();
        fs::create_dir(&consent).unwrap();
        World {
            _dir: dir,
            folder,
            consent,
        }
    }
    fn text(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn each_client_gets_the_strongest_form_it_supports() {
        let w = world();
        let c = command(&w.folder);
        let kinds = |client: Option<&str>| -> Vec<(PathBuf, &'static str)> {
            plan(client, &w.folder, &c)
                .unwrap()
                .into_iter()
                .map(|i| {
                    let file = i.file.strip_prefix(&w.folder).unwrap().to_path_buf();
                    let kind = match i.change {
                        Change::Hooks { .. } => "hooks",
                        Change::Block { .. } => "block",
                        Change::File { .. } => "file",
                    };
                    (file, kind)
                })
                .collect()
        };
        assert_eq!(
            kinds(Some("claude-code")),
            [(PathBuf::from(".claude/settings.local.json"), "hooks")]
        );
        assert_eq!(
            kinds(Some("cursor")),
            [
                (PathBuf::from(".cursor/hooks.json"), "hooks"),
                (
                    PathBuf::from(".cursor/rules/workspace-knowledge.mdc"),
                    "file"
                )
            ]
        );
        assert_eq!(
            kinds(Some("codex")),
            [(PathBuf::from("AGENTS.md"), "block")]
        );
        assert_eq!(
            kinds(Some("vscode")),
            [(PathBuf::from("AGENTS.md"), "block")]
        );
        assert!(kinds(Some("claude-desktop")).is_empty());
        assert!(kinds(None).is_empty());
    }
    #[test]
    fn the_hook_command_names_the_server_and_refuses_a_path_it_cannot_quote() {
        let w = world();
        let mut c = command(&w.folder);
        c.consent = Some(PathBuf::from("/private/consent dir"));
        let line = c.line("claude-code", "Stop").unwrap();
        let rest = "wk-mcp\" hook --client claude-code --event Stop --server \"https://wk.example\" --product \"demo\" --consent-dir \"/private/consent dir\"";
        assert!(line.starts_with('"') && line.ends_with(rest), "{line}");
        assert!(!line.contains("token") && !line.contains("Bearer"));
        for bad in [
            "/a/$(x)/wk-mcp",
            "/a/b\"c",
            "/a/b`c`",
            "/a/b'c",
            "/a/%PATH%/x",
        ] {
            c.exe = PathBuf::from(bad);
            assert!(c.line("claude-code", "Stop").is_err(), "{bad}");
        }
    }
    #[test]
    fn claude_hooks_only_watch_edits_and_cursor_hooks_have_the_cursor_shape() {
        let w = world();
        let claude = command(&w.folder).claude().unwrap();
        for event in ["PreToolUse", "PostToolUse"] {
            assert_eq!(claude["hooks"][event][0]["matcher"], CLAUDE_EDITS);
        }
        for event in ["UserPromptSubmit", "Stop"] {
            assert!(claude["hooks"][event][0].get("matcher").is_none());
            assert_eq!(claude["hooks"][event][0]["hooks"][0]["type"], "command");
        }
        let cursor = command(&w.folder).cursor().unwrap();
        assert_eq!(cursor["version"], 1);
        assert!(cursor["hooks"]["stop"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--client cursor --event stop"));
    }
    #[test]
    fn hooks_are_merged_into_existing_settings_and_removed_without_touching_the_rest() {
        let w = world();
        let settings = w.folder.join(".claude/settings.local.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let mine = json!({"permissions":{"allow":["Bash(ls)"]},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}]}});
        fs::write(&settings, mine.to_string()).unwrap();
        let planned = plan(Some("claude-code"), &w.folder, &command(&w.folder)).unwrap();
        assert!(turn_on(&w.consent, &w.folder, planned.clone())
            .unwrap()
            .is_empty());
        let after: Value = serde_json::from_str(&text(&settings)).unwrap();
        assert_eq!(after["permissions"], mine["permissions"]);
        assert_eq!(after["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(after["hooks"]["PreToolUse"].is_array());
        let saved = receipt(&w.consent, &w.folder).unwrap();
        assert!(saved.hooks_for(Some("claude-code")) && !saved.hooks_for(Some("cursor")));
        // Approving again does not add the entries twice.
        turn_on(&w.consent, &w.folder, planned).unwrap();
        let again: Value = serde_json::from_str(&text(&settings)).unwrap();
        assert_eq!(again, after);
        assert_eq!(receipt(&w.consent, &w.folder).unwrap().installs.len(), 1);

        assert!(turn_off(&w.consent, &w.folder).unwrap().is_empty());
        let left: Value = serde_json::from_str(&text(&settings)).unwrap();
        assert_eq!(left, mine);
        assert!(receipt(&w.consent, &w.folder).is_none());
    }
    #[test]
    fn a_settings_file_that_was_only_ours_is_deleted_and_one_with_comments_is_left_alone() {
        let w = world();
        let planned = plan(Some("claude-code"), &w.folder, &command(&w.folder)).unwrap();
        turn_on(&w.consent, &w.folder, planned.clone()).unwrap();
        let settings = w.folder.join(".claude/settings.local.json");
        assert!(settings.exists());
        turn_off(&w.consent, &w.folder).unwrap();
        assert!(!settings.exists());

        fs::write(&settings, "{\n  // mine\n  \"hooks\": {}\n}\n").unwrap();
        let problems = turn_on(&w.consent, &w.folder, planned).unwrap();
        assert!(problems[0].contains("not strict JSON"), "{problems:?}");
        assert!(text(&settings).contains("// mine"));
        // Automatic is still on; only this client's hooks are missing.
        let saved = receipt(&w.consent, &w.folder).unwrap();
        assert!(saved.installs.is_empty());
    }
    #[test]
    fn a_rule_block_is_added_replaced_and_removed_around_the_projects_own_text() {
        let w = world();
        let agents = w.folder.join("AGENTS.md");
        fs::write(&agents, "# Our rules\n\nUse tabs.\n").unwrap();
        let planned = plan(Some("codex"), &w.folder, &command(&w.folder)).unwrap();
        turn_on(&w.consent, &w.folder, planned.clone()).unwrap();
        let once = text(&agents);
        assert!(once.starts_with("# Our rules\n\nUse tabs.\n\n<!-- workspace-knowledge:start"));
        assert!(once.contains("Sharing is on for this project") && once.ends_with("end -->\n"));
        turn_on(&w.consent, &w.folder, planned).unwrap();
        assert_eq!(text(&agents), once);
        assert_eq!(once.matches(BLOCK_START).count(), 1);
        turn_off(&w.consent, &w.folder).unwrap();
        assert_eq!(text(&agents), "# Our rules\n\nUse tabs.\n");

        // A file that held nothing else goes away with the block.
        fs::remove_file(&agents).unwrap();
        let planned = plan(Some("vscode"), &w.folder, &command(&w.folder)).unwrap();
        turn_on(&w.consent, &w.folder, planned).unwrap();
        assert!(agents.exists());
        turn_off(&w.consent, &w.folder).unwrap();
        assert!(!agents.exists());
    }
    #[test]
    fn a_rule_file_the_developer_already_has_is_never_overwritten_or_deleted() {
        let w = world();
        let rule = w.folder.join(".cursor/rules/workspace-knowledge.mdc");
        fs::create_dir_all(rule.parent().unwrap()).unwrap();
        fs::write(&rule, "my own rule\n").unwrap();
        let planned = plan(Some("cursor"), &w.folder, &command(&w.folder)).unwrap();
        turn_on(&w.consent, &w.folder, planned).unwrap();
        assert_eq!(text(&rule), "my own rule\n");
        let saved = receipt(&w.consent, &w.folder).unwrap();
        assert_eq!(saved.installs.len(), 1, "only the hooks are ours");
        turn_off(&w.consent, &w.folder).unwrap();
        assert_eq!(text(&rule), "my own rule\n");
        assert!(!w.folder.join(".cursor/hooks.json").exists());
    }
    #[cfg(unix)]
    #[test]
    fn a_settings_directory_that_links_outside_the_folder_is_refused() {
        let w = world();
        let outside = w.consent.parent().unwrap().join("elsewhere");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, w.folder.join(".claude")).unwrap();
        let planned = plan(Some("claude-code"), &w.folder, &command(&w.folder)).unwrap();
        let problems = turn_on(&w.consent, &w.folder, planned).unwrap();
        assert!(
            problems[0].contains("outside the project folder"),
            "{problems:?}"
        );
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
    }
    #[test]
    fn the_agent_is_sent_back_once_per_batch_of_edits_and_never_in_a_loop() {
        let mut ledger = Ledger::default();
        let stop = json!({"stop_hook_active": false});
        assert!(
            ledger.stop("claude-code", &stop).is_none(),
            "nothing edited"
        );
        ledger.edit("backend/a.rs".into());
        let sent = ledger.stop("claude-code", &stop).unwrap();
        assert_eq!(sent["decision"], "block");
        assert!(sent["reason"].as_str().unwrap().contains("backend/a.rs"));
        assert!(ledger.stop("claude-code", &stop).is_none(), "only once");
        ledger.edit("backend/b.rs".into());
        assert!(
            ledger
                .stop("claude-code", &json!({"stop_hook_active": true}))
                .is_none(),
            "a stop that follows a hook is never blocked"
        );
        assert!(
            ledger.stop("claude-code", &json!({})).is_none(),
            "unknown counts as a loop"
        );
        assert!(ledger.stop("claude-code", &stop).is_some());
        ledger.edit("backend/c.rs".into());
        ledger.shared();
        assert!(
            ledger.stop("claude-code", &stop).is_none(),
            "shared work needs no reminder"
        );
        assert!(ledger.reminder().is_none());
    }
    #[test]
    fn cursor_gets_a_follow_up_message_only_on_a_first_completed_stop() {
        let mut ledger = Ledger::default();
        ledger.edit("backend/a.rs".into());
        for quiet in [
            json!({"status":"aborted","loop_count":0}),
            json!({"status":"completed","loop_count":1}),
            json!({"status":"completed"}),
        ] {
            assert!(ledger.stop("cursor", &quiet).is_none(), "{quiet}");
        }
        let sent = ledger
            .stop("cursor", &json!({"status":"completed","loop_count":0}))
            .unwrap();
        assert!(sent["followup_message"]
            .as_str()
            .unwrap()
            .contains("knowledge.share_session"));
        assert!(ledger.stop("other", &json!({})).is_none());
    }
    #[test]
    fn a_new_conversation_starts_clean_and_long_lists_are_shortened() {
        let mut ledger = Ledger::default();
        ledger.conversation(Some("one"));
        for n in 0..12 {
            ledger.edit(format!("backend/{n:02}.rs"));
        }
        assert!(ledger.first_look("backend/00.rs") && !ledger.first_look("backend/00.rs"));
        let hint = ledger.reminder().unwrap();
        assert!(hint.contains("backend/07.rs, and 4 more") && !hint.contains("backend/08.rs"));
        ledger.conversation(Some("one"));
        assert_eq!(ledger.edited.len(), 12, "same conversation keeps its edits");
        ledger.conversation(Some("two"));
        assert!(ledger.edited.is_empty() && ledger.first_look("backend/00.rs"));
    }
    #[test]
    fn the_ledger_survives_a_round_trip_and_a_damaged_one_reads_as_empty() {
        let w = world();
        let mut ledger = Ledger::default();
        ledger.conversation(Some("s"));
        ledger.edit("backend/a.rs".into());
        ledger.save(&w.consent, &w.folder);
        assert_eq!(Ledger::load(&w.consent, &w.folder), ledger);
        fs::write(state_file(&w.consent, &w.folder, "ledger").unwrap(), "{").unwrap();
        assert_eq!(Ledger::load(&w.consent, &w.folder), Ledger::default());
    }
    #[test]
    fn only_files_inside_an_approved_service_count() {
        let services = ["backend".to_string(), "web".to_string()];
        assert!(in_services("backend/src/a.rs", &services));
        assert!(in_services("web/x.ts", &services));
        for outside in ["backend", "backend2/a.rs", "README.md", "docs/backend/a.md"] {
            assert!(!in_services(outside, &services), "{outside}");
        }
    }
    #[test]
    fn a_hook_looks_for_its_folder_in_the_project_dir_then_roots_then_cwd() {
        let payload = json!({"cwd":"/c","workspace_roots":["/r1","/r2"]});
        assert_eq!(
            hook_folders(&payload, Some("/p".into())),
            [
                Path::new("/p"),
                Path::new("/r1"),
                Path::new("/r2"),
                Path::new("/c")
            ]
        );
        assert!(hook_folders(&json!({}), None).is_empty());
    }
}
