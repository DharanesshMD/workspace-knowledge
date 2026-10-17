//! Masking of likely credentials in history text before it is shown or exported.
//!
//! Conservative and not exhaustive: it targets the shapes credentials take in prompts, replies
//! and pasted shell or config text, and prefers masking too much to masking too little.
//!
//! - PEM blocks are replaced whole.
//! - Long prefixed or opaque words (`sk-…`, `ghp_…`, JWTs, 32+ character tokens) are masked.
//! - The value of an assignment to a credential name is masked whatever its length, keeping the
//!   key: `key=value`, `key: value`, `"key": "value"`, `KEY=value`, `Authorization: Bearer v`.
//!   Names match case-insensitively as a suffix with separators ignored (`accessToken`,
//!   `PGPASSWORD`, `SECRET_KEY_BASE`); `pwd`, `auth` and `bearer` also need a word boundary,
//!   and `auth` only counts when a `Bearer`, `Basic` or `Token` scheme follows.
//! - Quoted values are masked through their closing quote; unquoted ones through whitespace.
//! - URL userinfo passwords, `sig`/`signature` query parameters and `curl -u user:pass` are
//!   masked.
//! - A credential key that ends its line with `:` or a YAML block indicator masks the more
//!   deeply indented lines that follow.
//!
//! Known limits: values continued on later lines in any other way (a JSON string split by the
//! client, a heredoc) and credentials with no recognisable name are not found.

const SUFFIX_KEYS: [&str; 16] = [
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "accesskey",
    "privatekey",
    "credential",
    "credentials",
    "sessionkey",
    "signingkey",
    "encryptionkey",
    "secretkey",
    "secretkeybase",
    "authorization",
];

/// Mask likely credentials before display or export. Conservative and not exhaustive.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_key_block = false;
    // Indentation of a credential key whose value continues on the more indented lines below.
    let mut block_indent: Option<usize> = None;
    for line in text.split_inclusive('\n') {
        if line.contains("-----BEGIN") {
            in_key_block = true;
        }
        if in_key_block {
            if line.contains("-----END") {
                in_key_block = false;
            }
            out.push_str("[REDACTED KEY MATERIAL]\n");
            continue;
        }
        let body = line.trim_end_matches(['\n', '\r']);
        let newline = &line[body.len()..];
        if let Some(indent) = block_indent {
            let lead = body.len() - body.trim_start().len();
            if lead == body.len() {
                out.push_str(line);
                continue;
            }
            if lead > indent {
                out.push_str(&body[..lead]);
                out.push_str("[REDACTED]");
                out.push_str(newline);
                continue;
            }
        }
        let words = body
            .split(' ')
            .map(|word| {
                if secret_like(word) {
                    "[REDACTED]"
                } else {
                    word
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        let masked = mask_assignments(&mask_flags(&mask_userinfo(&words)));
        out.push_str(&masked);
        out.push_str(newline);
        block_indent = block_key(body);
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Credential,
    /// `auth`: ambiguous in code, so only masked when a scheme and a credential follow.
    SchemeOnly,
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

fn is_quote(c: u8) -> bool {
    matches!(c, b'"' | b'\'' | b'`')
}

/// Whether the identifier `ident` names a credential. `query` is set for names that follow
/// `?` or `&`, where `sig` and `*signature` are credentials too.
fn classify(ident: &str, query: bool) -> Option<Kind> {
    let lower = ident.to_ascii_lowercase();
    let squashed: String = lower
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.'))
        .collect();
    if SUFFIX_KEYS.iter().any(|key| squashed.ends_with(key))
        || (query && (squashed == "sig" || squashed.ends_with("signature")))
    {
        return Some(Kind::Credential);
    }
    let b = ident.as_bytes();
    // A short name needs a boundary before it: a separator, or a camelCase hump.
    let bounded = |at: usize| {
        at == 0
            || matches!(b[at - 1], b'_' | b'-' | b'.')
            || ((b[at - 1].is_ascii_lowercase() || b[at - 1].is_ascii_digit())
                && b[at].is_ascii_uppercase())
    };
    [
        ("pwd", Kind::Credential),
        ("bearer", Kind::Credential),
        ("auth", Kind::SchemeOnly),
    ]
    .into_iter()
    .find(|(key, _)| lower.ends_with(key) && bounded(ident.len() - key.len()))
    .map(|(_, kind)| kind)
}

/// Mask the value of every credential assignment in `line`, keeping the key.
fn mask_assignments(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let (mut copied, mut i) = (0, 0);
    while i < b.len() {
        if !name_char(b[i] as char) || (i > 0 && name_char(b[i - 1] as char)) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && name_char(b[i] as char) {
            i += 1;
        }
        let query = start > 0 && matches!(b[start - 1], b'?' | b'&');
        let Some(kind) = classify(&line[start..i], query) else {
            continue;
        };
        if let Some((from, to)) = assigned_value(line, i, kind, query) {
            out.push_str(&line[copied..from]);
            out.push_str("[REDACTED]");
            (copied, i) = (to, to);
        }
    }
    out.push_str(&line[copied..]);
    out
}

enum Found {
    Value(usize, usize),
    /// A type annotation (`&str`, `Option<String>`) ending at the offset, not a value.
    Type(usize),
}

/// The byte range of the value assigned after a key ending at `key_end`, if there is one.
fn assigned_value(s: &str, key_end: usize, kind: Kind, query: bool) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let space = |mut i: usize| {
        while i < b.len() && matches!(b[i], b' ' | b'\t') {
            i += 1;
        }
        i
    };
    let mut i = key_end;
    // The key's own closing quote, plain or escaped inside embedded JSON.
    if b.get(i) == Some(&b'\\') && b.get(i + 1).is_some_and(|c| is_quote(*c)) {
        i += 2;
    } else if b.get(i).is_some_and(|c| is_quote(*c)) {
        i += 1;
    }
    loop {
        i = space(i);
        let sep = *b.get(i)?;
        if !matches!(sep, b'=' | b':') {
            return None;
        }
        // `==`, `auth::login` and `token://` are not assignments.
        let next = b.get(i + 1).copied();
        if next == Some(sep) || (sep == b':' && next == Some(b'/')) {
            return None;
        }
        let after = if sep == b'=' && next == Some(b'>') {
            i + 2
        } else {
            i + 1
        };
        match value(s, space(after), kind, query, sep == b':')? {
            Found::Value(from, to) => return Some((from, to)),
            Found::Type(end) => {
                // `NAME: &str = "value"`: the value follows the `=` after the type.
                let mut j = end;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b" _<>&':[]?".contains(&b[j]))
                {
                    j += 1;
                }
                if b.get(j) == Some(&b'=') && b.get(j + 1) != Some(&b'=') {
                    i = j;
                    continue;
                }
                return None;
            }
        }
    }
}

const SCHEMES: [&str; 3] = ["bearer", "basic", "token"];

/// The value starting at `v`: a quoted string (masked inside the quotes), or a run up to
/// whitespace or a quote, minus trailing commas.
fn value(s: &str, v: usize, kind: Kind, query: bool, allow_type: bool) -> Option<Found> {
    let b = s.as_bytes();
    let c = *b.get(v)?;
    if allow_type && s[v..].starts_with("&'") {
        // A lifetime (`&'static str`) is a type, whose quote must not read as a string.
        let end = s[v..].find(char::is_whitespace).map_or(b.len(), |p| v + p);
        return Some(Found::Type(end));
    }
    let (open, quote, escaped) = if c == b'\\' && b.get(v + 1).is_some_and(|q| is_quote(*q)) {
        (2, b[v + 1], true)
    } else if is_quote(c) {
        (1, c, false)
    } else {
        (0, 0, false)
    };
    if open > 0 {
        let start = v + open;
        let end = closing_quote(b, start, quote, escaped);
        let mut from = start;
        let word_end = s[start..end].find(' ').map_or(end, |p| start + p);
        let has_scheme = word_end < end
            && SCHEMES
                .iter()
                .any(|x| s[start..word_end].eq_ignore_ascii_case(x));
        if has_scheme {
            from = word_end + s[word_end..end].len() - s[word_end..end].trim_start().len();
        } else if kind == Kind::SchemeOnly {
            return None;
        }
        return (from < end).then_some(Found::Value(from, end));
    }
    let raw_end = (v..b.len())
        .find(|&j| {
            b[j].is_ascii_whitespace() || is_quote(b[j]) || (query && matches!(b[j], b'&' | b'#'))
        })
        .unwrap_or(b.len());
    let mut end = raw_end;
    while end > v && b[end - 1] == b',' {
        end -= 1;
    }
    if end == v {
        return None;
    }
    let token = &s[v..end];
    if is_block_indicator(token) {
        return None;
    }
    if allow_type && is_type(token.trim_end_matches(';')) {
        return Some(Found::Type(raw_end));
    }
    if b.get(raw_end) == Some(&b' ') && SCHEMES.iter().any(|x| token.eq_ignore_ascii_case(x)) {
        let next = raw_end + s[raw_end..].len() - s[raw_end..].trim_start().len();
        return match value(s, next, Kind::Credential, query, false)? {
            Found::Value(from, to) => Some(Found::Value(from, to)),
            Found::Type(_) => None,
        };
    }
    (kind == Kind::Credential).then_some(Found::Value(v, end))
}

/// The offset of the unescaped `quote` closing a value that starts at `start`, else the end.
fn closing_quote(b: &[u8], start: usize, quote: u8, escaped: bool) -> usize {
    let mut j = start;
    while j < b.len() {
        if escaped {
            if b[j] == b'\\' && b.get(j + 1) == Some(&quote) {
                return j;
            }
            j += 1;
        } else if b[j] == b'\\' {
            j += 2;
        } else if b[j] == quote {
            return j;
        } else {
            j += 1;
        }
    }
    b.len()
}

/// Whether `token` reads as a type annotation in `name: Type` rather than a value.
fn is_type(token: &str) -> bool {
    [
        "&str", "&String", "String", "str", "string", "bool", "boolean", "number", "Option",
    ]
    .contains(&token)
        || ["Option<", "Vec<", "Secret<", "&'"]
            .iter()
            .any(|p| token.starts_with(p))
}

/// A YAML block scalar header (`|`, `>-`, `|2+`): the value is on the lines below.
fn is_block_indicator(token: &str) -> bool {
    token.starts_with(['|', '>'])
        && token[1..]
            .chars()
            .all(|c| matches!(c, '+' | '-') || c.is_ascii_digit())
}

/// The indentation of a line like `password:` or `token: |` whose value continues below.
fn block_key(body: &str) -> Option<usize> {
    let mut t = body.trim_end();
    if let Some(p) = t.rfind(' ') {
        let tail = &t[p + 1..];
        if is_block_indicator(tail) {
            t = t[..p].trim_end();
        }
    }
    let t = t.strip_suffix(':')?.trim_end_matches(['"', '\'', '`']);
    let start = t
        .char_indices()
        .rev()
        .find(|(_, c)| !name_char(*c))
        .map_or(0, |(p, c)| p + c.len_utf8());
    (classify(&t[start..], false) == Some(Kind::Credential))
        .then(|| body.len() - body.trim_start().len())
}

/// Mask the password in `scheme://user:password@host`.
fn mask_userinfo(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(p) = rest.find("://") {
        let (head, tail) = rest.split_at(p + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>'))
            .unwrap_or(tail.len());
        let span = &tail[..end];
        let masked = span.rfind('@').and_then(|at| {
            let colon = span[..at].find(':')?;
            let (user, pass) = (&span[..colon], &span[colon + 1..at]);
            let digits = pass.len() - pass.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            // `host:8080/path@x` is a port and a path, not user and password.
            let port = digits > 0 && pass[digits..].starts_with(['/', '?', '#']);
            (!pass.is_empty() && !port && !user.contains(['/', '?', '#']))
                .then(|| format!("{}[REDACTED]{}", &span[..=colon], &span[at..]))
        });
        out.push_str(masked.as_deref().unwrap_or(span));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Mask the password of `curl -u user:password`, `--user`, `--proxy-user` and `-U`.
fn mask_flags(line: &str) -> String {
    const FLAGS: [&str; 4] = ["--proxy-user", "--user", "-u", "-U"];
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let (mut copied, mut i) = (0, 0);
    while i < b.len() {
        let flag = (b[i] == b'-' && (i == 0 || b[i - 1].is_ascii_whitespace()))
            .then(|| FLAGS.iter().find(|f| line[i..].starts_with(**f)))
            .flatten();
        let Some(flag) = flag else {
            i += 1;
            continue;
        };
        let after = i + flag.len();
        let arg = match b.get(after) {
            Some(b'=') if flag.starts_with("--") => after + 1,
            Some(c) if c.is_ascii_whitespace() => {
                after + line[after..].len() - line[after..].trim_start().len()
            }
            Some(_) if flag.len() == 2 => after,
            _ => {
                i += 1;
                continue;
            }
        };
        let (start, end) = match b.get(arg) {
            Some(q) if is_quote(*q) => (
                arg + 1,
                line[arg + 1..]
                    .find(*q as char)
                    .map_or(b.len(), |p| arg + 1 + p),
            ),
            _ => (
                arg,
                line[arg..]
                    .find(char::is_whitespace)
                    .map_or(b.len(), |p| arg + p),
            ),
        };
        if let Some(colon) = line[start..end].find(':').filter(|c| start + c + 1 < end) {
            out.push_str(&line[copied..start + colon + 1]);
            out.push_str("[REDACTED]");
            copied = end;
        }
        i = end.max(i + 1);
    }
    out.push_str(&line[copied..]);
    out
}

fn secret_like(word: &str) -> bool {
    let w = word.trim_matches(|c: char| "\"'`,;:()[]{}<>\n\r".contains(c));
    let w = w.rsplit_once('=').map_or(w, |(_, v)| v);
    let prefixed = [
        "sk-",
        "sk_",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "eyJ",
        "AIza",
    ]
    .iter()
    .any(|p| w.starts_with(p) && w.len() >= 16);
    let opaque = w.len() >= 32
        && !w.contains('/')
        && w.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_+=.".contains(c))
        && w.chars().any(|c| c.is_ascii_digit())
        && w.chars().any(|c| c.is_ascii_alphabetic());
    prefixed || opaque
}
