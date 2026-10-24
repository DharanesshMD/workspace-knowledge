//! The one-shot local approval page behind `workspace.request_approval`.
//!
//! A model can only ask for this page; the consent record is written solely by a human clicking
//! Approve in the browser. The page is served on a literal-loopback ephemeral port under an
//! unguessable single-use path, rejects any other Host, and requires a per-page CSRF token and a
//! same-origin `Origin` on the form POST. It needs no JavaScript and shows no credentials.
use std::{
    collections::HashMap,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, Semaphore},
};

/// How long the developer has to answer.
const TTL: Duration = Duration::from_secs(600);
/// How long a finished or expired page keeps answering "already used" before the port closes.
const LINGER: Duration = Duration::from_secs(60);
const MAX_HEAD: usize = 8192;
const MAX_BODY: usize = 2048;
const MAX_CONNECTIONS: usize = 8;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const EXPIRED: &str = "This approval request has expired. Ask your assistant to request it again.";
const USED: &str = "This approval link has already been used.";

#[derive(Clone)]
pub struct Settings {
    pub ttl: Duration,
    pub linger: Duration,
    /// Open the page in the system browser (best effort).
    pub open_browser: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            ttl: TTL,
            linger: LINGER,
            open_browser: true,
        }
    }
}
/// What the developer is asked to approve.
pub struct Proposal {
    pub folder: PathBuf,
    pub product: String,
    pub services: Vec<String>,
    /// The consent directory the binding will be written to.
    pub consent: PathBuf,
    /// Server origin, for the collector command shown after approval.
    pub server: String,
    /// A sentence describing an approval this one replaces, if any.
    pub replaces: Option<String>,
}
pub struct Started {
    pub url: String,
    pub expires_in_seconds: u64,
}
enum Outcome {
    Approved(PathBuf),
    Declined,
    Failed(String),
}
enum Phase {
    Waiting,
    Done(Instant),
}
struct Shared {
    proposal: Proposal,
    token: String,
    csrf: String,
    host: String,
    origin: String,
    ttl: Duration,
    deadline: Instant,
    phase: Mutex<Phase>,
    changed: Notify,
}
/// Holds at most one pending approval request.
pub struct Gate {
    settings: Settings,
    current: tokio::sync::Mutex<Option<Arc<Shared>>>,
}

/// 244 random bits from the operating system.
fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
/// Equality that does not stop at the first differing byte.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}
fn html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}
/// Quote for a POSIX shell only when needed, so a copied command is exact.
fn shell(text: &str) -> String {
    if !text.is_empty()
        && text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&c))
    {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}
/// The exact command that connects the collector for one approved service.
pub fn connect_command(binding: &Path, service: &str, server: &str) -> String {
    format!(
        "wk-agent device connect --binding {} --service {} --server {} --yes",
        shell(&binding.to_string_lossy()),
        shell(service),
        shell(server)
    )
}

struct Parsed {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
impl Parsed {
    /// The value of a header that appears exactly once.
    fn header(&self, name: &str) -> Option<&str> {
        let mut found = self
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name));
        match (found.next(), found.next()) {
            (Some((_, v)), None) => Some(v),
            _ => None,
        }
    }
}
struct Reply {
    status: u16,
    reason: &'static str,
    body: String,
}
impl Reply {
    fn ok(body: String) -> Self {
        Self {
            status: 200,
            reason: "OK",
            body,
        }
    }
    fn refused(status: u16, reason: &'static str, message: &str) -> Self {
        Self {
            status,
            reason,
            body: layout(
                "Not approved",
                &format!(
                    "<h1>Not approved</h1><p>{}</p><p class=\"muted\">Nothing was saved.</p>",
                    html(message)
                ),
            ),
        }
    }
    fn gone(message: &str) -> Self {
        Self::refused(410, "Gone", message)
    }
    fn bytes(&self) -> Vec<u8> {
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nReferrer-Policy: same-origin\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'\r\n\r\n{}",
            self.status,
            self.reason,
            self.body.len(),
            self.body
        )
        .into_bytes()
    }
}
fn form(body: &[u8]) -> Result<HashMap<String, String>, ()> {
    let text = std::str::from_utf8(body).map_err(|_| ())?;
    let url = reqwest::Url::parse(&format!("http://form.invalid/?{text}")).map_err(|_| ())?;
    let mut fields = HashMap::new();
    for (k, v) in url.query_pairs() {
        // A repeated field is ambiguous, so the whole form is refused.
        if fields.insert(k.into_owned(), v.into_owned()).is_some() {
            return Err(());
        }
    }
    Ok(fields)
}
fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<style>
:root{{color-scheme:light dark;--bg:#fbfaf7;--fg:#1d1c1a;--muted:#6b675f;--line:#dcd8cf;--accent:#1f5f4a;--accent-fg:#fff;--panel:#fff}}
@media (prefers-color-scheme:dark){{:root{{--bg:#16181a;--fg:#ecebe7;--muted:#9c9a93;--line:#34383b;--accent:#5fbf9f;--accent-fg:#0d1512;--panel:#1d2022}}}}
body{{margin:0;background:var(--bg);color:var(--fg);font:16px/1.55 system-ui,-apple-system,"Segoe UI",sans-serif}}
main{{max-width:40rem;margin:0 auto;padding:2rem 1rem 3rem}}
h1{{font-size:1.5rem;margin:0 0 .5rem}}h2{{font-size:1.05rem;margin:1.6rem 0 .4rem}}
.lede{{margin:.25rem 0 1.25rem}}.muted{{color:var(--muted)}}.small{{font-size:.875rem}}
dl{{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:.75rem 1rem;margin:0 0 1rem}}
dt{{font-size:.8rem;text-transform:uppercase;letter-spacing:.04em;color:var(--muted);margin-top:.5rem}}dd{{margin:0}}
dd ul{{margin:.1rem 0;padding-left:1.1rem}}
code{{font:.9em ui-monospace,Menlo,Consolas,monospace;overflow-wrap:anywhere;user-select:all}}
.note{{border-left:3px solid var(--accent);padding:.25rem .75rem;margin:1rem 0}}
li{{margin:.3rem 0}}
.actions{{display:flex;gap:.75rem;margin:1.5rem 0 1rem;flex-wrap:wrap}}
button{{font:inherit;padding:.6rem 1.4rem;border-radius:8px;border:1px solid var(--line);background:var(--panel);color:var(--fg);cursor:pointer}}
button.primary{{background:var(--accent);color:var(--accent-fg);border-color:var(--accent);font-weight:600}}
button:focus-visible{{outline:3px solid var(--accent);outline-offset:2px}}
</style></head><body><main>
{body}
</main></body></html>"#
    )
}

impl Shared {
    fn pending(&self) -> bool {
        Instant::now() < self.deadline && matches!(*self.phase.lock().unwrap(), Phase::Waiting)
    }
    fn seconds_left(&self) -> u64 {
        self.deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
    }
    /// When the listener may close.
    fn end(&self, linger: Duration) -> Instant {
        match *self.phase.lock().unwrap() {
            Phase::Waiting => self.deadline + linger,
            Phase::Done(at) => at + linger,
        }
    }
    fn route(&self, request: &Parsed) -> Reply {
        if request.header("host") != Some(self.host.as_str()) {
            return Reply::refused(
                421,
                "Misdirected Request",
                "This page answers only at the address your assistant gave you.",
            );
        }
        let token_ok = request
            .path
            .strip_prefix("/a/")
            .is_some_and(|token| same(token, &self.token));
        if !token_ok {
            return Reply::refused(404, "Not Found", "Not found.");
        }
        match request.method.as_str() {
            "GET" => self.show(),
            "POST" => self.decide(request),
            _ => Reply::refused(405, "Method Not Allowed", "Method not allowed."),
        }
    }
    fn show(&self) -> Reply {
        match &*self.phase.lock().unwrap() {
            Phase::Waiting if Instant::now() < self.deadline => Reply::ok(self.page_form()),
            Phase::Waiting => Reply::gone(EXPIRED),
            Phase::Done(_) => Reply::gone(USED),
        }
    }
    fn decide(&self, request: &Parsed) -> Reply {
        if request.header("origin") != Some(self.origin.as_str()) {
            return Reply::refused(403, "Forbidden", "The request did not come from this page.");
        }
        let form_encoded = request.header("content-type").is_some_and(|t| {
            t.split(';').next().is_some_and(|m| {
                m.trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        });
        let fields = match form(&request.body) {
            Ok(fields) if form_encoded => fields,
            _ => return Reply::refused(400, "Bad Request", "The form could not be read."),
        };
        if !fields.get("csrf").is_some_and(|c| same(c, &self.csrf)) {
            return Reply::refused(
                403,
                "Forbidden",
                "The form is missing its security token. Reload the approval page.",
            );
        }
        let approve = match fields.get("decision").map(String::as_str) {
            Some("approve") => true,
            Some("decline") => false,
            _ => return Reply::refused(400, "Bad Request", "Choose Approve or Decline."),
        };
        // Check, decide and record under one lock so a link works exactly once.
        let mut phase = self.phase.lock().unwrap();
        if matches!(*phase, Phase::Done(_)) {
            return Reply::gone(USED);
        }
        if Instant::now() >= self.deadline {
            return Reply::gone(EXPIRED);
        }
        let outcome = if approve {
            match self.commit() {
                Ok(file) => Outcome::Approved(file),
                Err(reason) => Outcome::Failed(reason),
            }
        } else {
            Outcome::Declined
        };
        let reply = Reply::ok(self.page_outcome(&outcome));
        *phase = Phase::Done(Instant::now());
        drop(phase);
        self.changed.notify_one();
        reply
    }
    /// Write the consent record, applying the same validation as `wk workspace approve`.
    fn commit(&self) -> Result<PathBuf, String> {
        let p = &self.proposal;
        if p.folder.canonicalize().ok().as_deref() != Some(p.folder.as_path()) {
            return Err("the project folder changed since it was requested".into());
        }
        wk_scope::consent::folder_apart(&p.consent, &p.folder)?;
        wk_scope::consent::approve(Some(&p.consent), &p.folder, &p.product, p.services.clone())
            .map(|approved| approved.binding_file)
    }
    fn page_form(&self) -> String {
        let p = &self.proposal;
        let services: String = p
            .services
            .iter()
            .map(|s| {
                format!(
                    "<li><strong>{}</strong> <span class=\"muted\">in</span> <code>{}</code></li>",
                    html(s),
                    html(&p.folder.join(s).to_string_lossy())
                )
            })
            .collect();
        let replaces = p.replaces.as_ref().map_or(String::new(), |r| {
            format!("<p class=\"note\">{}</p>", html(r))
        });
        let body = format!(
            r#"<h1>Approve this project folder?</h1>
<p class="lede">Your AI assistant asked to connect this folder to Workspace Knowledge. Nothing is approved until you click Approve.</p>
<dl>
<dt>Folder</dt><dd><code>{folder}</code></dd>
<dt>Product</dt><dd>{product}</dd>
<dt>Services</dt><dd><ul>{services}</ul></dd>
</dl>
{replaces}
<h2>What approving means</h2>
<ul>
<li>Your conversation text with AI assistants stays on this computer. It is captured only for work inside the service folders listed above, and nowhere else in this project.</li>
<li>Short summaries of what you decided can be shared with teammates in these services. They are screened for secrets first.</li>
<li>Depending on your team's settings, a summary is shared either after you review it, or, if your team allows publishing without review, as soon as your assistant writes it. You can turn that off any time with <code>wk-agent auto-publish disable</code>.</li>
<li>You can undo this later: <code>wk workspace pause</code> stops it for now and <code>wk workspace revoke</code> deletes the approval.</li>
</ul>
<form method="post">
<input type="hidden" name="csrf" value="{csrf}">
<div class="actions"><button type="submit" name="decision" value="approve" class="primary">Approve</button><button type="submit" name="decision" value="decline">Decline</button></div>
</form>
<p class="muted small">This page is served only on this computer, works once, and expires {minutes} minutes after it was requested.</p>"#,
            folder = html(&p.folder.to_string_lossy()),
            product = html(&p.product),
            csrf = html(&self.csrf),
            minutes = (self.ttl.as_secs() / 60).max(1),
        );
        layout("Approve this project folder", &body)
    }
    fn page_outcome(&self, outcome: &Outcome) -> String {
        let p = &self.proposal;
        let body = match outcome {
            Outcome::Declined => "<h1>Declined</h1><p>Nothing was saved and nothing will be captured or shared. You can close this tab.</p>".to_string(),
            Outcome::Failed(reason) => format!(
                "<h1>Not approved</h1><p>The approval could not be saved: {}.</p><p class=\"muted\">Nothing was changed. You can close this tab.</p>",
                html(reason)
            ),
            Outcome::Approved(file) => {
                let commands: String = p
                    .services
                    .iter()
                    .map(|s| {
                        format!(
                            "<li><code>{}</code></li>",
                            html(&connect_command(file, s, &p.server))
                        )
                    })
                    .collect();
                let state = p.consent.to_string_lossy();
                let folder = p.folder.to_string_lossy();
                format!(
                    r#"<h1>Approved</h1>
<p>This computer now allows Workspace Knowledge to work inside these service folders of <code>{folder}</code>: <strong>{services}</strong>.</p>
<p>This is your local consent only. To start capturing, run this in a terminal, or ask your assistant to:</p>
<ul>{commands}</ul>
<p class="muted">To undo: <code>{pause}</code> pauses it, <code>{revoke}</code> deletes it. You can close this tab.</p>"#,
                    folder = html(&folder),
                    services = html(&p.services.join(", ")),
                    pause = html(&format!(
                        "wk --state-dir {} workspace pause {}",
                        shell(&state),
                        shell(&folder)
                    )),
                    revoke = html(&format!(
                        "wk --state-dir {} workspace revoke {}",
                        shell(&state),
                        shell(&folder)
                    )),
                )
            }
        };
        layout("Workspace Knowledge approval", &body)
    }
}

/// Read one request: head of at most 8 KiB and, for a POST, a form body of at most 2 KiB.
async fn read_request(stream: &mut TcpStream) -> Result<Parsed, ()> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buffer.len() > MAX_HEAD {
            return Err(());
        }
        let n = stream.read(&mut chunk).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        buffer.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&buffer[..head_end]).map_err(|_| ())?;
    let mut lines = head.split("\r\n");
    let first: Vec<&str> = lines.next().ok_or(())?.split(' ').collect();
    if first.len() != 3 || !matches!(first[2], "HTTP/1.1" | "HTTP/1.0") {
        return Err(());
    }
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(())?;
        if name.is_empty() || name.bytes().any(|c| c == b' ' || c == b'\t') {
            return Err(());
        }
        headers.push((name.to_string(), value.trim().to_string()));
    }
    let mut parsed = Parsed {
        method: first[0].to_string(),
        path: first[1].to_string(),
        headers,
        body: Vec::new(),
    };
    if parsed.header("transfer-encoding").is_some() {
        return Err(());
    }
    let length = match parsed.header("content-length") {
        None => 0,
        Some(v) => v.parse::<usize>().map_err(|_| ())?,
    };
    if length > MAX_BODY || (length > 0 && parsed.method != "POST") {
        return Err(());
    }
    let start = head_end + 4;
    while buffer.len() < start + length {
        let n = stream.read(&mut chunk).await.map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        buffer.extend_from_slice(&chunk[..n]);
    }
    parsed.body = buffer[start..start + length].to_vec();
    Ok(parsed)
}
async fn handle(mut stream: TcpStream, shared: Arc<Shared>) {
    let reply = match tokio::time::timeout(READ_TIMEOUT, read_request(&mut stream)).await {
        Ok(Ok(request)) => shared.route(&request),
        _ => Reply::refused(400, "Bad Request", "The request could not be read."),
    };
    let _ = stream.write_all(&reply.bytes()).await;
    let _ = stream.shutdown().await;
}
async fn serve(listener: TcpListener, shared: Arc<Shared>, linger: Duration) {
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let end = shared.end(linger);
        if Instant::now() >= end {
            break;
        }
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    if let Ok(permit) = permits.clone().try_acquire_owned() {
                        let shared = shared.clone();
                        tokio::spawn(async move {
                            handle(stream, shared).await;
                            drop(permit);
                        });
                    }
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            },
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(end)) => {}
            _ = shared.changed.notified() => {}
        }
    }
}

impl Gate {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            current: tokio::sync::Mutex::new(None),
        }
    }
    /// Seconds left on the pending request, if one is waiting for the developer.
    pub async fn waiting(&self) -> Option<u64> {
        let current = self.current.lock().await;
        current
            .as_ref()
            .filter(|s| s.pending())
            .map(|s| s.seconds_left())
    }
    /// Start the page for `proposal`. It approves nothing; only the developer's click does.
    /// Fails if another request is still waiting.
    pub async fn start(&self, proposal: Proposal) -> Result<Started, String> {
        let mut current = self.current.lock().await;
        if let Some(waiting) = current.as_ref().filter(|s| s.pending()) {
            return Err(format!(
                "an approval request for {} is already waiting for the developer ({} seconds left); ask them to answer it in their browser, or wait for it to expire",
                waiting.proposal.folder.display(),
                waiting.seconds_left()
            ));
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "cannot open the local approval page")?;
        let address = listener
            .local_addr()
            .map_err(|_| "cannot open the local approval page")?;
        let token = secret();
        let url = format!("http://{address}/a/{token}");
        let shared = Arc::new(Shared {
            proposal,
            token,
            csrf: secret(),
            host: address.to_string(),
            origin: format!("http://{address}"),
            ttl: self.settings.ttl,
            deadline: Instant::now() + self.settings.ttl,
            phase: Mutex::new(Phase::Waiting),
            changed: Notify::new(),
        });
        tokio::spawn(serve(listener, shared.clone(), self.settings.linger));
        *current = Some(shared);
        if self.settings.open_browser {
            let url = url.clone();
            // Best effort: the URL is also returned to the assistant to show the developer.
            tokio::task::spawn_blocking(move || {
                let _ = wk_login::open_browser(&url);
            });
        }
        Ok(Started {
            url,
            expires_in_seconds: self.settings.ttl.as_secs(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_and_shell_words_are_quoted() {
        assert_eq!(
            html("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert_eq!(shell("/Users/me/p"), "/Users/me/p");
        assert_eq!(shell("/Users/me/my project"), "'/Users/me/my project'");
        assert_eq!(shell("it's"), "'it'\\''s'");
        assert_eq!(shell(""), "''");
    }
    #[test]
    fn secrets_are_long_and_distinct_and_comparison_is_exact() {
        let (a, b) = (secret(), secret());
        assert!(a.len() >= 64 && a != b);
        assert!(same(&a, &a.clone()) && !same(&a, &b) && !same(&a, &a[1..]));
    }
    #[test]
    fn a_repeated_form_field_is_refused() {
        assert!(form(b"csrf=a&decision=approve").is_ok());
        assert!(form(b"decision=approve&decision=decline").is_err());
        assert!(form(&[0xff, 0xfe]).is_err());
    }

    // ---- the page, end to end against the real listener ----

    use crate::testkit::{answer, csrf, get, post, split};

    const FORM: &str = "application/x-www-form-urlencoded";

    struct Fixture {
        _dir: tempfile::TempDir,
        folder: PathBuf,
        consent: PathBuf,
    }
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("product").canonicalize_or_create();
        for service in ["backend", "web"] {
            std::fs::create_dir_all(folder.join(service)).unwrap();
        }
        let consent = dir.path().canonicalize().unwrap().join("consent");
        Fixture {
            _dir: dir,
            folder,
            consent,
        }
    }
    trait CanonicalOrCreate {
        fn canonicalize_or_create(&self) -> PathBuf;
    }
    impl CanonicalOrCreate for PathBuf {
        fn canonicalize_or_create(&self) -> PathBuf {
            std::fs::create_dir_all(self).unwrap();
            self.canonicalize().unwrap()
        }
    }
    impl Fixture {
        fn proposal(&self) -> Proposal {
            Proposal {
                folder: self.folder.clone(),
                product: "demo".into(),
                services: vec!["backend".into(), "web".into()],
                consent: self.consent.clone(),
                server: "https://wk.example".into(),
                replaces: None,
            }
        }
        fn nothing_written(&self) {
            assert!(
                !self.consent.exists(),
                "the consent directory must not even be created"
            );
        }
    }
    fn quick() -> Settings {
        Settings {
            open_browser: false,
            ..Settings::default()
        }
    }

    #[tokio::test]
    async fn approving_writes_a_binding_the_collector_accepts() {
        let f = fixture();
        let gate = Gate::new(quick());
        let started = gate.start(f.proposal()).await.unwrap();
        assert_eq!(started.expires_in_seconds, 600);
        assert!(
            started.url.starts_with("http://127.0.0.1:"),
            "{}",
            started.url
        );
        let (host, path) = split(&started.url);
        assert!(path.starts_with("/a/") && path.len() >= 3 + 64, "{path}");
        assert_eq!(gate.waiting().await.map(|s| s > 590), Some(true));

        let (status, page) = get(&host, &path, &host).await;
        assert_eq!(status, 200);
        let folder = f.folder.to_string_lossy();
        for needed in [
            folder.as_ref(),
            "demo",
            "backend",
            "web",
            "Approve",
            "Decline",
            "stays on this computer",
            "screened for secrets",
            "without review",
            "wk-agent auto-publish disable",
            "wk workspace revoke",
        ] {
            assert!(page.contains(needed), "page lacks {needed:?}");
        }
        assert!(!page.contains("<script"), "no JavaScript");
        assert!(
            !page.contains(&path[3..]),
            "the link token is not repeated in the page"
        );
        f.nothing_written();

        let (status, done) = answer(&started.url, "approve").await;
        assert_eq!(status, 200, "{done}");
        assert!(done.contains("<h1>Approved</h1>"), "{done}");
        let state = wk_scope::consent::state_dir(Some(&f.consent), &f.folder, false).unwrap();
        let binding = wk_scope::consent::read_binding(&state, &f.folder).unwrap();
        assert_eq!(binding.product, "demo");
        assert_eq!(binding.services, ["backend", "web"]);
        assert!(binding.eligible(&f.folder.join("backend")));
        assert!(binding.eligible(&f.folder.join("web")));
        assert!(!binding.eligible(&f.folder));
        let file = wk_scope::consent::binding_file(&state, &f.folder).unwrap();
        assert!(file.is_file());
        for service in ["backend", "web"] {
            let command = connect_command(&file, service, "https://wk.example");
            assert!(
                done.contains(&html(&command)),
                "the page shows the exact collector command: {command}"
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&f.consent).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "the consent directory is private");
        }
        assert_eq!(gate.waiting().await, None);
    }
    #[tokio::test]
    async fn declining_writes_nothing_and_spends_the_link() {
        let f = fixture();
        let gate = Gate::new(quick());
        let started = gate.start(f.proposal()).await.unwrap();
        let (host, path) = split(&started.url);
        let (_, form) = get(&host, &path, &host).await;
        let body = format!("csrf={}&decision=decline", csrf(&form));
        let origin = format!("http://{host}");
        let (status, page) = post(&host, &path, &host, Some(&origin), FORM, &body).await;
        assert_eq!(status, 200);
        assert!(page.contains("<h1>Declined</h1>"), "{page}");
        f.nothing_written();
        assert_eq!(gate.waiting().await, None);
        // Replaying the decline, or switching to approve with the same valid form, is refused.
        for replay in [body.clone(), body.replace("decline", "approve")] {
            let (status, _) = post(&host, &path, &host, Some(&origin), FORM, &replay).await;
            assert_eq!(status, 410);
        }
        f.nothing_written();
    }
    #[tokio::test]
    async fn every_malformed_or_foreign_request_is_refused_and_writes_nothing() {
        let f = fixture();
        let gate = Gate::new(quick());
        let started = gate.start(f.proposal()).await.unwrap();
        let (host, path) = split(&started.url);
        let origin = format!("http://{host}");
        let (_, page) = get(&host, &path, &host).await;
        let token = csrf(&page);
        let good = format!("csrf={token}&decision=approve");
        let port = host.rsplit(':').next().unwrap();

        // Wrong or truncated link token.
        let wrong = format!("/a/{}", "0".repeat(64));
        let short = &path[..path.len() - 1];
        for bad in [
            wrong.as_str(),
            short,
            "/",
            "/a/",
            "/a",
            &format!("{path}/"),
            &format!("{path}?x=1"),
        ] {
            assert_eq!(get(&host, bad, &host).await.0, 404, "GET {bad}");
            assert_eq!(
                post(&host, bad, &host, Some(&origin), FORM, &good).await.0,
                404,
                "POST {bad}"
            );
        }
        // Any Host other than the exact loopback address (DNS rebinding).
        for host_header in [
            "localhost".to_string(),
            format!("localhost:{port}"),
            format!("evil.example:{port}"),
            format!("127.0.0.1.evil.example:{port}"),
            "127.0.0.1".to_string(),
            format!("[::1]:{port}"),
            String::new(),
        ] {
            assert_eq!(
                get(&host, &path, &host_header).await.0,
                421,
                "GET Host {host_header:?}"
            );
            assert_eq!(
                post(&host, &path, &host_header, Some(&origin), FORM, &good)
                    .await
                    .0,
                421,
                "POST Host {host_header:?}"
            );
        }
        // Two Host headers are ambiguous.
        let doubled = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nHost: evil.example\r\n\r\n");
        assert_eq!(crate::testkit::raw(&host, &doubled).await.0, 421);
        // Origin: missing, opaque, another port, another host, another scheme.
        for origin_header in [
            None,
            Some("null".to_string()),
            Some(format!(
                "http://127.0.0.1:{}",
                port.parse::<u16>().unwrap().wrapping_add(1)
            )),
            Some(format!("http://localhost:{port}")),
            Some("https://evil.example".to_string()),
            Some(format!("https://{host}")),
            Some(format!("{origin}/")),
        ] {
            let (status, _) =
                post(&host, &path, &host, origin_header.as_deref(), FORM, &good).await;
            assert_eq!(status, 403, "Origin {origin_header:?}");
        }
        // CSRF: missing, wrong, empty, repeated; wrong encodings; unknown or missing decision.
        let other = secret();
        for (content_type, body, expect) in [
            (FORM, "decision=approve".to_string(), 403),
            (FORM, format!("csrf={other}&decision=approve"), 403),
            (FORM, "csrf=&decision=approve".to_string(), 403),
            (
                FORM,
                format!("csrf={token}&csrf={token}&decision=approve"),
                400,
            ),
            (
                FORM,
                format!("csrf={token}&decision=approve&decision=decline"),
                400,
            ),
            (FORM, format!("csrf={token}"), 400),
            (FORM, format!("csrf={token}&decision=APPROVE"), 400),
            (FORM, format!("csrf={token}&decision=yes"), 400),
            ("text/plain", good.clone(), 400),
            (
                "application/json",
                format!("{{\"csrf\":\"{token}\",\"decision\":\"approve\"}}"),
                400,
            ),
        ] {
            let (status, _) = post(&host, &path, &host, Some(&origin), content_type, &body).await;
            assert_eq!(status, expect, "{content_type} {body}");
        }
        // Oversized and malformed requests.
        let big = format!("csrf={token}&decision=approve&pad={}", "a".repeat(4096));
        let (status, _) = post(&host, &path, &host, Some(&origin), FORM, &big).await;
        assert!(matches!(status, 0 | 400), "oversized form: {status}");
        // Not HTTP at all, or no Host: refused before routing.
        for junk in [
            "BREW /coffee\r\n\r\n",
            "GET\r\n\r\n",
            "\r\n\r\n",
            "GET / HTTP/2\r\n\r\n",
        ] {
            assert_eq!(crate::testkit::raw(&host, junk).await.0, 400, "{junk:?}");
        }
        assert_eq!(
            crate::testkit::raw(&host, "BREW /coffee HTTP/1.1\r\n\r\n")
                .await
                .0,
            421
        );
        assert_eq!(
            crate::testkit::raw(
                &host,
                &format!("DELETE {path} HTTP/1.1\r\nHost: {host}\r\n\r\n")
            )
            .await
            .0,
            405
        );
        // Another path is not an approval endpoint.
        assert_eq!(get(&host, "/favicon.ico", &host).await.0, 404);

        f.nothing_written();
        // None of that burned the request: the developer's real click still works.
        assert!(gate.waiting().await.is_some());
        let (status, done) = post(&host, &path, &host, Some(&origin), FORM, &good).await;
        assert_eq!(status, 200, "{done}");
        assert!(done.contains("<h1>Approved</h1>"));
        assert!(f.consent.exists());
    }
    #[tokio::test]
    async fn a_link_works_exactly_once() {
        let f = fixture();
        let gate = Gate::new(quick());
        let started = gate.start(f.proposal()).await.unwrap();
        let (host, path) = split(&started.url);
        let (_, page) = get(&host, &path, &host).await;
        let body = format!("csrf={}&decision=approve", csrf(&page));
        let origin = format!("http://{host}");
        assert_eq!(
            post(&host, &path, &host, Some(&origin), FORM, &body)
                .await
                .0,
            200
        );
        // Delete what was written; a replay of the very same valid request must not rewrite it.
        let state = wk_scope::consent::state_dir(Some(&f.consent), &f.folder, false).unwrap();
        let file = wk_scope::consent::binding_file(&state, &f.folder).unwrap();
        std::fs::remove_file(&file).unwrap();
        let (status, replay) = post(&host, &path, &host, Some(&origin), FORM, &body).await;
        assert_eq!(status, 410, "{replay}");
        assert!(replay.contains("already been used"));
        assert!(!file.exists());
        let (status, again) = get(&host, &path, &host).await;
        assert_eq!(status, 410);
        assert!(!again.contains("csrf"), "a spent page shows no form");
    }
    #[tokio::test]
    async fn an_expired_request_is_refused_and_frees_the_slot() {
        let f = fixture();
        let gate = Gate::new(Settings {
            ttl: Duration::from_millis(250),
            linger: Duration::from_secs(30),
            open_browser: false,
        });
        let started = gate.start(f.proposal()).await.unwrap();
        let (host, path) = split(&started.url);
        let (_, page) = get(&host, &path, &host).await;
        let body = format!("csrf={}&decision=approve", csrf(&page));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(gate.waiting().await, None);
        let (status, expired) = post(
            &host,
            &path,
            &host,
            Some(&format!("http://{host}")),
            FORM,
            &body,
        )
        .await;
        assert_eq!(status, 410, "{expired}");
        assert!(expired.contains("expired"));
        assert_eq!(get(&host, &path, &host).await.0, 410);
        f.nothing_written();
        // A new request can start straight away.
        assert!(gate.start(f.proposal()).await.is_ok());
    }
    #[tokio::test]
    async fn only_one_request_can_wait_at_a_time() {
        let f = fixture();
        let gate = Gate::new(quick());
        let first = gate.start(f.proposal()).await.unwrap();
        let error = gate.start(f.proposal()).await.err().unwrap();
        assert!(error.contains("already waiting"), "{error}");
        assert!(
            !error.contains(&first.url),
            "the pending link is not repeated"
        );
        // Racing starts: exactly one wins.
        let gate = std::sync::Arc::new(Gate::new(quick()));
        let proposals: Vec<_> = (0..4).map(|_| f.proposal()).collect();
        let results = futures_join(gate.clone(), proposals).await;
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        // Declining frees the slot.
        let (status, _) = answer(&first.url, "decline").await;
        assert_eq!(status, 200);
        f.nothing_written();
    }
    async fn futures_join(
        gate: std::sync::Arc<Gate>,
        proposals: Vec<Proposal>,
    ) -> Vec<Result<Started, String>> {
        let tasks: Vec<_> = proposals
            .into_iter()
            .map(|p| {
                let gate = gate.clone();
                tokio::spawn(async move { gate.start(p).await })
            })
            .collect();
        let mut out = Vec::new();
        for task in tasks {
            out.push(task.await.unwrap());
        }
        out
    }
    #[tokio::test]
    async fn the_click_is_revalidated_so_a_stale_or_unsafe_request_writes_nothing() {
        // A service folder removed between the request and the click.
        let f = fixture();
        let gate = Gate::new(quick());
        let started = gate.start(f.proposal()).await.unwrap();
        std::fs::remove_dir(f.folder.join("web")).unwrap();
        let (status, page) = answer(&started.url, "approve").await;
        assert_eq!(status, 200);
        assert!(
            page.contains("<h1>Not approved</h1>") && page.contains("Nothing was changed"),
            "{page}"
        );
        f.nothing_written();

        // A consent directory inside the folder, or one that contains the folder.
        let f = fixture();
        for consent in [
            f.folder.join("backend/consent"),
            f.folder.join("consent"),
            f._dir.path().canonicalize().unwrap(),
        ] {
            let gate = Gate::new(quick());
            let mut proposal = f.proposal();
            proposal.consent = consent.clone();
            let started = gate.start(proposal).await.unwrap();
            let (_, page) = answer(&started.url, "approve").await;
            assert!(
                page.contains("<h1>Not approved</h1>"),
                "{consent:?}: {page}"
            );
            assert!(!consent.exists() || consent == f._dir.path().canonicalize().unwrap());
            assert!(!consent.join("consent").exists());
        }
        // A relative consent directory is refused by the shared validation.
        let gate = Gate::new(quick());
        let mut proposal = f.proposal();
        proposal.consent = PathBuf::from("relative/consent");
        let started = gate.start(proposal).await.unwrap();
        let (_, page) = answer(&started.url, "approve").await;
        assert!(page.contains("<h1>Not approved</h1>"), "{page}");
        assert!(!PathBuf::from("relative/consent").exists());
    }
    #[tokio::test]
    async fn page_text_is_escaped_so_a_hostile_folder_name_cannot_inject_markup() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("a<b>&\"'c").canonicalize_or_create();
        std::fs::create_dir(folder.join("backend")).unwrap();
        let gate = Gate::new(quick());
        let started = gate
            .start(Proposal {
                folder: folder.clone(),
                product: "demo".into(),
                services: vec!["backend".into()],
                consent: dir.path().canonicalize().unwrap().join("consent"),
                server: "https://wk.example".into(),
                replaces: Some("Replaces <b>x</b>.".into()),
            })
            .await
            .unwrap();
        let (host, path) = split(&started.url);
        let (_, page) = get(&host, &path, &host).await;
        assert!(!page.contains("a<b>"), "folder name must be escaped");
        assert!(page.contains("a&lt;b&gt;&amp;&quot;&#39;c"));
        assert!(!page.contains("<b>x</b>") && page.contains("&lt;b&gt;x&lt;/b&gt;"));
    }
}
