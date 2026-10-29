//! One private worker lease per effective host/identity during a controller run.
//! Closing stdin cancels synchronous work and lets the bootstrap remove itself.
pub(crate) mod transport;
use crate::ssh::{
    config::ConnectionConfig,
    connection::{command_timeout, shell_quote, Connection},
};
use anyhow::{anyhow, bail, Context, Result};
use rustsible_protocol::{Hello, Manifest, ModuleResult, Request, Response, MAX_REQUEST_BYTES};
use serde_yaml::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs::{File, OpenOptions},
    io::{Cursor, Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) enum Source {
    File(File),
    Bytes(Cursor<Vec<u8>>),
}
impl Read for Source {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::File(file) => file.read(bytes),
            Self::Bytes(bytes_source) => bytes_source.read(bytes),
        }
    }
}
impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::File(file) => file.seek(position),
            Self::Bytes(bytes) => bytes.seek(position),
        }
    }
}
impl Source {
    pub fn bytes(bytes: Vec<u8>) -> Self {
        Self::Bytes(Cursor::new(bytes))
    }
    pub fn file(path: &str) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .context("Cannot open local source")?;
        if !file.metadata()?.is_file() {
            bail!("Local source must be a regular file");
        }
        Ok(Self::File(file))
    }
}

#[derive(Default)]
pub(crate) struct Runtime {
    directory: Option<PathBuf>,
    credentials: crate::ssh::credentials::Credentials,
    diff: bool,
    leases: Mutex<HashMap<String, Arc<Mutex<Option<Client>>>>>,
}
impl Runtime {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            ..Self::default()
        }
    }
    pub fn with_diff(mut self, diff: bool) -> Self {
        self.diff = diff;
        self
    }
    pub fn with_credentials(mut self, credentials: crate::ssh::credentials::Credentials) -> Self {
        self.credentials = credentials;
        self
    }
    fn directory(&self) -> Result<PathBuf> {
        if let Some(directory) = &self.directory {
            return Ok(directory.clone());
        }
        #[cfg(test)]
        {
            Ok(crate::testing::worker_bundle())
        }
        #[cfg(not(test))]
        Ok(std::env::current_exe()?
            .parent()
            .context("Controller executable has no parent directory")?
            .join("workers"))
    }
    #[cfg(test)]
    pub fn execute(
        &self,
        config: &ConnectionConfig,
        module: &str,
        args: Value,
        check: bool,
    ) -> Result<ModuleResult> {
        let request = Request::new(
            module,
            args,
            check,
            command_timeout(config)?.as_millis().try_into()?,
        );
        self.execute_request(config, request, None)
    }
    pub(crate) fn execute_request(
        &self,
        config: &ConnectionConfig,
        mut request: Request,
        mut source: Option<&mut Source>,
    ) -> Result<ModuleResult> {
        let mut private_config = config.clone();
        self.credentials.apply(&mut private_config);
        let config = &private_config;
        request.diff |= self.diff
            && rustsible_protocol::registry::Builtin::lookup(&request.module)?.supports_diff();
        let module = request.module.clone();
        let check = request.check;
        let may_modify = request.asynchronous.is_some()
            || rustsible_protocol::registry::Builtin::lookup(&module)?
                .may_modify(&request.args, check);
        if let Some(source) = source.as_mut() {
            let size = source.seek(SeekFrom::End(0))?;
            source.rewind()?;
            request.payload = Some(rustsible_protocol::Payload {
                size,
                sha256: rustsible_protocol::sha256(source)?,
            });
            source.rewind()?;
        }
        request.validate()?;
        // Fingerprint only effective transport settings, never fact overlays.
        // A host keeps one live connection: changing settings shuts down the
        // previous lease, even when returning later to a former configuration.
        let key = config.worker_key()?;
        let lease = self
            .leases
            .lock()
            .map_err(|_| anyhow!("Worker cache lock poisoned"))?
            .entry(config.name.clone())
            .or_default()
            .clone();
        let mut slot = lease
            .lock()
            .map_err(|_| anyhow!("Worker lease lock poisoned"))?;
        if slot.as_ref().is_some_and(|client| client.config_key != key) {
            if let Some(mut client) = slot.take() {
                client
                    .close()
                    .context("Cannot close previous connection after configuration change")?;
            }
        }
        if slot.is_none() {
            *slot = Some(Client::start(config, &self.directory()?, &module, key)?);
        }
        let client = slot.as_mut().context("Missing worker lease")?;
        client.hello.validate(&client.arch, client.uid, &module)?;
        // Commands are normalized into one native parameter form before wire
        // encoding, so the worker never repeats controller CLI parsing.
        if let Value::String(command) = request.args {
            request.args = serde_yaml::to_value(BTreeMap::from([("cmd", command)]))?;
        }
        let frame = rustsible_protocol::encode(&request, MAX_REQUEST_BYTES)?;
        let deadline =
            Instant::now() + Duration::from_millis(request.timeout_ms) + Duration::from_secs(5);
        let mut diagnostic = Vec::new();
        let remote = matches!(&client.session, transport::Session::Remote { .. });
        let mut eligible = source.is_none()
            || request
                .payload
                .as_ref()
                .is_some_and(|payload| payload.size == 0);
        let result = (|| {
            client
                .session
                .send(&frame, deadline, &mut diagnostic)
                .map_err(|error| {
                    crate::ssh::connection::transport_error(error, remote, eligible)
                })?;
            if let Some(source) = source.as_mut() {
                let ack: rustsible_protocol::PayloadAck = client
                    .session
                    .receive(deadline, &mut diagnostic)
                    .map_err(|error| {
                        crate::ssh::connection::transport_error(error, remote, eligible)
                    })?;
                let ready = match ack {
                    rustsible_protocol::PayloadAck::Ready(ready) => ready,
                    rustsible_protocol::PayloadAck::Result(response) => return response.validate(),
                };
                ready.validate().map_err(|error| {
                    crate::ssh::connection::transport_error(error, remote, eligible)
                })?;
                eligible = request
                    .payload
                    .as_ref()
                    .is_some_and(|payload| payload.size == 0);
                let transfer_deadline = Instant::now()
                    + Duration::from_millis(request.timeout_ms).max(Duration::from_secs(30));
                let mut remaining = request
                    .payload
                    .as_ref()
                    .context("Missing payload metadata")?
                    .size;
                let mut buffer = [0; 65536];
                while remaining != 0 {
                    let limit = remaining.min(buffer.len() as u64) as usize;
                    let read = source.read(&mut buffer[..limit])?;
                    if read == 0 {
                        bail!("Local source ended before its declared size");
                    }
                    eligible |= read as u64 == remaining;
                    client
                        .session
                        .send_payload(&buffer[..read], transfer_deadline, &mut diagnostic)
                        .map_err(|error| {
                            crate::ssh::connection::transport_error(error, remote, eligible)
                        })?;
                    remaining -= read as u64;
                }
            }
            let response: Response = client
                .session
                .receive(
                    Instant::now()
                        + Duration::from_millis(request.timeout_ms)
                        + Duration::from_secs(5),
                    &mut diagnostic,
                )
                .map_err(|error| {
                    crate::ssh::connection::transport_error(error, remote, eligible)
                })?;
            response
                .validate()
                .map_err(|error| crate::ssh::connection::transport_error(error, remote, eligible))
        })();
        match result {
            Ok(result) => {
                if result.failed {
                    slot.take();
                }
                Ok(result)
            }
            Err(error) => {
                if request.payload.is_some()
                    && error
                        .downcast_ref::<crate::ssh::connection::TransportFailure>()
                        .is_some()
                {
                    // A target-side input write error may close stdin while a
                    // structured failure is already buffered on stdout.
                    if let Ok(response) = client.session.receive::<Response>(
                        Instant::now() + Duration::from_secs(1),
                        &mut diagnostic,
                    ) {
                        if let Ok(result) = response.validate() {
                            slot.take();
                            return Ok(result);
                        }
                    }
                }
                // Any partial request may already have reached the target.
                // Discard this lease; never replay a modifying invocation.
                slot.take();
                Ok(crate::modules::result::from_error(
                    error.context("Native worker exchange failed; invocation was not replayed"),
                    may_modify,
                ))
            }
        }
    }

    /// Explicit reset shares the same lock as requests, so shutdown cannot
    /// interrupt a task already executing on this host or replay its request.
    pub(crate) fn reset(&self, host: &str) -> Result<()> {
        let lease = self
            .leases
            .lock()
            .map_err(|_| anyhow!("Worker cache lock poisoned"))?
            .get(host)
            .cloned();
        if let Some(lease) = lease {
            let client = lease
                .lock()
                .map_err(|_| anyhow!("Worker lease lock poisoned"))?
                .take();
            if let Some(mut client) = client {
                client.close().context("Cannot reset worker connection")?;
            }
        }
        Ok(())
    }
}
struct Client {
    closed: bool,
    config_key: String,
    session: transport::Session,
    hello: Hello,
    arch: String,
    uid: u32,
}
impl Client {
    fn close(&mut self) -> Result<()> {
        self.closed = true;
        self.session.shutdown()
    }
    fn start(
        config: &ConnectionConfig,
        directory: &Path,
        module: &str,
        config_key: String,
    ) -> Result<Self> {
        let manifest_file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(directory.join("manifest.json")).context(
            "Cannot open worker manifest; specify --worker-dir with the release workers directory",
        )?;
        if !manifest_file.metadata()?.is_file() {
            bail!("Worker manifest must be a regular file");
        }
        let mut manifest_bytes = Vec::new();
        manifest_file
            .take(1024 * 1024 + 1)
            .read_to_end(&mut manifest_bytes)?;
        if manifest_bytes.len() > 1024 * 1024 {
            bail!("Worker manifest is too large");
        }
        let manifest: Manifest =
            serde_json::from_slice(&manifest_bytes).context("Invalid worker manifest")?;
        manifest.validate()?;
        let base = config
            .get_variable("rustsible_remote_tmp")
            .map(|value| value.into_owned())
            .unwrap_or_else(|| "/tmp".into());
        if !Path::new(&base).is_absolute() || base.chars().any(char::is_control) {
            bail!("rustsible_remote_tmp must be an absolute path without control characters");
        }
        let connection = Connection::from_config(config)
            .map_err(|error| crate::modules::ExecutionFailure::unreachable(error).into_error())?;
        let (rc, output, _) = connection
            .as_connection()
            .execute_command("uname -s; uname -m; id -u")
            .map_err(|error| crate::modules::ExecutionFailure::unreachable(error).into_error())?;
        if rc != 0 {
            bail!("Cannot identify target platform and login identity");
        }
        let mut lines = output.lines();
        if lines.next() != Some("Linux") {
            bail!("Native workers require a Linux target");
        }
        let arch = match lines.next() {
            Some("x86_64" | "amd64") => "x86_64",
            Some("aarch64" | "arm64") => "aarch64",
            _ => bail!("Unsupported target architecture"),
        }
        .to_owned();
        let login_uid: u32 = lines
            .next()
            .context("Missing target login identity")?
            .parse()
            .context("Invalid target login UID")?;
        let uid = if config.use_become {
            let (rc, output, _) = connection
                .as_connection()
                .execute_command(&format!("id -u -- {}", shell_quote(&config.become_user)))?;
            if rc != 0 {
                bail!("Cannot resolve target execution identity");
            }
            output.trim().parse().context("Invalid execution UID")?
        } else {
            login_uid
        };
        let artifact = manifest
            .workers
            .iter()
            .find(|artifact| artifact.arch == arch)
            .context("Release contains no worker for the target architecture")?;
        let mut binary = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(directory.join(&artifact.file))
            .context("Cannot open worker artifact")?;
        let metadata = binary.metadata()?;
        if !metadata.is_file() || metadata.len() == 0 {
            bail!("Worker artifact must be a nonempty regular file");
        }
        if rustsible_protocol::sha256(&mut binary)? != artifact.sha256.to_ascii_lowercase() {
            bail!("Worker artifact SHA-256 mismatch");
        }
        binary.rewind()?;
        let mut header = [0u8; 64];
        binary
            .read_exact(&mut header)
            .context("Incomplete worker ELF header")?;
        // Reject mislabeled architectures before a POSIX shell can try its
        // ENOEXEC script fallback on a foreign binary. Hello checks it again.
        let machine = if arch == "x86_64" { 62u16 } else { 183u16 };
        if &header[..4] != b"\x7fELF"
            || header[4..7] != [2, 1, 1]
            || !matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
            || u16::from_le_bytes([header[18], header[19]]) != machine
        {
            bail!("Worker artifact is not a Linux ELF64 executable for target architecture {arch}");
        }
        binary.rewind()?;
        let bootstrap = bootstrap(&base, metadata.len(), &artifact.sha256);
        let command = if config.use_become {
            let sudo = format!(
                "sudo -n -u {} -- sh -c {}",
                shell_quote(&config.become_user),
                shell_quote(&bootstrap)
            );
            if config.get_ssh_sudo_password().is_some() {
                format!("set -eu\nIFS= read -r rustsible_password\nprintf '%s\\n' \"$rustsible_password\" | sudo -S -p '' -u {} -v\nunset rustsible_password\n{sudo}\nrustsible_status=$?\nexit \"$rustsible_status\"", shell_quote(&config.become_user))
            } else {
                format!("{sudo}\nrustsible_status=$?\nexit \"$rustsible_status\"")
            }
        } else {
            bootstrap
        };
        let mut session = connection.start_worker(&command)?;
        // A short business-command deadline must not truncate a large binary
        // upload before that business command has even started.
        let deadline = Instant::now() + command_timeout(config)?.max(Duration::from_secs(30));
        let mut diagnostic = Vec::new();
        let initialized = (|| -> Result<Hello> {
            if config.use_become {
                if let Some(password) = config.get_ssh_sudo_password() {
                    if password.contains(['\n', '\r', '\0']) {
                        bail!("Become password cannot contain line breaks or NUL");
                    }
                    session.send(password.as_bytes(), deadline, &mut diagnostic)?;
                    session.send(b"\n", deadline, &mut diagnostic)?;
                }
            }
            session.send_file(&mut binary, deadline, &mut diagnostic)?;
            let hello: Hello = session.receive(deadline, &mut diagnostic)?;
            hello.validate(&arch, uid, module)?;
            Ok(hello)
        })();
        match initialized {
            Ok(hello) => Ok(Self {
                closed: false,
                config_key,
                session,
                hello,
                arch,
                uid,
            }),
            Err(error) => {
                let _ = session.diagnostics(&mut diagnostic);
                let cleanup = session.shutdown();
                // Bootstrap diagnostics contain fixed tool messages only;
                // requests have not been sent yet and passwords use stdin.
                let detail = String::from_utf8_lossy(&diagnostic);
                Err(error).with_context(|| format!("Native worker bootstrap failed (check executable temporary directory, architecture and sudo); {detail}; cleanup: {}", cleanup.err().map(|error| error.to_string()).unwrap_or_else(|| "completed".into())))
            }
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        if !self.closed {
            if let Err(error) = self.close() {
                eprintln!("Worker lease cleanup failed: {error:#}");
            }
        }
    }
}
// A directory descriptor pins staging across ancestor renames without changing
// the working directory inherited by user commands. All bootstrap tools inherit
// fd 3; the worker closes its copy before accepting module requests.
fn bootstrap(base: &str, length: u64, checksum: &str) -> String {
    format!(
        r#"set -eu
rustsible_umask=$(umask)
umask 077
exec 3< {base}
execution_uid=$(id -u)
root_identity=$(stat -c '%d:%i' /)
# Walk actual parent inodes in a subshell: checking path strings could race
# an ancestor rename, and business commands must retain the original cwd.
(
    CDPATH= cd -P -- /proc/self/fd/3
    while :; do
        directory_uid=$(stat -c %u .)
        directory_mode=$(stat -c %a .)
        if [ "$directory_uid" != 0 ] && [ "$directory_uid" != "$execution_uid" ]; then
            echo 'Unsafe worker directory ancestor owner; choose rustsible_remote_tmp under root or the execution identity' >&2; exit 1
        fi
        if [ $((0$directory_mode & 0022)) != 0 ] && [ $((0$directory_mode & 01000)) = 0 ]; then
            echo 'Unsafe writable worker directory ancestor: sticky bit required' >&2; exit 1
        fi
        [ "$(stat -c '%d:%i' .)" != "$root_identity" ] || break
        cd -P ..
    done
)
d=$(mktemp -d /proc/self/fd/3/.rustsible-worker-XXXXXXXXXX)
trap 'rm -rf -- "$d"' EXIT
trap 'exit 1' HUP INT TERM
head -c {length} > "$d/worker"
[ "$(wc -c < "$d/worker")" = {length} ] || {{ echo 'Incomplete worker upload' >&2; exit 1; }}
[ "$(sha256sum "$d/worker" | cut -d ' ' -f 1)" = {checksum} ] || {{ echo 'Uploaded worker checksum mismatch' >&2; exit 1; }}
chmod 0700 "$d/worker"
umask "$rustsible_umask"
RUSTSIBLE_BOOTSTRAP_DIR_FD=3 "$d/worker" --serve
"#,
        base = shell_quote(base),
        checksum = shell_quote(&checksum.to_ascii_lowercase())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    fn config(directory: &Path) -> ConnectionConfig {
        ConnectionConfig::from_context(
            "local",
            HashMap::from([
                ("ansible_connection".into(), Value::String("local".into())),
                (
                    "rustsible_remote_tmp".into(),
                    Value::String(directory.to_str().unwrap().into()),
                ),
            ]),
        )
        .unwrap()
    }
    #[test]
    fn run_reuses_one_private_worker_and_removes_every_scratch_file() {
        let temporary = tempfile::tempdir().unwrap();
        let config = config(temporary.path());
        let runtime = Runtime::default();
        for _ in 0..2 {
            let result = runtime
                .execute(&config, "setup", Value::Mapping(Default::default()), true)
                .unwrap();
            assert!(!result.failed, "{}", result.msg);
        }
        let leases = runtime.leases.lock().unwrap();
        assert_eq!(leases.len(), 1);
        drop(leases);
        let directories: Vec<_> = fs::read_dir(temporary.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(directories.len(), 1);
        assert_eq!(
            fs::metadata(&directories[0]).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::read_dir(&directories[0]).unwrap().count(),
            1,
            "request scratch survived execution"
        );
        drop(runtime);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
    }
    #[test]
    fn configuration_changes_and_explicit_reset_close_the_previous_lease() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let runtime = Runtime::default();
        let execute = |config: &ConnectionConfig| {
            let result = runtime
                .execute(config, "setup", Value::Mapping(Default::default()), true)
                .unwrap();
            assert!(!result.failed, "{}", result.msg);
        };
        execute(&config(first.path()));
        assert_eq!(fs::read_dir(first.path()).unwrap().count(), 1);
        execute(&config(second.path()));
        assert_eq!(fs::read_dir(first.path()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(second.path()).unwrap().count(), 1);
        assert_eq!(runtime.leases.lock().unwrap().len(), 1);
        let original = fs::read_dir(second.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        runtime.reset("local").unwrap();
        assert_eq!(fs::read_dir(second.path()).unwrap().count(), 0);
        execute(&config(second.path()));
        assert!(!original.exists());
        execute(&config(first.path()));
        assert_eq!(fs::read_dir(second.path()).unwrap().count(), 0);
        runtime.reset("local").unwrap();
        runtime.reset("absent").unwrap();
        assert_eq!(fs::read_dir(first.path()).unwrap().count(), 0);
    }
    #[test]
    fn unsafe_directory_and_damaged_or_missing_artifacts_fail_without_business_changes() {
        let temporary = tempfile::tempdir().unwrap();
        let business = temporary.path().join("business");
        fs::write(&business, b"original").unwrap();
        let args = serde_yaml::to_value(BTreeMap::from([(
            "cmd",
            format!(
                "printf changed > {}",
                shell_quote(business.to_str().unwrap())
            ),
        )]))
        .unwrap();
        let missing = Runtime::new(Some(temporary.path().join("absent")));
        assert!(missing
            .execute(&config(temporary.path()), "shell", args.clone(), false)
            .unwrap_err()
            .to_string()
            .contains("manifest"));
        let damaged = temporary.path().join("workers");
        fs::create_dir(&damaged).unwrap();
        let bundle = crate::testing::worker_bundle();
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
        fs::write(
            damaged.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(damaged.join(&manifest.workers[0].file), b"damaged binary").unwrap();
        assert!(Runtime::new(Some(damaged.clone()))
            .execute(&config(temporary.path()), "shell", args.clone(), false)
            .unwrap_err()
            .to_string()
            .contains("SHA-256"));
        let mut foreign = fs::read(bundle.join(&manifest.workers[0].file)).unwrap();
        let machine = if std::env::consts::ARCH == "x86_64" {
            183u16
        } else {
            62u16
        };
        foreign[18..20].copy_from_slice(&machine.to_le_bytes());
        manifest.workers[0].sha256 = rustsible_protocol::sha256(&mut foreign.as_slice()).unwrap();
        fs::write(damaged.join(&manifest.workers[0].file), foreign).unwrap();
        fs::write(
            damaged.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(Runtime::new(Some(damaged))
            .execute(&config(temporary.path()), "shell", args.clone(), false)
            .unwrap_err()
            .to_string()
            .contains("target architecture"));
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o777)).unwrap();
        let error = Runtime::default()
            .execute(&config(temporary.path()), "shell", args, false)
            .unwrap_err();
        assert!(format!("{error:#}").contains("sticky bit"), "{error:#}");
        let nested = temporary.path().join("private-base");
        fs::create_dir(&nested).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
        let error = Runtime::default()
            .execute(
                &config(&nested),
                "setup",
                Value::Mapping(Default::default()),
                false,
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("ancestor"), "{error:#}");
        assert_eq!(fs::read_dir(nested).unwrap().count(), 0);
        assert_eq!(fs::read(business).unwrap(), b"original");
        assert!(fs::read_dir(temporary.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".rustsible-worker-")));
    }
    #[test]
    fn command_files_keep_the_original_umask_while_worker_scratch_stays_private() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("business");
        let original = std::process::Command::new("sh")
            .args(["-c", "umask"])
            .output()
            .unwrap();
        let mask =
            u32::from_str_radix(std::str::from_utf8(&original.stdout).unwrap().trim(), 8).unwrap();
        let runtime = Runtime::default();
        let result = runtime
            .execute(
                &config(temporary.path()),
                "shell",
                Value::String(format!(
                    "pwd; printf x > {}",
                    shell_quote(path.to_str().unwrap())
                )),
                false,
            )
            .unwrap();
        assert!(!result.failed, "{}", result.msg);
        assert_eq!(
            result.stdout.trim(),
            std::env::current_dir().unwrap().to_str().unwrap()
        );
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o666 & !mask
        );
    }
    #[test]
    fn transport_timeout_is_uncertain_and_cleans_business_descendants_without_replay() {
        let temporary = tempfile::tempdir().unwrap();
        let marker = temporary.path().join("marker");
        let variables = HashMap::from([
            ("ansible_connection".into(), Value::String("local".into())),
            (
                "rustsible_remote_tmp".into(),
                Value::String(temporary.path().to_str().unwrap().into()),
            ),
            ("rustsible_command_timeout".into(), Value::Number(1.into())),
        ]);
        let config = ConnectionConfig::from_context("local", variables).unwrap();
        let runtime = Runtime::default();
        let result = runtime
            .execute(
                &config,
                "shell",
                Value::String(format!(
                    "printf x >> {}; sleep 60",
                    shell_quote(marker.to_str().unwrap())
                )),
                false,
            )
            .unwrap();
        assert!(result.uncertain && result.failed);
        assert_eq!(fs::read(marker).unwrap(), b"x");
        assert!(runtime
            .leases
            .lock()
            .unwrap()
            .values()
            .all(|lease| lease.lock().unwrap().is_none()));
        drop(runtime);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
    }
}
