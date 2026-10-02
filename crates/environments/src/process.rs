use crate::{openbao::Values, Error, Result};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
};
/// Exact echoes are suppressed. Transformations, subprocesses and network exfiltration are not preventable.
pub fn redact(mut output: Vec<u8>, values: &Values) -> Vec<u8> {
    let mut secrets: Vec<&str> = values
        .values()
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .collect();
    secrets.sort_by_key(|v| std::cmp::Reverse(v.len()));
    for secret in secrets {
        let mut clean = Vec::new();
        let mut cursor = 0;
        while cursor < output.len() {
            if output[cursor..].starts_with(secret.as_bytes()) {
                clean.extend_from_slice(b"[REDACTED]");
                cursor += secret.len();
            } else {
                clean.push(output[cursor]);
                cursor += 1;
            }
        }
        output = clean;
    }
    output
}
fn drain(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        // Continue draining after cap to avoid pipe deadlock, but discard all output if truncated.
        if retained.len() <= 1_048_576 {
            retained.extend_from_slice(&buf[..n]);
        }
    }
    if retained.len() > 1_048_576 {
        Ok(b"[child output exceeded limit; suppressed]\n".to_vec())
    } else {
        Ok(retained)
    }
}
pub fn run(workspace: &Path, program: &str, args: &[String], values: &Values) -> Result<i32> {
    if program.is_empty() {
        return Err(Error::Invalid);
    }
    let mut child = Command::new(program)
        .args(args)
        .current_dir(workspace)
        .env_remove("WK_BAO_TOKEN")
        .env_remove("BAO_TOKEN")
        .env_remove("VAULT_TOKEN")
        .envs(values)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| Error::Local)?;
    let out = child.stdout.take().ok_or(Error::Local)?;
    let err = child.stderr.take().ok_or(Error::Local)?;
    let a = std::thread::spawn(move || drain(out));
    let b = std::thread::spawn(move || drain(err));
    let status = child.wait().map_err(|_| Error::Local)?;
    let out = a
        .join()
        .map_err(|_| Error::Local)?
        .map_err(|_| Error::Local)?;
    let err = b
        .join()
        .map_err(|_| Error::Local)?
        .map_err(|_| Error::Local)?;
    std::io::stdout()
        .write_all(&redact(out, values))
        .map_err(|_| Error::Local)?;
    std::io::stderr()
        .write_all(&redact(err, values))
        .map_err(|_| Error::Local)?;
    Ok(status.code().unwrap_or(1))
}
