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
