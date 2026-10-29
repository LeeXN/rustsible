use anyhow::{bail, Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;

use super::ModuleResult;
use crate::ssh::config::ConnectionConfig;

pub use rustsible_protocol::registry::{Builtin, CheckSupport};

pub struct Invocation {
    pub module: Builtin,
    args: Value,
    check: bool,
    environment: std::collections::BTreeMap<String, String>,
    timeout_ms: Option<u64>,
    asynchronous: Option<(rustsible_protocol::AsyncSpec, u64)>,
}

impl Invocation {
    pub fn prepare(name: &str, args: &Value, check: bool) -> Result<Self> {
        let module = Builtin::lookup(name)?;
        let args = if matches!(module, Builtin::Command | Builtin::Shell) {
            if let Some(raw) = args.get("_raw_params") {
                let mut mapping = args
                    .as_mapping()
                    .context("Command requires a mapping")?
                    .clone();
                if mapping.contains_key(Value::String("cmd".into())) {
                    bail!("Command cannot specify raw text and cmd");
                }
                mapping.remove(Value::String("_raw_params".into()));
                mapping.insert(Value::String("cmd".into()), raw.clone());
                Value::Mapping(mapping)
            } else {
                args.clone()
            }
        } else {
            args.clone()
        };
        module.validate_capabilities(check, false, false)?;
        module.validate_parameters(&args)?;
        Ok(Self {
            module,
            args,
            check,
            environment: Default::default(),
            timeout_ms: None,
            asynchronous: None,
        })
    }
    pub(crate) fn with_execution(
        mut self,
        environment: std::collections::BTreeMap<String, String>,
        timeout_ms: Option<u64>,
    ) -> Self {
        self.environment = environment;
        self.timeout_ms = timeout_ms;
        self
    }
    pub(crate) fn with_async(mut self, seconds: u64, poll: u64, no_log: bool) -> Result<Self> {
        if seconds > 604800 || poll > 604800 {
            bail!("Async duration and poll interval must not exceed seven days");
        }
        if seconds != 0 {
            self.module.validate_capabilities(self.check, false, true)?;
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let counter = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let bytes = format!("{timestamp}-{}-{counter}", std::process::id()).into_bytes();
            let jid = format!("j{}", rustsible_protocol::sha256(&mut bytes.as_slice())?);
            self.asynchronous = Some((
                rustsible_protocol::AsyncSpec {
                    jid,
                    timeout_ms: seconds * 1000,
                    directory: None,
                    no_log,
                },
                poll,
            ));
        }
        Ok(self)
    }
    fn skip_check(&self) -> bool {
        self.check
            && (self.asynchronous.is_some()
                || (matches!(self.module, Builtin::Command | Builtin::Shell)
                    && self.args.get("creates").is_none()
                    && self.args.get("removes").is_none()))
    }
    pub fn needs_connection(&self) -> bool {
        !matches!(
            self.module,
            Builtin::Debug
                | Builtin::Assert
                | Builtin::Fail
                | Builtin::SetFact
                | Builtin::IncludeVars
                | Builtin::Meta
        ) && !self.skip_check()
    }
    pub(crate) fn execute(
        mut self,
        config: Option<&ConnectionConfig>,
        vars: &HashMap<String, Value>,
        runtime: &crate::worker::Runtime,
    ) -> ModuleResult {
        self.check |= super::result::is_check_active();
        let _changes = super::result::ChangeScope::enter(self.check);
        if self.skip_check() {
            return ModuleResult {
                skipped: true,
                msg: if self.asynchronous.is_some() {
                    format!(
                        "Check mode: async {} task was not started",
                        self.module.name()
                    )
                } else {
                    format!(
                        "Check mode: {} was not executed because it has no safe change prediction",
                        self.module.name()
                    )
                },
                ..ModuleResult::default()
            };
        }
        let result = (|| {
            if self.module == Builtin::Debug {
                return super::debug::execute_without_connection(&self.args);
            }
            if matches!(self.module, Builtin::Assert | Builtin::Fail) {
                return super::assertion::execute(self.module, &self.args, vars);
            }
            if self.module == Builtin::IncludeVars {
                return crate::vars::include(&self.args);
            }
            if self.module == Builtin::SetFact {
                let mut facts = self
                    .args
                    .as_mapping()
                    .context("set_fact requires a mapping")?
                    .clone();
                facts.remove(Value::String("cacheable".into()));
                return Ok(ModuleResult {
                    values: HashMap::from([("ansible_facts".into(), Value::Mapping(facts))]),
                    ..Default::default()
                });
            }
            if self.module == Builtin::Meta {
                if self.args.get("action").and_then(Value::as_str) != Some("reset_connection") {
                    bail!("This meta action requires playbook execution");
                }
                let host = vars
                    .get("inventory_hostname")
                    .and_then(Value::as_str)
                    .context("Connection reset requires an inventory host")?;
                runtime.reset(host)?;
                return Ok(ModuleResult {
                    msg: "Connection reset".into(),
                    ..Default::default()
                });
            }
            let config = config.context("Target module needs a connection configuration")?;
            let connection_timeout: u64 = crate::ssh::connection::command_timeout(config)?
                .as_millis()
                .try_into()?;
            let timeout_ms = if self.asynchronous.is_some() {
                connection_timeout
            } else {
                self.timeout_ms.unwrap_or(connection_timeout)
            };
            if let Some((specification, _)) = self.asynchronous.as_mut() {
                specification.directory = config
                    .get_variable("ansible_async_dir")
                    .map(|path| path.into_owned());
                if let Some(limit) = self.timeout_ms {
                    specification.timeout_ms = specification.timeout_ms.min(limit);
                }
            }
            if self.module == Builtin::AsyncStatus && self.args.get("_async_dir").is_none() {
                if let Some(directory) = config.get_variable("ansible_async_dir") {
                    self.args
                        .as_mapping_mut()
                        .context("Async status requires a mapping")?
                        .insert(
                            Value::String("_async_dir".into()),
                            Value::String(directory.into_owned()),
                        );
                }
            }
            let mut request = rustsible_protocol::Request::new(
                self.module.name(),
                self.args.clone(),
                self.check,
                timeout_ms,
            );
            request.environment = self.environment.clone();
            request.asynchronous = self
                .asynchronous
                .as_ref()
                .map(|(specification, _)| specification.clone());
            if matches!(self.module, Builtin::Copy | Builtin::Template) {
                let mut args = self.args.clone();
                let mut source = if self.module == Builtin::Template {
                    crate::worker::Source::bytes(super::template::render(&args, vars)?.into_bytes())
                } else if let Some(src) = args.get("src").and_then(Value::as_str) {
                    crate::worker::Source::file(src)?
                } else {
                    crate::worker::Source::bytes(
                        args.get("content")
                            .and_then(Value::as_str)
                            .context("Copy requires string content")?
                            .as_bytes()
                            .to_vec(),
                    )
                };
                if let Some(mapping) = args.as_mapping_mut() {
                    for name in ["src", "content", "vars"] {
                        mapping.remove(Value::String(name.into()));
                    }
                }
                request.args = args;
                return runtime.execute_request(config, request, Some(&mut source));
            }
            let mut launched = runtime.execute_request(config, request, None)?;
            if let Some((specification, poll)) = &self.asynchronous {
                launched
                    .values
                    .entry("ansible_job_id".into())
                    .or_insert_with(|| Value::String(specification.jid.clone()));
                launched.no_log |= specification.no_log;
                if *poll > 0 && launched.outcome() == crate::modules::Outcome::Ok {
                    return self.poll(
                        config,
                        runtime,
                        specification,
                        *poll,
                        connection_timeout,
                        launched.changed,
                    );
                }
            }
            Ok(launched)
        })();
        match result {
            Ok(mut result) => {
                result.failed |= result.rc.is_some_and(|rc| rc != 0);
                result.changed |= super::result::known_changed();
                result
            }
            Err(error) => {
                let mut result = crate::modules::result::from_error(
                    error,
                    self.asynchronous.is_some() || self.module.may_modify(&self.args, self.check),
                );
                if let Some((specification, _)) = &self.asynchronous {
                    result.values.insert(
                        "ansible_job_id".into(),
                        Value::String(specification.jid.clone()),
                    );
                    result.no_log |= specification.no_log;
                }
                result
            }
        }
    }
    fn poll(
        &self,
        config: &ConnectionConfig,
        runtime: &crate::worker::Runtime,
        specification: &rustsible_protocol::AsyncSpec,
        interval: u64,
        timeout: u64,
        changed: bool,
    ) -> Result<ModuleResult> {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(specification.timeout_ms)
            + std::time::Duration::from_secs(15);
        loop {
            let mut args = serde_yaml::Mapping::from_iter([(
                Value::String("jid".into()),
                Value::String(specification.jid.clone()),
            )]);
            if let Some(directory) = &specification.directory {
                args.insert(
                    Value::String("_async_dir".into()),
                    Value::String(directory.clone()),
                );
            }
            let mut result = runtime.execute_request(
                config,
                rustsible_protocol::Request::new(
                    "async_status",
                    Value::Mapping(args.clone()),
                    false,
                    timeout,
                ),
                None,
            )?;
            result
                .values
                .entry("ansible_job_id".into())
                .or_insert_with(|| Value::String(specification.jid.clone()));
            result.no_log |= specification.no_log;
            result.changed |= changed;
            if result.values.get("finished").and_then(Value::as_bool) == Some(true) {
                args.insert(
                    Value::String("mode".into()),
                    Value::String("cleanup".into()),
                );
                let cleaned = runtime.execute_request(
                    config,
                    rustsible_protocol::Request::new(
                        "async_status",
                        Value::Mapping(args),
                        false,
                        timeout,
                    ),
                    None,
                )?;
                if cleaned.failed {
                    result.failed = true;
                    result.msg = format!(
                        "{}; async result cleanup failed: {}",
                        result.msg, cleaned.msg
                    );
                }
                return Ok(result);
            }
            if result.failed || result.unreachable || result.uncertain {
                return Ok(result);
            }
            if std::time::Instant::now() >= deadline {
                result.failed = true;
                result.uncertain = true;
                result.msg =
                    "Async supervisor did not confirm a final result; query the retained job ID"
                        .into();
                return Ok(result);
            }
            std::thread::sleep(
                std::time::Duration::from_secs(interval)
                    .min(deadline.saturating_duration_since(std::time::Instant::now())),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_capabilities_have_one_contract() {
        for module in Builtin::ALL {
            for name in [
                module.name().to_string(),
                format!("ansible.builtin.{}", module.name()),
                format!("ansible.legacy.{}", module.name()),
            ] {
                assert_eq!(Builtin::lookup(&name).unwrap(), module);
            }
            assert!(module.validate_capabilities(true, false, false).is_ok());
            assert!(module.validate_capabilities(false, true, false).is_err());
        }
        assert!(Builtin::Command
            .validate_capabilities(false, false, true)
            .is_ok());
        assert!(Builtin::Copy
            .validate_capabilities(false, false, true)
            .is_err());
        assert_eq!(
            Builtin::lookup("ansible.builtin.systemd_service").unwrap(),
            Builtin::Systemd
        );
        assert!(Builtin::lookup("custom.python_module").is_err());
    }
    #[test]
    fn invalid_parameters_are_rejected_without_opening_a_connection() {
        for (module, yaml) in [
            ("copy", "content: x\ndest: /tmp/x\nunknown: true"),
            ("systemd", "name: demo\nstate: invalid\ndaemon_reload: true"),
            ("systemd", "daemon_reload: true\nenabled: true"),
            ("command", "cmd: echo x\nunknown: x"),
            ("file", "path: /tmp/x\nmode: '888'"),
            ("user", "name: x\nuid: -1"),
            (
                "lineinfile",
                "path: /tmp/x\nline: x\ninsertafter: EOF\ninsertbefore: BOF",
            ),
        ] {
            let args = serde_yaml::from_str(yaml).unwrap();
            for check in [false, true] {
                assert!(
                    Invocation::prepare(module, &args, check).is_err(),
                    "{module}"
                );
            }
        }
        let invocation =
            Invocation::prepare("shell", &Value::String("echo x".into()), true).unwrap();
        assert!(!invocation.needs_connection());
        assert_eq!(
            invocation
                .execute(None, &HashMap::new(), &crate::worker::Runtime::default())
                .outcome(),
            super::super::Outcome::Skipped
        );
    }
}
