use anyhow::{bail, Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;

use super::ModuleResult;
use crate::ssh::{config::ConnectionConfig, connection::Connection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckSupport {
    ReadOnly,
    Predict,
    Skip,
}

/// A single registry owns public names, parameter schemas, capability checks
/// and dispatch. Frontends only add their own variable/source context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Command,
    Shell,
    Debug,
    Copy,
    File,
    Template,
    Lineinfile,
    User,
    Service,
    Systemd,
    Setup,
    Stat,
    GetUrl,
    Package,
}

impl Builtin {
    pub const ALL: [Self; 14] = [
        Self::Command,
        Self::Shell,
        Self::Debug,
        Self::Copy,
        Self::File,
        Self::Template,
        Self::Lineinfile,
        Self::User,
        Self::Service,
        Self::Systemd,
        Self::Setup,
        Self::Stat,
        Self::GetUrl,
        Self::Package,
    ];

    pub fn lookup(name: &str) -> Result<Self> {
        let short = name
            .strip_prefix("ansible.builtin.")
            .or_else(|| name.strip_prefix("ansible.legacy."))
            .unwrap_or(name);
        Self::ALL
            .into_iter()
            .find(|module| {
                module.name() == short || (*module == Self::Systemd && short == "systemd_service")
            })
            .with_context(|| format!("Unsupported module: {name}"))
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Shell => "shell",
            Self::Debug => "debug",
            Self::Copy => "copy",
            Self::File => "file",
            Self::Template => "template",
            Self::Lineinfile => "lineinfile",
            Self::User => "user",
            Self::Service => "service",
            Self::Systemd => "systemd",
            Self::Setup => "setup",
            Self::Stat => "stat",
            Self::GetUrl => "get_url",
            Self::Package => "package",
        }
    }
    pub fn check_support(self) -> CheckSupport {
        match self {
            Self::Debug | Self::Setup | Self::Stat => CheckSupport::ReadOnly,
            Self::Command | Self::Shell => CheckSupport::Skip,
            _ => CheckSupport::Predict,
        }
    }
    pub fn validate_capabilities(self, _check: bool, diff: bool, asynchronous: bool) -> Result<()> {
        if diff {
            bail!("Module '{}' does not support diff", self.name());
        }
        if asynchronous {
            bail!("Module '{}' does not support async", self.name());
        }
        Ok(())
    }
    pub fn parameters(self) -> &'static [&'static str] {
        match self {
            Self::Command | Self::Shell => &["cmd"],
            Self::Debug => &["msg", "var", "_var_value"],
            Self::Copy => &["src", "content", "dest", "mode", "owner", "group", "force"],
            Self::File => &["path", "dest", "state", "src", "mode", "owner", "group"],
            Self::Template => &["src", "content", "dest", "mode", "owner", "group", "vars"],
            Self::Lineinfile => &[
                "path",
                "line",
                "regexp",
                "state",
                "backup",
                "create",
                "insertafter",
                "insertbefore",
                "owner",
                "group",
                "mode",
            ],
            Self::User => &[
                "name",
                "state",
                "uid",
                "gid",
                "group",
                "groups",
                "append",
                "home",
                "shell",
                "comment",
                "password",
                "create_home",
                "system",
                "remove",
            ],
            Self::Service => &["name", "state", "enabled"],
            Self::Systemd => &["name", "state", "enabled", "daemon_reload"],
            Self::Setup => &[],
            Self::Stat => &["path", "checksum_algorithm"],
            Self::GetUrl => &["url", "dest", "owner", "group", "mode", "force", "checksum"],
            Self::Package => &["name", "state", "update_cache"],
        }
    }

    /// Shared structural/type validation also runs in direct module APIs,
    /// before an implementation can perform its first state change.
    pub fn validate_parameters(self, args: &Value) -> Result<()> {
        if matches!(self, Self::Command | Self::Shell) {
            let command = super::extract_command_arg(args)?;
            if command.trim().is_empty() || command.contains('\0') {
                bail!("Command must be non-empty without NUL bytes");
            }
            if self == Self::Command {
                let argv = super::tokenize_args(&command)?;
                if argv.is_empty() {
                    bail!("Command must be non-empty");
                }
                for argument in argv {
                    super::file::quote_posix_shell_arg(&argument)?;
                }
            }
            return Ok(());
        }
        super::param::validate_params(args, self.parameters())?;
        let map = args
            .as_mapping()
            .context("Module arguments must be a mapping")?;
        for (key, value) in map {
            let key = key
                .as_str()
                .context("Module parameter names must be strings")?;
            let valid = match key {
                "msg" | "var" | "_var_value" if self == Self::Debug => true,
                "vars" => value.is_mapping(),
                "force" | "enabled" | "daemon_reload" | "backup" | "create" | "append"
                | "create_home" | "system" | "remove" | "update_cache" => value.is_bool(),
                "uid" | "gid" if self == Self::User => {
                    value.as_u64().is_some_and(|id| id < u32::MAX as u64)
                }
                "groups" if self == Self::User => {
                    value.is_string()
                        || value
                            .as_sequence()
                            .is_some_and(|values| values.iter().all(Value::is_string))
                }
                "name" if self == Self::Package => {
                    value.is_string()
                        || value.as_sequence().is_some_and(|values| {
                            !values.is_empty() && values.iter().all(Value::is_string)
                        })
                }
                _ => value.is_string(),
            };
            if !valid {
                bail!("Invalid type for {} parameter '{key}'", self.name());
            }
            let binary_content = key == "content" && matches!(self, Self::Copy | Self::Template);
            if !binary_content && value.as_str().is_some_and(|value| value.contains('\0')) {
                bail!("Module parameter '{key}' cannot contain NUL bytes");
            }
        }
        let string = |key: &str| args.get(key).and_then(Value::as_str);
        let required = |key: &str| -> Result<()> {
            if string(key).is_none_or(str::is_empty) {
                bail!("{} requires a non-empty '{key}' parameter", self.name());
            }
            Ok(())
        };
        match self {
            Self::Copy | Self::Template => {
                required("dest")?;
                if args.get("src").is_some() == args.get("content").is_some() {
                    bail!("{} requires exactly one of src or content", self.name());
                }
                if args.get("src").is_some() {
                    required("src")?;
                }
            }
            Self::File => {
                if args.get("path").is_some() == args.get("dest").is_some() {
                    bail!("File requires exactly one of path or dest");
                }
                required(if args.get("path").is_some() {
                    "path"
                } else {
                    "dest"
                })?;
                if string("state") == Some("link") {
                    required("src")?;
                }
            }
            Self::GetUrl => {
                required("url")?;
                required("dest")?;
            }
            Self::Stat | Self::Lineinfile => required("path")?,
            Self::Service | Self::Systemd => {
                if self == Self::Service
                    || args.get("name").is_some()
                    || args.get("state").is_some()
                    || args.get("enabled").is_some()
                {
                    required("name")?;
                }
                if string("name").is_some_and(|name| name.starts_with('-')) {
                    bail!("Service name cannot begin with '-'");
                }
                if args.get("state").is_none()
                    && args.get("enabled").is_none()
                    && !(self == Self::Systemd
                        && args.get("daemon_reload").and_then(Value::as_bool) == Some(true))
                {
                    bail!("Service requires state, enabled, or systemd daemon_reload=true");
                }
            }
            Self::User => required("name")?,
            Self::Package if args.get("name").is_none() => bail!("Package requires name"),
            _ => {}
        }
        let states: &[&str] = match self {
            Self::File => &["file", "directory", "link", "absent", "touch"],
            Self::User | Self::Lineinfile => &["present", "absent"],
            Self::Package => &["present", "absent", "latest"],
            Self::Service | Self::Systemd => &["started", "stopped", "restarted", "reloaded"],
            _ => &[],
        };
        if let Some(state) = string("state") {
            if !states.contains(&state.to_ascii_lowercase().as_str()) {
                bail!("Invalid {} state: {state}", self.name());
            }
        }
        if self != Self::User {
            if let Some(mode) = string("mode") {
                super::param::parse_mode(mode)?;
            }
            for key in ["owner", "group"] {
                if let Some(value) = string(key) {
                    super::param::validate_identity(value)?;
                }
            }
        }
        if self == Self::Stat {
            super::param::digest_hex_length(string("checksum_algorithm").unwrap_or("sha1"))?;
        }
        if self == Self::GetUrl {
            if let Some(checksum) = string("checksum") {
                let (algorithm, digest) = checksum
                    .split_once(':')
                    .context("checksum must use <algorithm>:<hex digest> format")?;
                if digest.len() != super::param::digest_hex_length(algorithm)?
                    || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    bail!("checksum must use <algorithm>:<hex digest> format");
                }
            }
        }
        if self == Self::Lineinfile {
            if args.get("insertafter").is_some() && args.get("insertbefore").is_some() {
                bail!("insertafter and insertbefore are mutually exclusive");
            }
            if string("line").is_some_and(|line| line.contains(['\r', '\n'])) {
                bail!("line must be a single line");
            }
            if let Some(pattern) = string("regexp") {
                regex::Regex::new(pattern)?;
            }
            if string("state") == Some("absent")
                && args.get("line").is_none()
                && args.get("regexp").is_none()
            {
                bail!("lineinfile state=absent requires line or regexp");
            }
            if string("state").unwrap_or("present") == "present" && args.get("line").is_none() {
                bail!("lineinfile state=present requires line");
            }
        }
        Ok(())
    }
}

pub struct Invocation {
    pub module: Builtin,
    args: Value,
    check: bool,
}

impl Invocation {
    pub fn prepare(name: &str, args: &Value, check: bool) -> Result<Self> {
        let module = Builtin::lookup(name)?;
        let args = if matches!(module, Builtin::Command | Builtin::Shell) {
            if let Some(raw) = args.get("_raw_params") {
                if args.as_mapping().is_none_or(|map| map.len() != 1) {
                    bail!("Raw command cannot be combined with other parameters");
                }
                raw.clone()
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
        })
    }
    pub fn needs_connection(&self) -> bool {
        self.module != Builtin::Debug
            && !(self.check && self.module.check_support() == CheckSupport::Skip)
    }
    pub(crate) fn execute(
        mut self,
        config: Option<&ConnectionConfig>,
        vars: &HashMap<String, Value>,
        runtime: &crate::worker::Runtime,
    ) -> ModuleResult {
        self.check |= super::result::is_check_active();
        let _changes = super::result::ChangeScope::enter(self.check);
        if self.check && self.module.check_support() == CheckSupport::Skip {
            return ModuleResult {
                skipped: true,
                msg: format!(
                    "Check mode: {} was not executed because it has no safe change prediction",
                    self.module.name()
                ),
                ..ModuleResult::default()
            };
        }
        let result = (|| {
            if self.module == Builtin::Debug {
                return super::debug::execute_without_connection(&self.args);
            }
            let config = config.context("Target module needs a connection configuration")?;
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
                return runtime.execute_with_source(
                    config,
                    self.module.name(),
                    args,
                    self.check,
                    Some(&mut source),
                );
            }
            if rustsible_protocol::NATIVE_MODULES.contains(&self.module.name()) {
                return runtime.execute(config, self.module.name(), self.args.clone(), self.check);
            }
            let connection = Connection::from_config(config)
                .map_err(|error| super::ExecutionFailure::unreachable(error).into_error())?;
            self.dispatch(
                connection.as_connection(),
                config.use_become,
                &config.become_user,
            )
        })();
        match result {
            Ok(mut result) => {
                result.failed |= result.rc.is_some_and(|rc| rc != 0);
                result.changed |= super::result::known_changed();
                result
            }
            Err(error) => crate::modules::result::from_error(
                error,
                self.module.check_support() != CheckSupport::ReadOnly && !self.check,
            ),
        }
    }
    fn dispatch(
        &self,
        connection: &dyn crate::ssh::connection::SshConnection,
        use_become: bool,
        user: &str,
    ) -> Result<ModuleResult> {
        let args = &self.args;
        match self.module {
            Builtin::Command | Builtin::Shell | Builtin::Setup | Builtin::Stat => {
                unreachable!("Native modules use the worker")
            }
            Builtin::Copy | Builtin::Template | Builtin::File | Builtin::Lineinfile => {
                unreachable!("Native file modules use the worker")
            }
            Builtin::User => super::user::execute(connection, args, use_become, user, self.check),
            Builtin::Service => {
                super::service::execute(connection, args, use_become, user, self.check)
            }
            Builtin::Systemd => {
                super::service::execute_systemd(connection, args, use_become, user, self.check)
            }
            Builtin::GetUrl => unreachable!("Native downloads use the worker"),
            Builtin::Package => {
                super::package::execute(connection, args, use_become, user, self.check)
            }
            Builtin::Debug => unreachable!("Debug is controller-side"),
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
            assert!(module.validate_capabilities(false, false, true).is_err());
        }
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
