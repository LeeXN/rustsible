use anyhow::{bail, Context, Result};
use rustsible_protocol::{registry::Builtin, ModuleResult, Request};
use serde_yaml::Value;
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Read,
    net::{TcpStream, ToSocketAddrs},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::{Duration, Instant},
};

const MAX_MATCH_BYTES: u64 = 8 * 1024 * 1024;

pub fn execute(request: &Request) -> Result<ModuleResult> {
    Builtin::WaitFor.validate_parameters(&request.args)?;
    if request.check {
        return Ok(ModuleResult {
            skipped: true,
            msg: "Check mode: wait_for was skipped".into(),
            ..Default::default()
        });
    }
    let args = &request.args;
    let seconds = |name: &str, default| args.get(name).and_then(Value::as_u64).unwrap_or(default);
    let timeout = Duration::from_secs(seconds("timeout", 300));
    let connect_timeout = Duration::from_secs(seconds("connect_timeout", 5));
    let interval = Duration::from_secs(seconds("sleep", 1));
    let delay = Duration::from_secs(seconds("delay", 0));
    let state = args
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("started")
        .to_ascii_lowercase();
    let present = matches!(state.as_str(), "started" | "present");
    let path = args.get("path").and_then(Value::as_str);
    let port = args.get("port").and_then(Value::as_u64);
    let regex = args
        .get("search_regex")
        .and_then(Value::as_str)
        .map(|source| regex::RegexBuilder::new(source).multi_line(true).build())
        .transpose()?;
    let addresses = if let Some(port) = port {
        (
            args.get("host")
                .and_then(Value::as_str)
                .unwrap_or("127.0.0.1"),
            port as u16,
        )
            .to_socket_addrs()
            .context("Cannot resolve wait_for host")?
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let start = Instant::now();
    std::thread::sleep(delay);
    let deadline = start + timeout;
    if path.is_none() && port.is_none() {
        std::thread::sleep(timeout);
        return Ok(result(start, "Wait duration completed"));
    }
    loop {
        if Instant::now() >= deadline {
            bail!("wait_for timed out before its condition was satisfied");
        }
        let ready = if let Some(path) = path {
            match std::fs::metadata(path) {
                Ok(metadata) => {
                    if !present {
                        false
                    } else if let Some(regex) = &regex {
                        if !metadata.is_file() {
                            bail!("wait_for regex matching requires a regular file");
                        }
                        matches_file(Path::new(path), regex)?
                    } else {
                        present
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => !present,
                Err(error) => return Err(error).context("Cannot inspect wait_for path"),
            }
        } else {
            let mut open = false;
            for address in &addresses {
                let remaining = deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1));
                if TcpStream::connect_timeout(address, connect_timeout.min(remaining)).is_ok() {
                    open = true;
                    break;
                }
                if Instant::now() >= deadline {
                    break;
                }
            }
            open == present
        };
        if ready {
            return Ok(result(start, "Wait condition satisfied"));
        }
        if Instant::now() >= deadline {
            bail!("wait_for timed out before its condition was satisfied");
        }
        std::thread::sleep(interval.min(deadline.saturating_duration_since(Instant::now())));
    }
}
fn matches_file(path: &Path, regex: &regex::Regex) -> Result<bool> {
    // Nonblocking open plus an fd type check closes the metadata/open race
    // where a regular file could be replaced by a FIFO while polling.
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_MATCH_BYTES {
        bail!("wait_for matching input must be a regular file no larger than 8 MiB");
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_MATCH_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MATCH_BYTES {
        bail!("wait_for matching input exceeded 8 MiB");
    }
    let text = std::str::from_utf8(&bytes).context("wait_for matching requires UTF-8 text")?;
    Ok(regex.is_match(text))
}
fn result(start: Instant, message: &str) -> ModuleResult {
    ModuleResult {
        msg: message.into(),
        values: HashMap::from([(
            "elapsed".into(),
            Value::Number((start.elapsed().as_secs()).into()),
        )]),
        ..Default::default()
    }
}
