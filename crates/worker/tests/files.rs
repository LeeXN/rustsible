use rustsible_protocol::{
    Hello, ModuleResult, Payload, PayloadReady, Request, Response, MAX_REQUEST_BYTES,
    MAX_RESPONSE_BYTES,
};
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    time::Duration,
};
fn request(module: &str, args: serde_json::Value, check: bool) -> Request {
    let mut request = Request::new(module, serde_yaml::to_value(args).unwrap(), check, 30000);
    request.managed = false;
    request
}
fn start(request: Request, payload: Option<&[u8]>, mask: Option<u32>) -> (Child, ChildStdin) {
    start_env(request, payload, mask, &[])
}
fn start_env(
    mut request: Request,
    payload: Option<&[u8]>,
    mask: Option<u32>,
    envs: &[(&str, String)],
) -> (Child, ChildStdin) {
    if let Some(payload) = payload.filter(|_| request.payload.is_none()) {
        request.payload = Some(Payload {
            size: payload.len() as u64,
            sha256: rustsible_protocol::sha256(&mut &payload[..]).unwrap(),
        });
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustsible-worker"));
    command
        .arg("--serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost");
    if let Some(mask) = mask {
        unsafe {
            command.pre_exec(move || {
                libc::umask(mask);
                Ok(())
            });
        }
    }
    command.envs(envs.iter().map(|(key, value)| (*key, value)));
    let mut child = command.spawn().unwrap();
    let _: Hello =
        rustsible_protocol::read_frame(child.stdout.as_mut().unwrap(), MAX_RESPONSE_BYTES).unwrap();
    let mut input = child.stdin.take().unwrap();
    rustsible_protocol::write_frame(&mut input, &request, MAX_REQUEST_BYTES).unwrap();
    if let Some(payload) = payload {
        let ready: PayloadReady =
            rustsible_protocol::read_frame(child.stdout.as_mut().unwrap(), MAX_RESPONSE_BYTES)
                .unwrap();
        ready.validate().unwrap();
        input.write_all(payload).unwrap();
        input.flush().unwrap();
    }
    (child, input)
}
fn finish(mut child: Child, input: ChildStdin) -> ModuleResult {
    let result: Response =
        rustsible_protocol::read_frame(child.stdout.as_mut().unwrap(), MAX_RESPONSE_BYTES).unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    result.validate().unwrap()
}
fn run(module: &str, args: serde_json::Value, check: bool) -> ModuleResult {
    let (child, input) = start(request(module, args, check), None, None);
    finish(child, input)
}
fn http(bytes: &'static [u8], length: Option<usize>) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/artifact", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(stream.read(&mut request).unwrap() > 0);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            length.unwrap_or(bytes.len())
        )
        .unwrap();
        let _ = stream.write_all(bytes);
    });
    (url, server)
}
fn xattr(path: &Path, value: Option<&[u8]>) -> Vec<u8> {
    use std::os::fd::AsRawFd;
    let file = fs::File::open(path).unwrap();
    let name = c"user.rustsible-protection";
    if let Some(value) = value {
        assert_eq!(
            unsafe {
                libc::fsetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                )
            },
            0
        );
    }
    let mut buffer = vec![0; 1024];
    let length = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    assert!(length >= 0);
    buffer.truncate(length as usize);
    buffer
}
#[test]
fn download_retains_force_false_and_metadata_and_updates_for_checksum() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("artifact");
    fs::write(&path, b"original").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o6755)).unwrap();
    xattr(&path, Some(b"keep protection"));
    let original = fs::metadata(&path).unwrap();
    for check in [true, false] {
        let result = run(
            "get_url",
            json!({"url":"http://127.0.0.1:1/must-not-connect","dest":path,"force":false}),
            check,
        );
        assert!(!result.failed && !result.changed, "{}", result.msg);
    }
    let (url, server) = http(b"hello", None);
    let args = json!({"url":url,"dest":path,"force":false,"checksum":"sha1:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"});
    let result = run("get_url", args.clone(), false);
    assert!(!result.failed && result.changed, "{}", result.msg);
    server.join().unwrap();
    assert_eq!(
        result.values["checksum_src"].as_str(),
        Some("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d")
    );
    assert_eq!(fs::read(&path).unwrap(), b"hello");
    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        (after.mode(), after.uid(), after.gid()),
        (original.mode(), original.uid(), original.gid())
    );
    assert_eq!(xattr(&path, None), b"keep protection");
    assert!(!run("get_url", args, false).changed);
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
}
#[test]
fn failed_downloads_and_invalid_arguments_leave_original_inode_and_protection_intact() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("artifact");
    fs::write(&path, b"original").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    xattr(&path, Some(b"protection"));
    let original = fs::metadata(&path).unwrap();
    for (bytes, length, checksum) in [
        (b"corrupt".as_slice(), None, true),
        (b"short".as_slice(), Some(999), false),
    ] {
        let (url, server) = http(bytes, length);
        let mut args = json!({"url":url,"dest":path,"force":true});
        if checksum {
            args["checksum"] = "sha1:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d".into();
        }
        let result = run("get_url", args, false);
        assert!(result.failed && !result.changed, "{}", result.msg);
        server.join().unwrap();
    }
    for (key, value) in [
        ("mode", "888"),
        ("owner", "4294967295"),
        ("owner", "missing-rustsible-account-99999"),
        ("checksum", "sha1:not-a-digest"),
        ("dest", "/tmp/"),
    ] {
        let mut args =
            json!({"url":"http://127.0.0.1:1/must-not-connect","dest":path,"force":true});
        args[key] = value.into();
        for check in [false, true] {
            assert!(run("get_url", args.clone(), check).failed);
        }
    }
    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        (after.ino(), after.mode(), after.uid(), after.gid()),
        (
            original.ino(),
            original.mode(),
            original.uid(),
            original.gid()
        )
    );
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(xattr(&path, None), b"protection");
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
}
#[test]
fn check_uses_head_and_a_new_download_keeps_target_umask() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("artifact");
    let (url, server) = http(b"hello", None);
    let result = run("get_url", json!({"url":url,"dest":path}), true);
    assert!(!result.failed && result.changed, "{}", result.msg);
    server.join().unwrap();
    assert!(!path.exists());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
    let (url, server) = http(b"hello", None);
    let (child, input) = start(
        request("get_url", json!({"url":url,"dest":path}), false),
        None,
        Some(0o077),
    );
    let result = finish(child, input);
    assert!(!result.failed, "{}", result.msg);
    server.join().unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
}
#[test]
fn streamed_binary_over_16_mib_is_atomic_and_check_and_force_false_preserve_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("binary");
    fs::write(&path, b"original").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    xattr(&path, Some(b"binary-protection"));
    let mut payload = vec![0u8; 20 * 1024 * 1024 + 7];
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte = (index % 256) as u8;
    }
    let args = json!({"dest":path});
    let (child, input) = start(request("copy", args.clone(), true), Some(&payload), None);
    let result = finish(child, input);
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert_eq!(fs::read(&path).unwrap(), b"original");
    let (child, input) = start(request("copy", args.clone(), false), Some(&payload), None);
    let result = finish(child, input);
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert_eq!(fs::read(&path).unwrap(), payload);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o755);
    assert_eq!(xattr(&path, None), b"binary-protection");
    let (child, input) = start(
        request("copy", json!({"dest":path,"force":false}), false),
        Some(b"must not overwrite"),
        None,
    );
    let result = finish(child, input);
    assert!(!result.failed && !result.changed, "{}", result.msg);
    assert_eq!(fs::read(&path).unwrap(), payload);
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
}
#[test]
fn cancelling_a_download_cleans_destination_staging_after_killing_the_module() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("artifact");
    fs::write(&path, b"original").unwrap();
    let original = fs::metadata(&path).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/slow", listener.local_addr().unwrap());
    let (started, ready) = std::sync::mpsc::channel();
    let (finish_server, stop) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        assert!(stream.read(&mut request).unwrap() > 0);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9999999\r\n\r\npartial")
            .unwrap();
        started.send(()).unwrap();
        stop.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    let (child, input) = start(
        request(
            "get_url",
            json!({"url":url,"dest":path,"force":true}),
            false,
        ),
        None,
        None,
    );
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(fs::read_dir(temporary.path()).unwrap().count() > 1);
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Response =
        rustsible_protocol::decode(&output.stdout, MAX_RESPONSE_BYTES).unwrap();
    assert!(response.result.uncertain);
    finish_server.send(()).unwrap();
    server.join().unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::metadata(&path).unwrap().ino(), original.ino());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
}
#[test]
fn files_directories_links_and_line_updates_are_native_and_idempotent() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("nested/child/");
    let args = json!({"path":directory,"state":"directory","mode":"0750"});
    let predicted = run("file", args.clone(), true);
    assert!(!predicted.failed && predicted.changed, "{}", predicted.msg);
    assert!(!directory.exists());
    let first = run("file", args.clone(), false);
    assert!(!first.failed && first.changed, "{}", first.msg);
    let second = run("file", args, false);
    assert!(!second.failed && !second.changed, "{}", second.msg);
    let path = directory.join("lines");
    let args = json!({"path":path,"line":"entry=one","create":true,"mode":"0755"});
    assert!(run("lineinfile", args.clone(), false).changed);
    assert!(!run("lineinfile", args, false).changed);
    fs::write(&path, b"entry=one\nentry=two\n").unwrap();
    let args = json!({"path":path,"regexp":"^entry=","line":"entry=last","backup":true});
    let result = run("lineinfile", args.clone(), false);
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert_eq!(fs::read(&path).unwrap(), b"entry=one\nentry=last\n");
    assert_eq!(
        fs::read(result.values["backup_file"].as_str().unwrap()).unwrap(),
        b"entry=one\nentry=two\n"
    );
    assert!(!run("lineinfile", args, false).changed);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o755);
    let link = directory.join("link");
    let args = json!({"path":link,"state":"link","src":path});
    assert!(run("file", args.clone(), false).changed);
    assert!(!run("file", args, false).changed);
    assert_eq!(fs::read_link(&link).unwrap(), path);
    assert!(run("file", json!({"path":directory,"state":"absent"}), true).changed);
    assert!(directory.exists());
    assert!(run("file", json!({"path":directory,"state":"absent"}), false).changed);
    assert!(!directory.exists());
    assert!(
        !run(
            "file",
            json!({"path":directory.join("already/absent"),"state":"absent"}),
            false
        )
        .changed
    );
}

#[path = "support/http.rs"]
mod http_fixture;
struct HttpFixture {
    url: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
}
impl HttpFixture {
    fn start(tls: bool) -> Self {
        use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tls");
        let config = if tls {
            let certificates = CertificateDer::pem_file_iter(root.join("server.pem"))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let key = PrivateKeyDer::from_pem_file(root.join("server-key.pem")).unwrap();
            Some(std::sync::Arc::new(
                rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(certificates, key)
                    .unwrap(),
            ))
        } else {
            None
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!(
            "{}://{}",
            if tls { "https" } else { "http" },
            listener.local_addr().unwrap()
        );
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished = stop.clone();
        let server = std::thread::spawn(move || {
            while !finished.load(std::sync::atomic::Ordering::Relaxed) {
                match listener.accept() {
                    Ok((connection, _)) => {
                        let root = root.clone();
                        let config = config.clone();
                        std::thread::spawn(move || {
                            let _ = http_fixture::handle(connection, &root, config);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("HTTP fixture accept: {error}"),
                }
            }
        });
        Self {
            url,
            stop,
            server: Some(server),
        }
    }
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.server.take().unwrap().join().unwrap();
    }
}

#[test]
fn uri_methods_bodies_status_redirects_and_auth_are_structured_and_private() {
    let server = HttpFixture::start(false);
    let post = run(
        "uri",
        json!({"url":format!("{}/echo",server.url),"method":"POST","body_format":"json","body":{"message":"hello"},"headers":{"X-Test":"yes"},"return_content":true}),
        false,
    );
    assert!(!post.failed && !post.changed, "{}", post.msg);
    assert_eq!(post.values["status"].as_u64(), Some(200));
    assert_eq!(post.values["json"]["method"].as_str(), Some("POST"));
    assert_eq!(
        post.values["json"]["headers"]["x-test"].as_str(),
        Some("yes")
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(post.values["json"]["body"].as_str().unwrap())
            .unwrap(),
        json!({"message":"hello"})
    );
    let form = run(
        "uri",
        json!({"url":format!("{}/echo",server.url),"method":"PUT","body_format":"form-urlencoded","body":{"space":"a b"}}),
        false,
    );
    assert!(!form.failed, "{}", form.msg);
    assert_eq!(form.values["json"]["body"].as_str(), Some("space=a+b"));
    assert!(!form.values.contains_key("content"));
    let rejected = run(
        "uri",
        json!({"url":format!("{}/status",server.url),"status_code":[200,201]}),
        false,
    );
    assert!(rejected.failed);
    assert_eq!(rejected.values["status"].as_u64(), Some(503));
    assert!(
        !run(
            "uri",
            json!({"url":format!("{}/status",server.url),"status_code":503}),
            false
        )
        .failed
    );
    let redirected = run(
        "uri",
        json!({"url":format!("{}/redirect",server.url),"return_content":true}),
        false,
    );
    assert!(!redirected.failed, "{}", redirected.msg);
    assert_eq!(redirected.values["content"].as_str(), Some("hello"));
    let untouched = run(
        "uri",
        json!({"url":format!("{}/redirect",server.url),"method":"POST","status_code":302}),
        false,
    );
    assert!(!untouched.failed, "{}", untouched.msg);
    for force in [false, true] {
        let auth = run(
            "uri",
            json!({"url":format!("{}/echo",server.url),"url_username":"user","url_password":"pass","force_basic_auth":force}),
            false,
        );
        assert!(!auth.failed && auth.no_log, "{}", auth.msg);
        assert_eq!(
            auth.values["json"]["headers"]
                .get("authorization")
                .and_then(serde_yaml::Value::as_str),
            force.then_some("Basic dXNlcjpwYXNz")
        );
        assert!(!serde_json::to_string(&auth.displayed())
            .unwrap()
            .contains("dXNlcjpwYXNz"));
    }
    let challenge = run(
        "uri",
        json!({"url":format!("{}/auth",server.url),"url_username":"user","url_password":"pass"}),
        false,
    );
    assert!(!challenge.failed && challenge.no_log, "{}", challenge.msg);
}

#[test]
fn uri_reuses_target_ca_proxy_and_bounded_response_policy() {
    let tls = HttpFixture::start(true);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/tls/ca.pem");
    let rejected = run(
        "uri",
        json!({"url":format!("{}/hello?URI_PRIVATE_SENTINEL",tls.url)}),
        false,
    );
    assert!(rejected.failed && !rejected.msg.contains("URI_PRIVATE_SENTINEL"));
    let accepted = run(
        "uri",
        json!({"url":format!("{}/hello",tls.url),"ca_path":root,"return_content":true}),
        false,
    );
    assert!(!accepted.failed, "{}", accepted.msg);
    assert_eq!(accepted.values["content"].as_str(), Some("hello"));
    let server = HttpFixture::start(false);
    let proxy = [
        ("HTTP_PROXY", server.url.clone()),
        ("http_proxy", server.url.clone()),
        ("NO_PROXY", String::new()),
        ("no_proxy", String::new()),
    ];
    let (child, input) = start_env(
        request(
            "uri",
            json!({"url":"http://must-not-resolve.invalid/hello","return_content":true}),
            false,
        ),
        None,
        None,
        &proxy,
    );
    let proxied = finish(child, input);
    assert!(!proxied.failed, "{}", proxied.msg);
    assert_eq!(proxied.values["content"].as_str(), Some("hello"));
    for path in ["oversized", "redirect-loop", "truncated"] {
        assert!(
            run(
                "uri",
                json!({"url":format!("{}/{path}",server.url),"return_content":true}),
                false
            )
            .failed,
            "{path}"
        );
    }
    assert!(run("uri", json!({"url":format!("{}/echo",server.url),"method":"POST","body_format":"raw","body":{"invalid":"object"}}), false).failed);
    assert!(run("uri", json!({"url":format!("{}/echo",server.url),"headers":{"Injected":"bad\r\nHeader: true"}}), false).failed);
    assert!(run("uri", json!({"url":"http://must-not-resolve.invalid/","method":"POST","body":"must-not-send"}), true).skipped);
}

#[test]
fn wait_for_observes_delayed_content_absence_and_tcp_readiness_with_bounds() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("ready");
    let changing = path.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        fs::write(changing, "READY\n").unwrap();
    });
    let ready = run(
        "wait_for",
        json!({"path":path,"search_regex":"^READY$","timeout":3,"sleep":0}),
        false,
    );
    assert!(!ready.failed && !ready.changed, "{}", ready.msg);
    writer.join().unwrap();
    assert!(
        run(
            "wait_for",
            json!({"path":path,"search_regex":"MISSING","state":"absent","timeout":1}),
            false
        )
        .failed
    );
    assert!(run("wait_for", json!({"path":path,"timeout":0}), false).failed);
    assert!(
        run(
            "wait_for",
            json!({"path":path,"delay":1,"timeout":1}),
            false
        )
        .failed
    );
    fs::remove_file(&path).unwrap();
    assert!(
        !run(
            "wait_for",
            json!({"path":path,"state":"absent","timeout":1}),
            false
        )
        .failed
    );
    assert!(run("wait_for", json!({"path":path,"timeout":0}), false).failed);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let delayed = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let _ = listener.accept().unwrap();
    });
    let tcp = run(
        "wait_for",
        json!({"port":port,"timeout":3,"sleep":0}),
        false,
    );
    assert!(!tcp.failed, "{}", tcp.msg);
    delayed.join().unwrap();
    assert!(
        !run(
            "wait_for",
            json!({"port":port,"state":"stopped","timeout":1}),
            false
        )
        .failed
    );
    assert!(run("wait_for", json!({"port":port,"path":path}), false).failed);
    fs::write(&path, vec![b'x'; 8 * 1024 * 1024 + 1]).unwrap();
    let bounded = run(
        "wait_for",
        json!({"path":path,"search_regex":"x","timeout":1}),
        false,
    );
    assert!(bounded.failed && bounded.msg.contains("8 MiB"));
    assert!(
        run(
            "wait_for",
            json!({"path":path,"search_regex":"[","timeout":0}),
            false
        )
        .failed
    );
    assert!(run("wait_for", json!({"port":port,"timeout":30}), true).skipped);
}
#[test]
fn https_uses_target_ca_and_http_honors_proxy_and_redirect_limits_without_leaking_credentials() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("artifact");
    fs::write(&path, b"original").unwrap();
    let tls = HttpFixture::start(true);
    let args =
        json!({"url":format!("{}/hello?private-token-canary",tls.url),"dest":path,"force":true});
    let result = run("get_url", args.clone(), false);
    assert!(result.failed && !result.changed);
    assert!(!result.msg.contains("private-token-canary"));
    assert_eq!(fs::read(&path).unwrap(), b"original");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/tls/ca.pem");
    let empty = temporary.path().join("empty-certs");
    fs::create_dir(&empty).unwrap();
    let env = [
        ("SSL_CERT_FILE", root.to_str().unwrap().into()),
        ("SSL_CERT_DIR", empty.to_str().unwrap().into()),
    ];
    let (child, input) = start_env(request("get_url", args, false), None, None, &env);
    let result = finish(child, input);
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert_eq!(fs::read(&path).unwrap(), b"hello");
    let server = HttpFixture::start(false);
    for endpoint in ["redirect-loop", "status"] {
        let result = run(
            "get_url",
            json!({"url":format!("{}/{endpoint}?private-token-canary",server.url),"dest":path,"force":true}),
            false,
        );
        assert!(result.failed && !result.changed, "{}", result.msg);
        assert!(!result.msg.contains("private-token-canary"));
        assert_eq!(fs::read(&path).unwrap(), b"hello");
    }
    let result = run(
        "get_url",
        json!({"url":format!("{}/redirect",server.url),"dest":path,"force":true}),
        false,
    );
    assert!(!result.failed && !result.changed, "{}", result.msg);
    let proxy = [
        ("HTTP_PROXY", server.url.clone()),
        ("http_proxy", server.url.clone()),
        ("NO_PROXY", String::new()),
        ("no_proxy", String::new()),
    ];
    let (child, input) = start_env(
        request(
            "get_url",
            json!({"url":"http://must-not-resolve.invalid/hello","dest":path,"force":true}),
            false,
        ),
        None,
        None,
        &proxy,
    );
    let result = finish(child, input);
    assert!(!result.failed && !result.changed, "{}", result.msg);
    assert_eq!(fs::read(&path).unwrap(), b"hello");
}
#[test]
fn ancestor_rename_during_download_keeps_operations_and_cleanup_on_the_opened_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let parent = temporary.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("artifact");
    fs::write(&path, b"original").unwrap();
    let moved = temporary.path().join("moved");
    let original_path = path.clone();
    let old = parent.clone();
    let renamed = moved.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/artifact", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut bytes = [0; 4096];
        assert!(stream.read(&mut bytes).unwrap() > 0);
        fs::rename(old, &renamed).unwrap();
        fs::create_dir_all(original_path.parent().unwrap()).unwrap();
        fs::write(original_path, b"keep replacement").unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
            .unwrap();
    });
    let result = run(
        "get_url",
        json!({"url":url,"dest":path,"force":true}),
        false,
    );
    assert!(!result.failed && result.changed, "{}", result.msg);
    server.join().unwrap();
    assert_eq!(fs::read(moved.join("artifact")).unwrap(), b"hello");
    assert_eq!(fs::read(&path).unwrap(), b"keep replacement");
    assert_eq!(fs::read_dir(moved).unwrap().count(), 1);
}
#[test]
fn metadata_updates_do_not_require_read_permission_and_invalid_settings_do_not_create_parents() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("artifact");
    fs::write(&path, b"original").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let result = run("file", json!({"path":path,"mode":"0600"}), false);
    assert!(!result.failed && result.changed, "{}", result.msg);
    assert_eq!(fs::read(&path).unwrap(), b"original");
    let nested = temporary.path().join("must-not-exist/child");
    for check in [false, true] {
        let result = run(
            "file",
            json!({"path":nested,"state":"directory","mode":"888"}),
            check,
        );
        assert!(result.failed && !result.changed);
        assert!(!nested.parent().unwrap().exists());
    }
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o777)).unwrap();
    let result = run(
        "get_url",
        json!({"url":"http://127.0.0.1:1/must-not-connect","dest":path,"force":true}),
        false,
    );
    assert!(result.failed && result.msg.contains("Unsafe destination directory"));
    assert_eq!(fs::read(&path).unwrap(), b"original");
}

#[test]
fn posix_acl_and_explicit_unchanged_owner_survive_content_replacement() {
    use std::os::fd::AsRawFd;
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("executable");
    fs::write(&path, b"original").unwrap();
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, permission, id) in [
        (1u16, 7u16, u32::MAX),
        (2, 4, 12345),
        (4, 5, u32::MAX),
        (16, 5, u32::MAX),
        (32, 1, u32::MAX),
    ] {
        acl.extend(tag.to_le_bytes());
        acl.extend(permission.to_le_bytes());
        acl.extend(id.to_le_bytes());
    }
    let file = fs::File::open(&path).unwrap();
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                c"system.posix_acl_access".as_ptr(),
                acl.as_ptr().cast(),
                acl.len(),
                0,
            )
        },
        0
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o6751)).unwrap();
    let before = fs::metadata(&path).unwrap();
    let mut original_acl = vec![0u8; 1024];
    let size = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            c"system.posix_acl_access".as_ptr(),
            original_acl.as_mut_ptr().cast(),
            original_acl.len(),
        )
    };
    assert!(size > 0);
    original_acl.truncate(size as usize);
    drop(file);
    let (child, input) = start(
        request(
            "copy",
            json!({"dest":path,"owner":before.uid().to_string(),"group":before.gid().to_string()}),
            false,
        ),
        Some(b"replacement"),
        None,
    );
    let result = finish(child, input);
    assert!(!result.failed && result.changed, "{}", result.msg);
    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        (after.mode(), after.uid(), after.gid()),
        (before.mode(), before.uid(), before.gid())
    );
    let file = fs::File::open(&path).unwrap();
    let mut installed = vec![0u8; 1024];
    let size = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            c"system.posix_acl_access".as_ptr(),
            installed.as_mut_ptr().cast(),
            installed.len(),
        )
    };
    assert!(size > 0);
    installed.truncate(size as usize);
    assert_eq!(installed, original_acl);
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
}
#[test]
fn corrupt_or_truncated_payloads_and_symbolic_destinations_never_modify_business_files() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("original");
    fs::write(&path, b"keep original").unwrap();
    let before = fs::metadata(&path).unwrap();
    let mut bad = request("copy", json!({"dest":path}), false);
    bad.payload = Some(Payload {
        size: 7,
        sha256: "0".repeat(64),
    });
    let (child, input) = start(bad, Some(b"corrupt"), None);
    let result = finish(child, input);
    assert!(result.failed && !result.changed, "{}", result.msg);
    let mut truncated = request("copy", json!({"dest":path}), false);
    truncated.payload = Some(Payload {
        size: 99,
        sha256: "0".repeat(64),
    });
    let (child, input) = start(truncated, Some(b"short"), None);
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Response =
        rustsible_protocol::decode(&output.stdout, MAX_RESPONSE_BYTES).unwrap();
    assert!(response.result.failed && !response.result.changed);
    let link = temporary.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let (child, input) = start(
        request("copy", json!({"dest":link}), false),
        Some(b"must not follow"),
        None,
    );
    let result = finish(child, input);
    assert!(result.failed && !result.changed);
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&path).unwrap(), b"keep original");
    assert_eq!(fs::metadata(&path).unwrap().ino(), before.ino());
    assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 2);
}

fn archive_bytes(entries: &[(&str, &[u8], u32)], compressed: bool) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes, mode) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder.append_data(&mut header, name, *bytes).unwrap();
    }
    let bytes = builder.into_inner().unwrap();
    if !compressed {
        return bytes;
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&bytes).unwrap();
    encoder.finish().unwrap()
}
#[test]
fn unarchive_streams_tar_gzip_zip_preserves_existing_metadata_and_predicts_without_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("dest");
    fs::create_dir(&destination).unwrap();
    for compressed in [false, true] {
        let bytes = archive_bytes(
            &[("nested/tool", b"v1", 0o755), ("plain", b"data", 0o644)],
            compressed,
        );
        let args = json!({"src":"controller-relative.tar", "dest":destination});
        let (child, input) = start(request("unarchive", args.clone(), true), Some(&bytes), None);
        let prediction = finish(child, input);
        assert!(
            !prediction.failed && prediction.changed,
            "{}",
            prediction.msg
        );
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        let (child, input) = start(
            request("unarchive", args.clone(), false),
            Some(&bytes),
            None,
        );
        let applied = finish(child, input);
        assert!(!applied.failed && applied.changed, "{}", applied.msg);
        assert_eq!(fs::read(destination.join("nested/tool")).unwrap(), b"v1");
        assert_eq!(
            fs::metadata(destination.join("nested/tool"))
                .unwrap()
                .mode()
                & 0o777,
            0o755
        );
        let (child, input) = start(request("unarchive", args, false), Some(&bytes), None);
        let noop = finish(child, input);
        assert!(!noop.failed && !noop.changed, "{}", noop.msg);
        fs::remove_dir_all(destination.join("nested")).unwrap();
        fs::remove_file(destination.join("plain")).unwrap();
    }
    let zip_path = temporary.path().join("archive.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
    zip.start_file(
        "nested/tool",
        zip::write::SimpleFileOptions::default().unix_permissions(0o755),
    )
    .unwrap();
    zip.write_all(b"zip tool").unwrap();
    zip.finish().unwrap();
    let args = json!({"src":zip_path,"dest":destination,"remote_src":true});
    let applied = run("unarchive", args.clone(), false);
    assert!(!applied.failed && applied.changed, "{}", applied.msg);
    let path = destination.join("nested/tool");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    let bytes = archive_bytes(&[("nested/tool", b"new tar tool", 0o644)], false);
    let archive = temporary.path().join("update.tar");
    fs::write(&archive, bytes).unwrap();
    let updated = run(
        "unarchive",
        json!({"src":archive,"dest":destination,"remote_src":true}),
        false,
    );
    assert!(!updated.failed && updated.changed, "{}", updated.msg);
    assert_eq!(fs::read(&path).unwrap(), b"new tar tool");
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o750);
    let skipped = run(
        "unarchive",
        json!({"src":temporary.path().join("missing.tar"),"dest":destination,"remote_src":true,"creates":"nested/tool"}),
        false,
    );
    assert!(!skipped.failed && !skipped.changed, "{}", skipped.msg);
}
#[test]
fn unarchive_rejects_traversal_links_corruption_and_all_conflicts_before_modification() {
    let temporary = tempfile::tempdir().unwrap();
    let dest = temporary.path().join("dest");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("kept"), b"original").unwrap();
    fs::set_permissions(dest.join("kept"), fs::Permissions::from_mode(0o755)).unwrap();
    let safe = archive_bytes(
        &[("kept", b"replace", 0o644), ("other", b"new", 0o644)],
        false,
    );
    let mut traversal = safe.clone();
    traversal[0..100].fill(0);
    traversal[..10].copy_from_slice(b"../escaped");
    // Recompute the tar header checksum after deliberately bypassing Builder's
    // own safe-path guard; the module must reject a valid malicious archive.
    traversal[148..156].fill(b' ');
    let checksum: u32 = traversal[..512].iter().map(|byte| *byte as u32).sum();
    traversal[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    let mut corrupt = archive_bytes(&[("kept", b"replace", 0o644)], true);
    let end = corrupt.len();
    corrupt[end - 8] ^= 1;
    let mut truncated = safe.clone();
    truncated.truncate(514);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(
            "kept",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"replace").unwrap();
    let mut bad_crc = writer.finish().unwrap().into_inner();
    let mut reader = zip::ZipArchive::new(std::io::Cursor::new(&bad_crc)).unwrap();
    let offset = reader.by_index(0).unwrap().data_start().unwrap() as usize;
    drop(reader);
    bad_crc[offset] ^= 1;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .add_symlink(
            "escaped",
            "../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    let zip_link = writer.finish().unwrap().into_inner();
    let mut linked = tar::Builder::new(Vec::new());
    for kind in [tar::EntryType::Symlink, tar::EntryType::Link] {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../escaped").unwrap();
        header.set_cksum();
        linked
            .append_data(
                &mut header,
                if kind.is_symlink() { "symbol" } else { "hard" },
                &b""[..],
            )
            .unwrap();
    }
    for bytes in [
        traversal,
        corrupt,
        truncated,
        bad_crc,
        zip_link,
        linked.into_inner().unwrap(),
    ] {
        let (child, input) = start(
            request("unarchive", json!({"src":"archive","dest":dest}), false),
            Some(&bytes),
            None,
        );
        let result = finish(child, input);
        assert!(result.failed && !result.changed, "{}", result.msg);
        assert_eq!(fs::read(dest.join("kept")).unwrap(), b"original");
        assert_eq!(
            fs::metadata(dest.join("kept")).unwrap().mode() & 0o777,
            0o755
        );
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
        assert!(!temporary.path().join("escaped").exists());
    }
    std::os::unix::fs::symlink(temporary.path(), dest.join("nested")).unwrap();
    let bytes = archive_bytes(
        &[
            ("kept", b"replace", 0o644),
            ("nested/escaped", b"danger", 0o644),
        ],
        false,
    );
    let (child, input) = start(
        request("unarchive", json!({"src":"archive","dest":dest}), false),
        Some(&bytes),
        None,
    );
    let result = finish(child, input);
    assert!(result.failed && !result.changed, "{}", result.msg);
    assert_eq!(fs::read(dest.join("kept")).unwrap(), b"original");
    assert!(!temporary.path().join("escaped").exists());
}
#[test]
fn git_checkout_revision_update_dirty_rejection_and_check_are_observable() {
    let temporary = tempfile::tempdir().unwrap();
    let repo = temporary.path().join("source");
    fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-C", repo.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "--initial-branch=main"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "user.name", "Fixture"]);
    fs::write(repo.join("version"), b"first").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "first"]);
    let first = git(&["rev-parse", "HEAD"]);
    git(&["tag", "-a", "v1", "-m", "first release"]);
    let dest = temporary.path().join("checkout");
    let args = json!({"repo":repo,"dest":dest});
    let prediction = run("git", args.clone(), true);
    assert!(
        !prediction.failed && prediction.changed,
        "{}",
        prediction.msg
    );
    assert!(!dest.exists());
    let applied = run("git", args.clone(), false);
    assert!(!applied.failed && applied.changed, "{}", applied.msg);
    assert_eq!(fs::read(dest.join("version")).unwrap(), b"first");
    assert!(!run("git", args.clone(), false).changed);
    fs::write(repo.join("version"), b"second").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "second"]);
    let second = git(&["rev-parse", "HEAD"]);
    let no_update = run(
        "git",
        json!({"repo":repo,"dest":dest,"update":false}),
        false,
    );
    assert!(!no_update.failed && !no_update.changed);
    assert_eq!(fs::read(dest.join("version")).unwrap(), b"first");
    let before_refs = fs::read(dest.join(".git/HEAD")).unwrap();
    let prediction = run("git", args.clone(), true);
    assert!(!prediction.failed && prediction.changed);
    assert_eq!(fs::read(dest.join(".git/HEAD")).unwrap(), before_refs);
    fs::write(dest.join("version"), b"local changes").unwrap();
    let refused = run("git", args.clone(), false);
    assert!(refused.failed && !refused.changed, "{}", refused.msg);
    assert_eq!(fs::read(dest.join("version")).unwrap(), b"local changes");
    fs::write(dest.join("version"), b"first").unwrap();
    let updated = run("git", args.clone(), false);
    assert!(!updated.failed && updated.changed, "{}", updated.msg);
    assert_eq!(updated.values["after"].as_str(), Some(second.as_str()));
    assert_eq!(fs::read(dest.join("version")).unwrap(), b"second");
    for version in [first.as_str(), "v1", "refs/tags/v1"] {
        let pinned = run(
            "git",
            json!({"repo":repo,"dest":dest,"version":version}),
            false,
        );
        assert!(!pinned.failed, "{}", pinned.msg);
        assert_eq!(pinned.values["after"].as_str(), Some(first.as_str()));
        assert_eq!(fs::read(dest.join("version")).unwrap(), b"first");
    }
    assert!(
        run(
            "git",
            json!({"repo":"ext::touch /tmp/unsafe","dest":dest}),
            false
        )
        .failed
    );
    assert!(
        run(
            "git",
            json!({"repo":"https://user:secret@example.invalid/repo","dest":dest}),
            false
        )
        .failed
    );
}
