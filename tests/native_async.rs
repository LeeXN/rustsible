use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    AsyncSpec, Hello, ModuleResult, Request, Response, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
};
use serde_yaml::Value;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;
mod support;

fn worker() -> Result<PathBuf> {
    // Freeze once before any test can fork; another test's fork must not
    // inherit a writer to an executable that a peer is about to execute.
    let target =
        support::worker_bundle().join(format!("rustsible-worker-linux-{}", std::env::consts::ARCH));
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
    Ok(target)
}
fn invoke(worker: &Path, mut request: Request) -> Result<ModuleResult> {
    request.managed = false;
    let mut child = Command::new(worker)
        .arg("--serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().context("worker stdin")?;
    let mut output = child.stdout.take().context("worker stdout")?;
    let hello: Hello = rustsible_protocol::read_frame(&mut output, MAX_RESPONSE_BYTES)?;
    assert_eq!(hello.version, rustsible_protocol::VERSION);
    rustsible_protocol::write_frame(&mut input, &request, MAX_REQUEST_BYTES)?;
    let response: Response = rustsible_protocol::read_frame(&mut output, MAX_RESPONSE_BYTES)?;
    drop(input);
    let status = child.wait_with_output()?;
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    response.validate()
}
fn start(
    worker: &Path,
    base: &Path,
    jid: &str,
    script: String,
    timeout_ms: u64,
    no_log: bool,
) -> Result<ModuleResult> {
    let mut request = Request::new(
        "shell",
        Value::Mapping(serde_yaml::Mapping::from_iter([(
            Value::String("cmd".into()),
            Value::String(script),
        )])),
        false,
        10_000,
    );
    request.asynchronous = Some(AsyncSpec {
        jid: jid.into(),
        timeout_ms,
        directory: Some(base.to_string_lossy().into_owned()),
        no_log,
    });
    invoke(worker, request)
}
fn status(worker: &Path, base: &Path, jid: &str, mode: &str, check: bool) -> Result<ModuleResult> {
    let args = Value::Mapping(serde_yaml::Mapping::from_iter([
        (Value::String("jid".into()), Value::String(jid.into())),
        (Value::String("mode".into()), Value::String(mode.into())),
        (
            Value::String("_async_dir".into()),
            Value::String(base.to_string_lossy().into_owned()),
        ),
    ]));
    invoke(worker, Request::new("async_status", args, check, 10_000))
}
fn complete(worker: &Path, base: &Path, jid: &str) -> Result<ModuleResult> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let result = status(worker, base, jid, "status", false)?;
        if result.values.get("finished").and_then(Value::as_bool) == Some(true) {
            return Ok(result);
        }
        if result.failed || Instant::now() >= deadline {
            bail!("Async result was not confirmed: {}", result.msg);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn async_survives_worker_exit_and_keeps_private_atomic_state_until_explicit_cleanup() -> Result<()>
{
    let directory = TempDir::new()?;
    let worker = worker()?;
    let base = directory.path().join("jobs");
    let jid = format!("j{:064x}", 1);
    let gate = directory.path().join("gate");
    let destination = directory.path().join("completed");
    let launched = start(
        &worker,
        &base,
        &jid,
        format!(
            "while [ ! -f {} ]; do sleep 0.01; done; printf complete > {}; printf output",
            gate.display(),
            destination.display()
        ),
        10_000,
        false,
    )?;
    assert!(!launched.failed, "{}", launched.msg);
    assert_eq!(launched.values["started"], Value::Bool(true));
    assert_eq!(launched.values["finished"], Value::Bool(false));
    assert!(
        !destination.exists(),
        "job must await a signal created after its original worker exited"
    );
    let job = base.join(&jid);
    assert!(status(&worker, &base, &jid, "cleanup", false)?.failed);
    let duplicate = start(
        &worker,
        &base,
        &jid,
        format!("printf wrong > {}", destination.display()),
        5_000,
        false,
    )?;
    assert!(duplicate.failed);
    assert!(!destination.exists());
    assert_eq!(fs::metadata(&base)?.mode() & 0o7777, 0o700);
    assert_eq!(fs::metadata(&job)?.mode() & 0o7777, 0o700);
    assert_eq!(
        fs::metadata(job.join("status.json"))?.mode() & 0o7777,
        0o600
    );
    fs::write(gate, b"ready")?;
    let result = complete(&worker, &base, &jid)?;
    assert!(!result.failed && !result.uncertain, "{}", result.msg);
    assert_eq!(result.stdout, "output");
    assert_eq!(fs::read_to_string(destination)?, "complete");
    fs::set_permissions(job.join("status.json"), fs::Permissions::from_mode(0o644))?;
    assert!(status(&worker, &base, &jid, "status", false)?.failed);
    fs::set_permissions(job.join("status.json"), fs::Permissions::from_mode(0o600))?;
    assert!(status(&worker, &base, &jid, "cleanup", true)?.changed);
    assert!(job.join("status.json").exists());
    assert!(status(&worker, &base, &jid, "cleanup", false)?.changed);
    assert!(!job.exists());
    assert!(!status(&worker, &base, &jid, "cleanup", false)?.changed);
    Ok(())
}

#[test]
fn async_deadline_kills_detached_descendants_and_keeps_no_log_across_status_queries() -> Result<()>
{
    let directory = TempDir::new()?;
    let worker = worker()?;
    let base = directory.path().join("jobs");
    let jid = format!("j{:064x}", 2);
    let pid = directory.path().join("child-pid");
    let launched = start(
        &worker,
        &base,
        &jid,
        format!(
            "setsid /bin/sh -c 'echo $$ > {}; sleep 30' & wait",
            pid.display()
        ),
        2_000,
        true,
    )?;
    assert!(!launched.failed, "{}", launched.msg);
    let result = complete(&worker, &base, &jid)?;
    assert!(result.failed && !result.uncertain, "{}", result.msg);
    assert_eq!(result.values["timed_out"], Value::Bool(true));
    assert!(result.no_log);
    let child = fs::read_to_string(pid)?.trim().parse::<u32>()?;
    assert!(
        !Path::new(&format!("/proc/{child}")).exists(),
        "timeout left a detached descendant"
    );
    assert!(status(&worker, &base, &jid, "cleanup", false)?.changed);
    Ok(())
}

#[test]
fn async_corrupt_state_and_invalid_ids_never_leak_or_escape_the_job_directory() -> Result<()> {
    let directory = TempDir::new()?;
    let worker = worker()?;
    let base = directory.path().join("jobs");
    let jid = format!("j{:064x}", 3);
    let launched = start(
        &worker,
        &base,
        &jid,
        "printf ASYNC_PRIVATE_SENTINEL".into(),
        5_000,
        true,
    )?;
    assert!(!launched.failed, "{}", launched.msg);
    let result = complete(&worker, &base, &jid)?;
    assert_eq!(result.stdout, "ASYNC_PRIVATE_SENTINEL");
    assert!(result.no_log);
    assert!(!serde_json::to_string(&result.displayed())?.contains("ASYNC_PRIVATE_SENTINEL"));
    fs::write(
        base.join(&jid).join("status.json"),
        br#"{"version":"CORRUPT_PRIVATE_SENTINEL"}"#,
    )?;
    let corrupt = status(&worker, &base, &jid, "status", false)?;
    assert!(corrupt.failed);
    assert!(
        !corrupt.msg.contains("CORRUPT_PRIVATE_SENTINEL"),
        "{}",
        corrupt.msg
    );
    let outside = directory.path().join("outside");
    fs::write(&outside, b"keep")?;
    let invalid = status(&worker, &base, "../outside", "cleanup", false)?;
    assert!(invalid.failed);
    assert_eq!(fs::read(outside)?, b"keep");
    Ok(())
}
