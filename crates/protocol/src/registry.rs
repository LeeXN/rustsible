use anyhow::{bail, Context, Result};
use serde_yaml::Value;
pub const SETUP_SUBSETS: &[&str] = &[
    "all",
    "min",
    "platform",
    "distribution",
    "network",
    "architecture",
    "system",
    "kernel",
    "machine",
    "distribution_version",
    "distribution_major_version",
    "distribution_release",
    "os_family",
    "date_time",
    "interfaces",
    "all_ipv4_addresses",
    "all_ipv6_addresses",
    "default_ipv4",
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckSupport {
    ReadOnly,
    Predict,
}

/// One registry owns public names, schemas and capabilities for both the
/// controller frontends and worker; transport and variable context stay out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Command,
    Shell,
    Debug,
    Meta,
    SetFact,
    IncludeVars,
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
    AsyncStatus,
    Assert,
    Fail,
    Group,
}

impl Builtin {
    pub const ALL: [Self; 21] = [
        Self::Command,
        Self::Shell,
        Self::Debug,
        Self::Meta,
        Self::SetFact,
        Self::IncludeVars,
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
        Self::AsyncStatus,
        Self::Assert,
        Self::Fail,
        Self::Group,
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
            Self::Meta => "meta",
            Self::SetFact => "set_fact",
            Self::IncludeVars => "include_vars",
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
            Self::AsyncStatus => "async_status",
            Self::Assert => "assert",
            Self::Fail => "fail",
            Self::Group => "group",
        }
    }
    pub fn check_support(self) -> CheckSupport {
        match self {
            Self::Debug
            | Self::Assert
            | Self::Fail
            | Self::Meta
            | Self::SetFact
            | Self::IncludeVars
            | Self::Setup
            | Self::Stat => CheckSupport::ReadOnly,
            _ => CheckSupport::Predict,
        }
    }
    pub fn validate_capabilities(self, _check: bool, diff: bool, asynchronous: bool) -> Result<()> {
        if diff {
            bail!("Module '{}' does not support diff", self.name());
        }
        if asynchronous && !self.supports_async() {
            bail!("Module '{}' does not support async", self.name());
        }
        Ok(())
    }
    pub fn supports_async(self) -> bool {
        matches!(
            self,
            Self::Command
                | Self::Shell
                | Self::User
                | Self::Service
                | Self::Systemd
                | Self::Package
                | Self::File
                | Self::Lineinfile
                | Self::GetUrl
        )
    }
    pub fn may_modify(self, args: &Value, check: bool) -> bool {
        !check
            && if self == Self::AsyncStatus {
                args.get("mode").and_then(Value::as_str) == Some("cleanup")
            } else {
                self.check_support() != CheckSupport::ReadOnly
            }
    }
    pub fn parameters(self) -> &'static [&'static str] {
        match self {
            Self::Command | Self::Shell => &[
                "cmd",
                "argv",
                "chdir",
                "creates",
                "removes",
                "stdin",
                "stdin_add_newline",
                "strip_empty_ends",
                "expand_argument_vars",
            ],
            Self::Debug => &["msg", "var", "_var_value"],
            Self::Meta => &["action"],
            Self::SetFact => &[],
            Self::IncludeVars => &["file", "dir", "name"],
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
            Self::Setup => &["gather_subset", "filter", "gather_timeout"],
            Self::Stat => &["path", "checksum_algorithm"],
            Self::GetUrl => &["url", "dest", "owner", "group", "mode", "force", "checksum"],
            Self::Package => &["name", "state", "update_cache"],
            Self::AsyncStatus => &["jid", "mode", "_async_dir"],
            Self::Assert => &["that", "fail_msg", "msg", "success_msg", "quiet"],
            Self::Fail => &["msg"],
            Self::Group => &["name", "gid", "state", "system"],
        }
    }

    /// Shared structural/type validation also runs in direct module APIs,
    /// before an implementation can perform its first state change.
    pub fn validate_parameters(self, args: &Value) -> Result<()> {
        if self == Self::SetFact {
            let mapping = args
                .as_mapping()
                .context("set_fact requires a variable mapping")?;
            if mapping.is_empty() || mapping.keys().all(|key| key.as_str() == Some("cacheable")) {
                bail!("set_fact requires at least one variable");
            }
            for (name, value) in mapping {
                let name = name.as_str().context("set_fact names must be strings")?;
                if name == "cacheable" {
                    if value.as_bool() != Some(false) {
                        bail!("Persistent fact caching is not supported; cacheable must be false");
                    }
                } else {
                    crate::param::validate_variable_name(name, "set_fact")?;
                }
            }
            return Ok(());
        }
        if matches!(self, Self::Command | Self::Shell) {
            crate::CommandArgs::parse(args, self == Self::Shell)?;
            return Ok(());
        }
        crate::param::validate_params(args, self.parameters())?;
        let map = args
            .as_mapping()
            .context("Module arguments must be a mapping")?;
        for (key, value) in map {
            let key = key
                .as_str()
                .context("Module parameter names must be strings")?;
            let valid = match key {
                "gather_subset" | "filter" if self == Self::Setup => {
                    value.is_string()
                        || value
                            .as_sequence()
                            .is_some_and(|values| values.iter().all(Value::is_string))
                }
                "gather_timeout" if self == Self::Setup => value
                    .as_u64()
                    .is_some_and(|value| (1..=604800).contains(&value)),
                "msg" | "var" | "_var_value" if self == Self::Debug => true,
                "that" if self == Self::Assert => {
                    value.is_bool()
                        || value.is_string()
                        || value.as_sequence().is_some_and(|conditions| {
                            conditions
                                .iter()
                                .all(|value| value.is_string() || value.is_bool())
                        })
                }
                "quiet" if self == Self::Assert => value.is_bool(),
                "vars" => value.is_mapping(),
                "force" | "enabled" | "daemon_reload" | "backup" | "create" | "append"
                | "create_home" | "system" | "remove" | "update_cache" => value.is_bool(),
                "uid" | "gid" if matches!(self, Self::User | Self::Group) => {
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
            Self::IncludeVars => {
                if args.get("file").is_some() == args.get("dir").is_some() {
                    bail!("include_vars requires exactly one of file or dir");
                }
                required(if args.get("file").is_some() {
                    "file"
                } else {
                    "dir"
                })?;
                if let Some(name) = string("name") {
                    crate::param::validate_variable_name(name, "include_vars name")?;
                }
            }
            Self::Assert => {
                if args.get("that").is_none() {
                    bail!("assert requires that");
                }
                if args.get("fail_msg").is_some() && args.get("msg").is_some() {
                    bail!("assert fail_msg and its msg alias are mutually exclusive");
                }
            }
            Self::Group => {
                required("name")?;
            }
            Self::AsyncStatus => {
                required("jid")?;
                crate::validate_job_id(string("jid").expect("Required job ID"))?;
                if let Some(directory) = string("_async_dir") {
                    crate::validate_async_directory(directory)?;
                }
                if !matches!(string("mode"), None | Some("status" | "cleanup")) {
                    bail!("Async status mode must be status or cleanup");
                }
            }
            Self::Meta => {
                if !matches!(
                    string("action"),
                    Some(
                        "reset_connection"
                            | "flush_handlers"
                            | "clear_host_errors"
                            | "end_host"
                            | "end_play"
                    )
                ) {
                    bail!("Unsupported meta action");
                }
            }
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
            Self::User | Self::Lineinfile | Self::Group => &["present", "absent"],
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
            if let Some(mode) = string("mode").filter(|_| self != Self::AsyncStatus) {
                crate::param::parse_mode(mode)?;
            }
            for key in ["owner", "group"] {
                if let Some(value) = string(key) {
                    crate::param::validate_identity(value)?;
                }
            }
        }
        if self == Self::Stat {
            crate::param::digest_hex_length(string("checksum_algorithm").unwrap_or("sha1"))?;
        }
        if self == Self::Setup {
            if let Some(filters) = args.get("filter").and_then(Value::as_sequence) {
                if filters
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|value| value.contains('\0'))
                {
                    bail!("Fact filters cannot contain NUL bytes");
                }
            }
            if let Some(subsets) = args.get("gather_subset") {
                let names = if let Some(value) = subsets.as_str() {
                    value.split(',').collect::<Vec<_>>()
                } else {
                    subsets
                        .as_sequence()
                        .context("gather_subset requires strings")?
                        .iter()
                        .filter_map(Value::as_str)
                        .collect()
                };
                for name in names {
                    if !SETUP_SUBSETS.contains(&name.strip_prefix('!').unwrap_or(name)) {
                        bail!("Unsupported native facts subset: {name}");
                    }
                }
            }
        }
        if self == Self::GetUrl {
            if let Some(checksum) = string("checksum") {
                let (algorithm, digest) = checksum
                    .split_once(':')
                    .context("checksum must use <algorithm>:<hex digest> format")?;
                if digest.len() != crate::param::digest_hex_length(algorithm)?
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
