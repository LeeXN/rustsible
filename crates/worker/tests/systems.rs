use rustsible_protocol::{
    Hello, ModuleResult, Request, Response, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
struct Worker {
    child: Child,
    input: Option<ChildStdin>,
    output: ChildStdout,
}
impl Worker {
    fn new(env: &[(&str, String)]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustsible-worker"))
            .arg("--serve")
            .envs(env.iter().map(|(key, value)| (*key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let _: Hello = rustsible_protocol::read_frame(&mut output, MAX_RESPONSE_BYTES).unwrap();
        Self {
            child,
            input: Some(input),
            output,
        }
    }
    fn run(&mut self, module: &str, args: serde_json::Value, check: bool) -> ModuleResult {
        let mut request = Request::new(module, serde_yaml::to_value(args).unwrap(), check, 10000);
        request.managed = false;
        rustsible_protocol::write_frame(self.input.as_mut().unwrap(), &request, MAX_REQUEST_BYTES)
            .unwrap();
        let response: Response =
            rustsible_protocol::read_frame(&mut self.output, MAX_RESPONSE_BYTES).unwrap();
        response.validate().unwrap()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.wait();
    }
}
fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
#[test]
fn native_sysv_daemon_survives_next_synchronous_task_and_is_stopped_by_service() {
    let temp = tempfile::tempdir().unwrap();
    let pid = temp.path().join("pid");
    let log = temp.path().join("argv");
    script(
        &temp.path().join("service"),
        r#"#!/bin/sh
printf '%s\000' "$@" >> "$LOG"
case "$2" in
status)
    [ -s "$PID" ] || exit 3
    value=$(/bin/cat "$PID")
    [ -e "/proc/$value/stat" ] || exit 3
    read -r id comm state rest < "/proc/$value/stat"
    [ "$state" != Z ] || exit 3
    exit 0;;
start) /usr/bin/setsid /bin/sleep 60 </dev/null >/dev/null 2>&1 & printf '%s' "$!" > "$PID";;
stop) kill "$(/bin/cat "$PID")"; /bin/rm "$PID";;
*) exit 4;;
esac
"#,
    );
    let env = [
        ("PATH", temp.path().to_str().unwrap().into()),
        ("PID", pid.to_str().unwrap().into()),
        ("LOG", log.to_str().unwrap().into()),
    ];
    struct DaemonCleanup(std::path::PathBuf);
    impl Drop for DaemonCleanup {
        fn drop(&mut self) {
            if let Ok(pid) = fs::read_to_string(&self.0) {
                if let Ok(pid) = pid.parse::<i32>() {
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                    }
                }
            }
        }
    }
    let _cleanup = DaemonCleanup(pid.clone());
    let mut worker = Worker::new(&env);
    let result = worker.run(
        "service",
        json!({"name":"literal;not-a-shell","state":"started"}),
        true,
    );
    assert!(result.changed && !result.failed, "{}", result.msg);
    assert!(!pid.exists());
    let result = worker.run(
        "service",
        json!({"name":"literal;not-a-shell","state":"started"}),
        false,
    );
    assert!(result.changed && !result.failed, "{}", result.msg);
    let process = fs::read_to_string(&pid).unwrap().parse::<i32>().unwrap();
    let stopped = worker.run("command", json!({"cmd":"/bin/true"}), false);
    assert!(!stopped.failed);
    assert!(
        Path::new(&format!("/proc/{process}")).exists(),
        "later task killed an adopted service"
    );
    let repeat = worker.run(
        "service",
        json!({"name":"literal;not-a-shell","state":"started"}),
        false,
    );
    assert!(!repeat.failed && !repeat.changed, "{}", repeat.msg);
    let result = worker.run(
        "service",
        json!({"name":"literal;not-a-shell","state":"stopped"}),
        false,
    );
    assert!(result.changed && !result.failed, "{}", result.msg);
    assert!(!pid.exists());
    assert!(
        !worker
            .run(
                "service",
                json!({"name":"literal;not-a-shell","state":"stopped"}),
                false
            )
            .changed
    );
    assert!(fs::read(log)
        .unwrap()
        .windows(b"literal;not-a-shell\0".len())
        .any(|window| window == b"literal;not-a-shell\0"));
}
#[test]
fn cache_is_refreshed_before_candidate_query_and_one_transaction_receives_the_list() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("events");
    let candidate = temp.path().join("candidate");
    let version = temp.path().join("version");
    fs::write(&candidate, b"1").unwrap();
    fs::write(&version, b"1").unwrap();
    script(
        &temp.path().join("dpkg-query"),
        "#!/bin/sh\nprintf 'installed\\t%s' \"$(/bin/cat \"$VERSION\")\"\n",
    );
    script(&temp.path().join("apt-cache"),"#!/bin/sh\nprintf 'query\\n' >> \"$LOG\"\nprintf ' Candidate: %s\\n' \"$(/bin/cat \"$CANDIDATE\")\"\n");
    script(
        &temp.path().join("apt-get"),
        r#"#!/bin/sh
case "$*" in
*update*) printf 'refresh\n' >> "$LOG"; printf 2 > "$CANDIDATE";;
*install*) printf 'transaction\n' >> "$LOG"; printf '%s\000' "$@" > "$ARGV"; /bin/cat "$CANDIDATE" > "$VERSION"; if [ "${FAIL_TRANSACTION:-}" = yes ]; then printf partial-transaction-output; printf denied >&2; exit 100; fi;;
*) exit 9;;
esac
"#,
    );
    let argv = temp.path().join("argv");
    let env = [
        ("PATH", temp.path().to_str().unwrap().into()),
        ("LOG", log.to_str().unwrap().into()),
        ("VERSION", version.to_str().unwrap().into()),
        ("CANDIDATE", candidate.to_str().unwrap().into()),
        ("ARGV", argv.to_str().unwrap().into()),
    ];
    let mut worker = Worker::new(&env);
    let result = worker.run(
        "package",
        json!({"name":["one","two"],"state":"latest","update_cache":true}),
        true,
    );
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert!(!argv.exists());
    assert!(!fs::read_to_string(&log).unwrap().contains("refresh"));
    fs::write(&log, []).unwrap();
    let result = worker.run(
        "package",
        json!({"name":["one","two"],"state":"latest","update_cache":true}),
        false,
    );
    assert!(!result.failed && result.changed, "{}", result.msg);
    let events = fs::read_to_string(&log).unwrap();
    assert!(
        events.starts_with("refresh\nquery\nquery\ntransaction\n"),
        "{events}"
    );
    assert_eq!(events.matches("transaction").count(), 1);
    let args = fs::read(argv).unwrap();
    assert!(args.ends_with(b"--\0one\0two\0"));
    let result = worker.run(
        "package",
        json!({"name":["one","two"],"state":"latest"}),
        false,
    );
    assert!(!result.failed && !result.changed, "{}", result.msg);
    fs::write(&version, b"1").unwrap();
    fs::write(&candidate, b"2").unwrap();
    let mut failure_env = env.to_vec();
    failure_env.push(("FAIL_TRANSACTION", "yes".into()));
    let mut failing = Worker::new(&failure_env);
    let result = failing.run(
        "package",
        json!({"name":["one","two"],"state":"latest"}),
        false,
    );
    assert!(result.failed && result.changed, "{}", result.msg);
    assert_eq!(result.rc, Some(100));
    assert_eq!(result.stdout, "partial-transaction-output");
    assert_eq!(result.stderr, "denied");
}
#[test]
fn invalid_system_parameters_never_invoke_a_mutating_tool_and_user_check_is_read_only() {
    let mut worker = Worker::new(&[]);
    for args in [
        json!({"name":"irrelevant","state":"invalid","daemon_reload":true}),
        json!({"name":"-injected","daemon_reload":true}),
    ] {
        let result = worker.run("systemd", args, false);
        assert!(result.failed && !result.changed);
    }
    let name = Command::new("id").arg("-un").output().unwrap();
    let name = String::from_utf8(name.stdout).unwrap();
    let before = fs::read("/etc/passwd").unwrap();
    let result = worker.run(
        "user",
        json!({"name":name.trim(),"create_home":false}),
        true,
    );
    assert!(!result.failed && !result.changed, "{}", result.msg);
    let result = worker.run(
        "user",
        json!({"name":name.trim(),"comment":"check must not modify this","create_home":false}),
        true,
    );
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert_eq!(fs::read("/etc/passwd").unwrap(), before);
}

#[test]
fn manager_backends_use_cached_prediction_batch_transactions_and_reject_unsupported_combinations() {
    for manager in ["dnf", "yum", "pacman", "zypper"] {
        let temp = tempfile::tempdir().unwrap();
        let version = temp.path().join("version");
        fs::write(&version, b"1").unwrap();
        let log = temp.path().join("log");
        let fixture = r#"#!/bin/sh
printf '%s ' "${0##*/}" "$@" >> "$LOG"; printf '\n' >> "$LOG"
case "${0##*/}" in
rpm) /bin/cat "$VERSION";;
pacman)
    case "$1" in
    -Q) printf 'one %s\n' "$(/bin/cat "$VERSION")";;
    -Qu) if [ "$(/bin/cat "$VERSION")" = 1 ]; then printf 'one 1 -> 2\n'; else exit 1; fi;;
    -S) printf 2 > "$VERSION";;
    *) exit 8;;
    esac;;
dnf|yum)
    case "$*" in
    *check-update*) [ "$(/bin/cat "$VERSION")" = 2 ] || exit 100;;
    *makecache*) :;;
    *upgrade*) printf 2 > "$VERSION";;
    *) exit 8;;
    esac;;
zypper) case "$*" in *refresh*) :;;*) exit 8;;esac;;
esac
"#;
        script(&temp.path().join(manager), fixture);
        if manager != "pacman" {
            script(&temp.path().join("rpm"), fixture);
        }
        let env = [
            ("PATH", temp.path().to_str().unwrap().into()),
            ("VERSION", version.to_str().unwrap().into()),
            ("LOG", log.to_str().unwrap().into()),
        ];
        let mut worker = Worker::new(&env);
        if manager == "zypper" {
            let result = worker.run(
                "package",
                json!({"name":"one","state":"latest","update_cache":true}),
                false,
            );
            assert!(result.failed && !result.changed);
            assert!(!log.exists());
            continue;
        }
        let result = worker.run(
            "package",
            json!({"name":["one","two"],"state":"latest"}),
            true,
        );
        assert!(
            !result.failed && result.changed,
            "{}: {}",
            manager,
            result.msg
        );
        assert_eq!(fs::read(&version).unwrap(), b"1");
        let log_text = fs::read_to_string(&log).unwrap();
        assert!(!log_text.contains("makecache") && !log_text.contains(" -Sy"));
        if manager != "pacman" {
            assert!(log_text.contains("--cacheonly"));
        }
        let result = worker.run(
            "package",
            json!({"name":["one","two"],"state":"latest"}),
            false,
        );
        assert!(
            !result.failed && result.changed,
            "{}: {}",
            manager,
            result.msg
        );
        assert_eq!(fs::read(&version).unwrap(), b"2");
        assert!(
            !worker
                .run(
                    "package",
                    json!({"name":["one","two"],"state":"latest"}),
                    false
                )
                .changed
        );
        let log_text = fs::read_to_string(log).unwrap();
        assert_eq!(
            log_text
                .lines()
                .filter(|line| line.contains("upgrade") || line.contains(" -S "))
                .count(),
            1,
            "{log_text}"
        );
    }
}

#[test]
fn upstart_planning_and_failure_capabilities_are_explicit_and_use_argv() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let log = temp.path().join("log");
    script(
        &temp.path().join("initctl"),
        r#"#!/bin/sh
printf '%s ' "$@" >> "$LOG"; printf '\n' >> "$LOG"
case "$1" in
status) if [ -f "$STATE" ]; then printf 'fixture start/running, process 123\n'; else printf 'fixture stop/waiting\n'; fi;;
start) printf active > "$STATE";;
stop) /bin/rm -f "$STATE";;
*) exit 7;;
esac
"#,
    );
    let env = [
        ("PATH", temp.path().to_str().unwrap().into()),
        ("STATE", state.to_str().unwrap().into()),
        ("LOG", log.to_str().unwrap().into()),
    ];
    let mut worker = Worker::new(&env);
    let result = worker.run(
        "service",
        json!({"name":"fixture","state":"started","enabled":true}),
        false,
    );
    assert!(result.failed && !result.changed);
    assert!(!log.exists());
    let result = worker.run("service", json!({"name":"fixture","state":"started"}), true);
    assert!(!result.failed && result.changed);
    assert!(!state.exists());
    let result = worker.run(
        "service",
        json!({"name":"fixture","state":"started"}),
        false,
    );
    assert!(!result.failed && result.changed);
    assert!(state.exists());
    assert!(
        !worker
            .run(
                "service",
                json!({"name":"fixture","state":"started"}),
                false
            )
            .changed
    );
    let result = worker.run(
        "service",
        json!({"name":"fixture","state":"stopped"}),
        false,
    );
    assert!(!result.failed && result.changed);
    assert!(!state.exists());
}
