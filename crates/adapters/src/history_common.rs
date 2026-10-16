//! Helpers shared by the Claude Code, Codex and Copilot history readers: bounded, symlink-safe
//! file reads, titles, RFC 3339 timestamps, the tag-wrapped part rule and `file://` URIs.
use crate::cursor_history::{History, Message};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Take};
use std::path::{Path, PathBuf};

/// Why a session file was not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// Not a regular file (a symlink, directory or device), or replaced between check and open.
    NotRegular,
    /// The file could not be opened or read.
    Unreadable,
    /// The file is larger than the reader's limit.
    TooLarge,
}

impl Refused {
    /// Count the refusal: oversized files as `skipped`, anything else as unreadable.
    pub(crate) fn count(self, history: &mut History) {
        match self {
            Refused::TooLarge => history.skipped += 1,
            Refused::NotRegular | Refused::Unreadable => history.unreadable_records += 1,
        }
    }
}

/// Open `file` only if it is a regular file (never a symlink) of at most `limit` bytes. The
/// handle reads at most `limit + 1` bytes, so a file that grows after the check stays bounded
/// and the caller can tell that it outgrew the limit.
///
/// The size comes from the opened handle. On Unix the handle must be the very file that was
/// checked (same device and inode), so a symlink swapped in between the check and the open is
/// refused. Residual races: on other platforms (Windows) a swap in that window is followed,
/// since refusing it needs `O_NOFOLLOW`-style flags not available without a new dependency; and
/// on every platform only the final component is checked, so a parent directory replaced by a
/// symlink after the directory walk is followed.
pub(crate) fn open_regular(file: &Path, limit: u64) -> Result<Take<File>, Refused> {
    let checked = std::fs::symlink_metadata(file).map_err(|_| Refused::Unreadable)?;
    if !checked.file_type().is_file() {
        return Err(Refused::NotRegular);
    }
    let handle = File::open(file).map_err(|_| Refused::Unreadable)?;
    let opened = handle.metadata().map_err(|_| Refused::Unreadable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (opened.dev(), opened.ino()) != (checked.dev(), checked.ino()) {
            return Err(Refused::NotRegular);
        }
    }
    if !opened.is_file() {
        return Err(Refused::NotRegular);
    }
    if opened.len() > limit {
        return Err(Refused::TooLarge);
    }
    Ok(handle.take(limit + 1))
}

/// The whole contents of `file`, read with [`open_regular`]'s checks and bound.
pub(crate) fn read_regular(file: &Path, limit: u64) -> Result<Vec<u8>, Refused> {
    let mut bytes = Vec::new();
    open_regular(file, limit)?
        .read_to_end(&mut bytes)
        .map_err(|_| Refused::Unreadable)?;
    if bytes.len() as u64 > limit {
        return Err(Refused::TooLarge);
    }
    Ok(bytes)
}

/// The lines of a file opened with [`open_regular`], never more than `limit` bytes in total.
pub(crate) struct Lines {
    reader: BufReader<Take<File>>,
    line: Vec<u8>,
    total: u64,
    limit: u64,
}

impl Lines {
    pub(crate) fn open(file: &Path, limit: u64) -> Result<Self, Refused> {
        Ok(Lines {
            reader: BufReader::new(open_regular(file, limit)?),
            line: Vec::new(),
            total: 0,
            limit,
        })
    }

    /// The next line including its terminator, or `None` at the end of the file. `TooLarge`
    /// once the file has grown past the limit, `Unreadable` on a read error.
    pub(crate) fn next_line(&mut self) -> Result<Option<&[u8]>, Refused> {
        self.line.clear();
        let n = self
            .reader
            .read_until(b'\n', &mut self.line)
            .map_err(|_| Refused::Unreadable)?;
        self.total += n as u64;
        if self.total > self.limit {
            return Err(Refused::TooLarge);
        }
        Ok((n > 0).then_some(self.line.as_slice()))
    }
}

/// The first non-blank line of `text`, trimmed, at most 80 characters.
pub(crate) fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(80).collect())
}

/// The first non-blank line of the first prompt, at most 80 characters.
pub(crate) fn title(messages: &[Message]) -> String {
    messages
        .iter()
        .find(|m| m.role == "user")
        .and_then(|m| first_line(&m.text))
        .unwrap_or_else(|| "Untitled chat".into())
}

/// Whether `s`, trimmed, is context a tool injected: one or more `<name attrs>…</name>` blocks
/// separated only by whitespace, at least one of them with a tool-style name containing `_` or
/// `-` (`ide_opened_file`, `task-notification`, `environment_context`). Tag names match
/// `[A-Za-z][A-Za-z0-9_-]*`. Parts made only of plain-named blocks such as `<task>` or
/// `<context>` are prompts a developer wrote and are kept, as is any part with other text.
pub(crate) fn injected_tag_blocks(s: &str) -> bool {
    let mut rest = s.trim();
    let mut tool_style = false;
    while !rest.is_empty() {
        let Some(open) = rest.strip_prefix('<') else {
            return false;
        };
        let len = open
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(open.len());
        let (name, after) = open.split_at(len);
        if !name.starts_with(|c: char| c.is_ascii_alphabetic())
            || !after.starts_with(|c: char| c == '>' || c.is_whitespace())
        {
            return false;
        }
        let Some(gt) = after.find('>') else {
            return false;
        };
        let body = &after[gt + 1..];
        let close = format!("</{name}>");
        let Some(end) = body.find(&close) else {
            return false;
        };
        tool_style |= name.contains(['_', '-']);
        rest = body[end + close.len()..].trim_start();
    }
    tool_style
}

/// The workspace a recorded working directory names, only when it is absolute. A relative
/// path would be resolved against wherever `wk` runs and could match the wrong workspace.
/// Rooted paths count as absolute: on Windows that also admits `\dir` paths without a drive,
/// though the agents record `C:\dir` in practice.
pub(crate) fn absolute_dir(cwd: Option<&str>) -> Option<PathBuf> {
    cwd.map(PathBuf::from).filter(|p| p.has_root())
}

/// A session id safe to use in conversation ids and file names: `candidate` if it matches
/// `[A-Za-z0-9._-]{1,128}` and is not `.` or `..`, else `file`'s stem under the same rule, else
/// the first 16 hex digits of the SHA-256 of `file`'s name.
pub(crate) fn session_id(candidate: Option<&str>, file: &Path) -> String {
    let safe = |s: &str| {
        (1..=128).contains(&s.len())
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    if let Some(id) = candidate.filter(|id| safe(id)) {
        return id.to_owned();
    }
    let stem = file.file_stem().unwrap_or_default().to_string_lossy();
    if safe(&stem) {
        return stem.into_owned();
    }
    let digest = Sha256::digest(file.file_name().unwrap_or_default().as_encoded_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Milliseconds since the Unix epoch for an RFC 3339 timestamp such as
/// `2026-10-15T10:00:00.250Z` or `2026-10-15T15:30:00+05:30`.
pub(crate) fn rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return None,
    };
    if !(1..=days_in_month).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    if second > 60 {
        return None;
    }
    let mut i = 19;
    let mut millis = 0;
    if b[i] == b'.' {
        let start = i + 1;
        i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        let digits: String = s[start..i].chars().chain("000".chars()).take(3).collect();
        millis = digits.parse().ok()?;
    }
    let offset_minutes = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let (h, m) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
            if h > 23 || m > 59 {
                return None;
            }
            let total = h * 60 + m;
            if *sign == b'-' {
                -total
            } else {
                total
            }
        }
        _ => return None,
    };
    // Days from civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset_minutes * 60;
    Some(seconds * 1_000 + millis)
}

/// The absolute local path of a `file:///…` URI with an empty or `localhost` authority,
/// percent-decoded. Anything else (other schemes, hosts, queries, bad escapes, `.` or `..`
/// components that would move the path) is `None`.
pub(crate) fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') || rest.contains(['?', '#']) {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let path = String::from_utf8(out).ok()?;
    if path.contains('\0') || path.split(['/', '\\']).any(|c| c == "." || c == "..") {
        return None;
    }
    // Windows folders look like `file:///c%3A/dir`.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if cfg!(windows) && drive.is_ascii_alphabetic() => &path[1..],
        _ => &path,
    };
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_tool_style_tag_blocks() {
        for injected in [
            "<a_b>x</a_b>",
            "<ide_selection path=\"p\">x <b>y</b></ide_selection>\n <c-d></c-d>",
            "<env_x attr=\"a>b\">x</env_x>",
            "<task>x</task>\n<task-notification>y</task-notification>",
        ] {
            assert!(injected_tag_blocks(injected), "{injected}");
        }
        for typed in [
            "",
            "plain",
            "<a>x</a>",
            "<task>Fix the limiter</task>",
            "<context>a</context> <div>b</div>",
            "<a_b>x</a_b> tail",
            "lead <a_b>x</a_b>",
            "<a_b>unclosed",
            "<a_b x=\"1\"",
            "<1a_b>x</1a_b>",
            "<a_b>x</b_a>",
            "< a_b>x</a_b>",
        ] {
            assert!(!injected_tag_blocks(typed), "{typed}");
        }
    }

    #[test]
    fn parses_rfc3339() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_ms("2026-10-15T10:00:00.000Z"),
            Some(1_792_058_400_000)
        );
        assert_eq!(
            rfc3339_ms("2026-10-15T15:30:00.250123+05:30"),
            Some(1_792_058_400_250)
        );
        assert_eq!(
            rfc3339_ms("2024-02-29T00:00:00-01:00"),
            Some(1_709_168_400_000)
        );
        for bad in [
            "",
            "2026-10-15",
            "2026-13-01T00:00:00Z",
            "2026-10-15T10:00:00",
            "x026-10-15T10:00:00Z",
            "2026-02-31T10:00:00Z",
            "2025-02-29T10:00:00Z",
            "2026-04-31T10:00:00Z",
        ] {
            assert_eq!(rfc3339_ms(bad), None, "{bad}");
        }
    }

    #[test]
    fn decodes_local_file_uris_only() {
        #[cfg(unix)]
        {
            let ok = |uri: &str| file_uri_path(uri).map(|p| p.to_string_lossy().into_owned());
            assert_eq!(ok("file:///work/my%20app").as_deref(), Some("/work/my app"));
            assert_eq!(
                ok("file://localhost/work/app").as_deref(),
                Some("/work/app")
            );
            assert_eq!(ok("file:///w/%E2%9C%93").as_deref(), Some("/w/\u{2713}"));
        }
        #[cfg(windows)]
        assert_eq!(
            file_uri_path("file:///c%3A/Users/me/app"),
            Some(PathBuf::from("c:/Users/me/app"))
        );
        for bad in [
            "",
            "/work/app",
            "file:/work/app",
            "file://server/share/app",
            "vscode-remote://ssh-remote%2Bbox/work/app",
            "file:///work/bad%2",
            "file:///work/bad%zz",
            "file:///work/%FF",
            "file:///work/nul%00",
            "file:///work/app?x=1",
            "file:///work/app#frag",
            "file:///work/../etc",
            "file:///work/./app",
            "file:///work/app/..",
            "file:///work/%2E%2E/etc",
            "file:///work/app%2F..%2Fx",
            "file:///work/app%5C..%5Cx",
        ] {
            assert_eq!(file_uri_path(bad), None::<PathBuf>, "{bad}");
        }
    }

    #[test]
    fn only_absolute_working_directories_are_workspaces() {
        assert_eq!(
            absolute_dir(Some("/work/app")),
            Some(PathBuf::from("/work/app"))
        );
        for bad in [
            None,
            Some(""),
            Some("work/app"),
            Some("./app"),
            Some("../app"),
        ] {
            assert_eq!(absolute_dir(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn session_ids_fall_back_to_the_stem_then_a_hash() {
        let file = Path::new("dir/stem.jsonl");
        assert_eq!(session_id(Some("Ab9._-x"), file), "Ab9._-x");
        assert_eq!(session_id(Some(&"x".repeat(128)), file).len(), 128);
        for bad in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "a:b",
            "a b",
            "é",
            &"x".repeat(129),
        ] {
            assert_eq!(session_id(Some(bad), file), "stem", "{bad:?}");
        }
        assert_eq!(session_id(None, file), "stem");
        let hashed = session_id(Some("../x"), Path::new("dir/bad name.jsonl"));
        assert_eq!(hashed.len(), 16);
        assert!(hashed.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(hashed, session_id(None, Path::new("dir/bad name2.jsonl")));
        assert_eq!(session_id(None, Path::new("..")).len(), 16);
    }

    #[test]
    fn titles_use_the_first_non_blank_prompt_line() {
        let m = |role: &str, text: &str| Message {
            id: String::new(),
            role: role.into(),
            text: text.into(),
        };
        assert_eq!(title(&[]), "Untitled chat");
        assert_eq!(
            title(&[m("assistant", "Hi"), m("user", "\n  First  \nsecond")]),
            "First"
        );
        assert_eq!(title(&[m("user", &"x".repeat(100))]).len(), 80);
    }
}
