use rustsible_protocol::{
    decode, encode, read_frame, Hello, ModuleResult, Outcome, Request, Response, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES,
};
use std::{
    fs,
    io::Write,
    process::{Child, ChildStdin, Command, Stdio},
    time::{Duration, Instant},
};

fn start(mut request: Request) -> (Child, ChildStdin) {
    request.managed = false;
    start_at(
        request,
        std::path::Path::new(env!("CARGO_BIN_EXE_rustsible-worker")),
    )
}
fn start_at(request: Request, binary: &std::path::Path) -> (Child, ChildStdin) {
    let mut child = Command::new(binary)
        .arg("--serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _: Hello = read_frame(child.stdout.as_mut().unwrap(), MAX_RESPONSE_BYTES).unwrap();
    let mut input = child.stdin.take().unwrap();
    input
        .write_all(&encode(&request, MAX_REQUEST_BYTES).unwrap())
        .unwrap();
    input.flush().unwrap();
    (child, input)
}
fn finish(mut child: Child, input: ChildStdin) -> ModuleResult {
    let response: Response =
        read_frame(child.stdout.as_mut().unwrap(), MAX_RESPONSE_BYTES).unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    response.validate().unwrap()
}
fn request(module: &str, args: serde_json::Value) -> Request {
    Request::new(module, serde_yaml::to_value(args).unwrap(), false, 30000)
}

#[test]
fn command_argv_stdin_working_directory_and_glob_guards_are_native() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("already.flag"), "original").unwrap();
    let run = |args, check| {
        let mut request = request("command", args);
        request.check = check;
        let (child, input) = start(request);
        finish(child, input)
    };
    let result = run(
        serde_json::json!({"argv":["/bin/cat"],"stdin":"a\nb","stdin_add_newline":false,"chdir":temp.path()}),
        false,
    );
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert_eq!(result.stdout, "a\nb");
    let result = run(
        serde_json::json!({"argv":["/usr/bin/printf","%s|%s","a b","; touch injected"],"chdir":temp.path()}),
        false,
    );
    assert_eq!(result.stdout, "a b|; touch injected");
    assert!(!temp.path().join("injected").exists());
    for check in [false, true] {
        for args in [
            serde_json::json!({"cmd":"touch changed","chdir":temp.path(),"creates":"*.flag"}),
            serde_json::json!({"cmd":"touch changed","chdir":temp.path(),"removes":"missing.*"}),
        ] {
            let result = run(args, check);
            assert!(
                !result.failed && !result.changed && !result.skipped,
                "{}",
                result.msg
            );
        }
    }
    let result = run(
        serde_json::json!({"cmd":"touch changed","chdir":temp.path(),"creates":"changed"}),
        true,
    );
    assert!(!result.failed && result.changed && !result.skipped);
    assert!(!temp.path().join("changed").exists());
    let result = run(
        serde_json::json!({"cmd":"touch changed","chdir":temp.path(),"creates":"missing-parent/child"}),
        true,
    );
    assert!(!result.failed && result.changed, "{}", result.msg);
    for args in [
        serde_json::json!({"cmd":"touch changed","argv":["true"]}),
        serde_json::json!({"argv":[]}),
        serde_json::json!({"cmd":"touch changed","stdin_add_newline":"no"}),
        serde_json::json!({"cmd":"touch changed","chdir":temp.path().join("missing")}),
    ] {
        let result = run(args, false);
        assert!(result.failed && !result.changed, "{}", result.msg);
    }
    assert!(!temp.path().join("changed").exists());
}

#[test]
fn task_environment_is_private_to_one_request_and_arguments_expand_without_a_shell() {
    let mut first = request(
        "command",
        serde_json::json!({"argv":["/usr/bin/printf", "%s|%s|%s\n", "$RUSTSIBLE_ENV_TEST", "${RUSTSIBLE_ENV_TEST}", "$RUSTSIBLE_MISSING_ENV_TEST"]}),
    );
    first
        .environment
        .insert("RUSTSIBLE_ENV_TEST".into(), "one word; literal".into());
    let (mut child, mut input) = start(first);
    let response: Response =
        read_frame(child.stdout.as_mut().unwrap(), MAX_RESPONSE_BYTES).unwrap();
    assert_eq!(
        response.validate().unwrap().stdout,
        "one word; literal|one word; literal|$RUSTSIBLE_MISSING_ENV_TEST"
    );
    let mut second = request(
        "shell",
        serde_json::json!({"cmd":"printf %s \"${RUSTSIBLE_ENV_TEST-unset}\""}),
    );
    second.managed = false;
    input
        .write_all(&encode(&second, MAX_REQUEST_BYTES).unwrap())
        .unwrap();
    input.flush().unwrap();
    assert_eq!(finish(child, input).stdout, "unset");
}
fn quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\"'\"'"))
}
fn wait_file(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while fs::metadata(path).map_or(true, |metadata| metadata.len() == 0) {
        assert!(Instant::now() < deadline, "worker did not create marker");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn handshake_and_linux_facts_are_native_and_match_the_target() {
    let output = Command::new(env!("CARGO_BIN_EXE_rustsible-worker"))
        .arg("--handshake")
        .output()
        .unwrap();
    assert!(output.status.success());
    let hello: Hello = decode(&output.stdout, MAX_RESPONSE_BYTES).unwrap();
    hello
        .validate(std::env::consts::ARCH, unsafe { libc::geteuid() }, "setup")
        .unwrap();
    let (child, input) = start(request("setup", serde_json::json!({})));
    let result = finish(child, input);
    assert_eq!(result.outcome(), Outcome::Ok);
    assert_eq!(
        result.values["ansible_facts"]["ansible_architecture"].as_str(),
        Some(std::env::consts::ARCH)
    );
    assert!(
        result.values["ansible_facts"]["ansible_date_time"]["epoch"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn fact_subsets_filters_and_normalized_names_match_the_declared_surface() {
    let args = serde_json::json!({"gather_subset":["!all","!min","architecture","distribution","network"],"filter":["ansible_architecture","ansible_os_family","ansible_interfaces","ansible_all_ipv[46]_addresses"]});
    let (child, input) = start(request("setup", args));
    let result = finish(child, input);
    assert!(!result.failed, "{}", result.msg);
    let facts = result.values["ansible_facts"].as_mapping().unwrap();
    assert_eq!(
        facts["ansible_architecture"].as_str(),
        Some(std::env::consts::ARCH)
    );
    assert!(facts["ansible_os_family"].is_string());
    assert!(facts["ansible_interfaces"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some("lo")));
    assert!(facts["ansible_all_ipv4_addresses"].is_sequence());
    assert!(facts["ansible_all_ipv6_addresses"].is_sequence());
    assert!(!facts.contains_key(serde_yaml::Value::String("ansible_date_time".into())));
    let (child, input) = start(request(
        "setup",
        serde_json::json!({"gather_subset":["!all","!min"]}),
    ));
    assert!(finish(child, input).values["ansible_facts"]
        .as_mapping()
        .unwrap()
        .is_empty());
    let (child, input) = start(request(
        "setup",
        serde_json::json!({"gather_subset":["python"]}),
    ));
    assert!(finish(child, input).failed);
}
#[test]
fn stat_checksums_and_link_handling_use_rust() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    fs::write(&path, b"hello").unwrap();
    for (algorithm, checksum) in [
        ("sha1", "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"),
        (
            "sha256",
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        ),
    ] {
        let (child, input) = start(request(
            "stat",
            serde_json::json!({"path":path,"checksum_algorithm":algorithm}),
        ));
        let result = finish(child, input);
        assert_eq!(result.values["stat"]["checksum"].as_str(), Some(checksum));
    }
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let (child, input) = start(request("stat", serde_json::json!({"path":link})));
    let result = finish(child, input);
    assert_eq!(result.values["stat"]["islnk"].as_bool(), Some(true));
    assert!(result.values["stat"].get("checksum").is_none());
    let (child, input) = start(request(
        "stat",
        serde_json::json!({"path":directory.path().join("absent")}),
    ));
    assert_eq!(
        finish(child, input).values["stat"]["exists"].as_bool(),
        Some(false)
    );
}
#[test]
fn commands_preserve_argv_and_check_never_executes() {
    let (child, input) = start(request(
        "command",
        serde_json::json!({"cmd":"printf '%s' 'literal;echo nope'"}),
    ));
    assert_eq!(finish(child, input).stdout, "literal;echo nope");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("marker");
    let mut preview = request(
        "shell",
        serde_json::json!({"cmd":format!("touch {}",quote(&path))}),
    );
    preview.check = true;
    let (child, input) = start(preview);
    assert_eq!(finish(child, input).outcome(), Outcome::Skipped);
    assert!(!path.exists());
}
#[test]
fn eof_and_signals_kill_and_reap_setsid_descendants_and_hide_program_argv() {
    for signal in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let leader = directory.path().join("leader");
        let escaped = directory.path().join("escaped");
        let body = format!(
            "echo $$ > {}; setsid sh -c 'echo $$ > {}; sleep 60' & wait # private-program-canary",
            quote(&leader),
            escaped.display()
        );
        let (child, input) = start(request("shell", serde_json::json!({"cmd":body})));
        wait_file(&leader);
        wait_file(&escaped);
        let leader = fs::read_to_string(leader)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        let escaped = fs::read_to_string(escaped)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        for pid in [child.id() as i32, leader] {
            let argv = fs::read(format!("/proc/{pid}/cmdline")).unwrap();
            assert!(!String::from_utf8_lossy(&argv).contains("private-program-canary"));
        }
        if signal {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGTERM);
            }
        }
        drop(input);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = decode::<Response>(&output.stdout, MAX_RESPONSE_BYTES)
            .unwrap()
            .validate()
            .unwrap();
        assert_eq!(result.outcome(), Outcome::Uncertain);
        for pid in [leader, escaped] {
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists(),
                "descendant {pid} survived cancellation"
            );
        }
    }
}
#[test]
fn timeout_and_invalid_protocol_never_report_success() {
    let mut timed = request("shell", serde_json::json!({"cmd":"sleep 60"}));
    timed.timeout_ms = 100;
    let (child, input) = start(timed);
    assert_eq!(finish(child, input).outcome(), Outcome::Uncertain);
    let mut invalid = request("setup", serde_json::json!({}));
    invalid.build_id = "wrong-build".into();
    let (child, input) = start(invalid);
    assert_eq!(finish(child, input).outcome(), Outcome::Failed);
}

#[test]
fn managed_directory_is_removed_by_worker_even_without_a_bootstrap_exit_trap() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempfile::tempdir().unwrap();
    let base = parent.path().join("base");
    fs::create_dir(&base).unwrap();
    let directory = base.join(".rustsible-worker-0123456789");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let binary = directory.join("worker");
    // A writer in this multithreaded harness can be inherited by another
    // concurrent spawn until that child execs, making our exec fail ETXTBSY.
    // Copy in a separate process and wait until all its write handles close.
    assert!(Command::new("cp")
        .arg(env!("CARGO_BIN_EXE_rustsible-worker"))
        .arg(&binary)
        .status()
        .unwrap()
        .success());
    let marker = parent.path().join("started");
    let (child, input) = start_at(
        request(
            "shell",
            serde_json::json!({"cmd":format!("printf x > {}; sleep 60", quote(&marker))}),
        ),
        &binary,
    );
    wait_file(&marker);
    let moved = parent.path().join("moved-base");
    fs::rename(&base, &moved).unwrap();
    fs::create_dir_all(&directory).unwrap();
    let unrelated = directory.join("unrelated");
    fs::write(&unrelated, b"keep replacement data").unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        decode::<Response>(&output.stdout, MAX_RESPONSE_BYTES)
            .unwrap()
            .result
            .outcome(),
        Outcome::Uncertain
    );
    assert!(
        !moved.join(".rustsible-worker-0123456789").exists(),
        "worker relied on the missing bootstrap shell to clean up"
    );
    assert_eq!(
        fs::read(unrelated).unwrap(),
        b"keep replacement data",
        "cleanup followed the stale ancestor pathname"
    );
}
