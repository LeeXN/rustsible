use crate::inventory::Inventory;
use anyhow::{bail, Context, Result};
use serde_yaml::Value;
use std::{
    collections::HashMap,
    fs,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[derive(Debug, Default)]
pub struct Config {
    pub file: Option<PathBuf>,
    pub inventory: Option<PathBuf>,
    pub worker_dir: Option<PathBuf>,
    pub role_paths: Vec<PathBuf>,
    pub forks: Option<usize>,
    pub verbosity: Option<u8>,
    defaults: HashMap<String, Value>,
}
impl Config {
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let file = if let Some(path) = explicit {
            Some(path.to_path_buf())
        } else if let Some(path) = std::env::var_os("ANSIBLE_CONFIG") {
            let path = PathBuf::from(path);
            Some(if path.is_dir() {
                path.join("ansible.cfg")
            } else {
                path
            })
        } else {
            let mut candidates = vec![std::env::current_dir()?.join("ansible.cfg")];
            if let Some(home) = std::env::var_os("HOME") {
                candidates.push(PathBuf::from(home).join(".ansible.cfg"));
            }
            candidates.push(PathBuf::from("/etc/ansible/ansible.cfg"));
            candidates.into_iter().find(|path| path.is_file())
        };
        let mut config = if let Some(path) = file {
            let source = path
                .canonicalize()
                .with_context(|| format!("Cannot resolve configuration {}", path.display()))?;
            let mut file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&source)
                .with_context(|| format!("Cannot open configuration {}", source.display()))?;
            if !file.metadata()?.is_file() {
                bail!("Configuration must be a regular file: {}", source.display());
            }
            let mut content = String::new();
            file.read_to_string(&mut content)
                .with_context(|| format!("Cannot read configuration {}", source.display()))?;
            Self::parse(&content, &source)?
        } else {
            Self::default()
        };
        for (name, section, key) in [
            ("ANSIBLE_INVENTORY", "defaults", "inventory"),
            ("ANSIBLE_FORKS", "defaults", "forks"),
            ("ANSIBLE_ROLES_PATH", "defaults", "roles_path"),
            ("ANSIBLE_VERBOSITY", "defaults", "verbosity"),
            ("ANSIBLE_REMOTE_USER", "defaults", "remote_user"),
            ("ANSIBLE_PRIVATE_KEY_FILE", "defaults", "private_key_file"),
            ("ANSIBLE_TIMEOUT", "defaults", "timeout"),
            ("ANSIBLE_HOST_KEY_CHECKING", "defaults", "host_key_checking"),
            ("ANSIBLE_BECOME", "privilege_escalation", "become"),
            ("ANSIBLE_BECOME_USER", "privilege_escalation", "become_user"),
            (
                "ANSIBLE_BECOME_METHOD",
                "privilege_escalation",
                "become_method",
            ),
        ] {
            if let Ok(value) = std::env::var(name) {
                config
                    .set(section, key, &value, &std::env::current_dir()?)
                    .with_context(|| format!("Invalid environment option {name}"))?;
            }
        }
        Ok(config)
    }
    fn parse(content: &str, source: &Path) -> Result<Self> {
        let mut config = Self {
            file: Some(source.to_path_buf()),
            ..Default::default()
        };
        let mut section = String::new();
        let base = source
            .parent()
            .context("Configuration has no parent directory")?;
        for (index, raw) in content.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_ascii_lowercase();
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .or_else(|| line.split_once(':'))
                .with_context(|| format!("Configuration line {} must use key=value", index + 1))?;
            let value = value.trim();
            let value = if let Some(quote) = value
                .chars()
                .next()
                .filter(|character| matches!(character, '\'' | '"'))
            {
                value
                    .strip_prefix(quote)
                    .and_then(|value| value.strip_suffix(quote))
                    .context("Unterminated quoted configuration value")?
            } else {
                value
            };
            config
                .set(&section, &key.trim().to_ascii_lowercase(), value, base)
                .with_context(|| {
                    format!(
                        "{}:{}: unsupported or invalid configuration option [{}].{}",
                        source.display(),
                        index + 1,
                        section,
                        key.trim()
                    )
                })?;
        }
        Ok(config)
    }
    fn set(&mut self, section: &str, key: &str, value: &str, base: &Path) -> Result<()> {
        if value.is_empty() {
            bail!("Configuration value must not be empty");
        }
        match (section, key) {
            ("defaults", "inventory") => self.inventory = Some(resolve_path(base, value)?),
            ("defaults", "worker_dir") => self.worker_dir = Some(resolve_path(base, value)?),
            ("defaults", "roles_path") => {
                self.role_paths = std::env::split_paths(value)
                    .map(|path| resolve_path(base, &path.to_string_lossy()))
                    .collect::<Result<Vec<_>>>()?;
            }
            ("defaults", "forks") => {
                let count = value.parse::<usize>().context("forks must be an integer")?;
                if count == 0 {
                    bail!("forks must be positive");
                }
                self.forks = Some(count);
            }
            ("defaults", "verbosity") => {
                let level = value
                    .parse::<u8>()
                    .context("verbosity must be an integer")?;
                if level > 4 {
                    bail!("verbosity must be between zero and four");
                }
                self.verbosity = Some(level);
            }
            ("defaults", "remote_user") => {
                self.defaults
                    .insert("ansible_user".into(), Value::String(value.into()));
            }
            ("defaults", "private_key_file") => {
                self.defaults.insert(
                    "ansible_ssh_private_key_file".into(),
                    Value::String(resolve_path(base, value)?.to_string_lossy().into_owned()),
                );
            }
            ("defaults", "timeout") => {
                let seconds = value
                    .parse::<u64>()
                    .context("timeout must be a positive integer")?;
                if seconds == 0 || seconds > u32::MAX as u64 / 1000 {
                    bail!("timeout is outside the supported range");
                }
                self.defaults
                    .insert("ansible_ssh_timeout".into(), Value::Number(seconds.into()));
            }
            ("defaults", "host_key_checking") => {
                self.defaults.insert(
                    "rustsible_host_key_checking".into(),
                    Value::Bool(boolean(value)?),
                );
            }
            ("privilege_escalation", "become") => {
                self.defaults
                    .insert("ansible_become".into(), Value::Bool(boolean(value)?));
            }
            ("privilege_escalation", "become_user") => {
                self.defaults
                    .insert("ansible_become_user".into(), Value::String(value.into()));
            }
            ("privilege_escalation", "become_method") if value == "sudo" => {
                self.defaults
                    .insert("ansible_become_method".into(), Value::String(value.into()));
            }
            _ => bail!("Unsupported configuration option [{section}].{key}"),
        }
        Ok(())
    }
    pub fn apply_inventory(&self, inventory: &mut Inventory) {
        inventory.defaults = Some(std::sync::Arc::new(self.defaults.clone()));
        inventory.apply_group_vars();
    }
    pub fn host_limit(&self, matches: &clap::ArgMatches) -> Result<usize> {
        let forks = if matches.value_source("forks") == Some(clap::parser::ValueSource::CommandLine)
        {
            matches
                .get_one::<usize>("forks")
                .copied()
                .context("Missing --forks")?
        } else {
            self.forks.unwrap_or(5)
        };
        if forks == 0 {
            bail!("--forks must be greater than zero");
        }
        Ok(forks)
    }
}
fn boolean(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("Configuration requires a boolean"),
    }
}
fn resolve_path(base: &Path, value: &str) -> Result<PathBuf> {
    let path = if let Some(rest) = value.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").context("Cannot expand configuration home path")?)
            .join(rest)
    } else {
        PathBuf::from(value)
    };
    Ok(base.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_types_paths_and_host_defaults_keep_inventory_precedence() {
        let source = Path::new("/project/ansible.cfg");
        let config = Config::parse("[defaults]\ninventory=hosts\nforks=3\nworker_dir=workers\nremote_user=default-user\nprivate_key_file=key\ntimeout=7\nhost_key_checking=yes\n[privilege_escalation]\nbecome=false\nbecome_user=app\nbecome_method=sudo\n", source).unwrap();
        assert_eq!(config.inventory, Some(PathBuf::from("/project/hosts")));
        assert_eq!(config.worker_dir, Some(PathBuf::from("/project/workers")));
        assert_eq!(config.forks, Some(3));
        let matches = crate::cli::build_cli()
            .try_get_matches_from(["rustsible", "ad-hoc", "all", "-m", "debug"])
            .unwrap();
        assert_eq!(
            config.host_limit(matches.subcommand().unwrap().1).unwrap(),
            3
        );
        let matches = crate::cli::build_cli()
            .try_get_matches_from(["rustsible", "ad-hoc", "all", "-m", "debug", "--forks", "7"])
            .unwrap();
        assert_eq!(
            config.host_limit(matches.subcommand().unwrap().1).unwrap(),
            7
        );
        let mut inventory = Inventory::new();
        let mut host = crate::inventory::Host::new("node");
        host.set_variable("ansible_user", "inventory-user");
        inventory.add_host(host);
        config.apply_inventory(&mut inventory);
        let variables = inventory.filter_hosts("all")[0].all_variables();
        assert_eq!(variables["ansible_user"].as_str(), Some("inventory-user"));
        assert_eq!(variables["ansible_become_user"].as_str(), Some("app"));
        assert_eq!(variables["ansible_become"].as_bool(), Some(false));
        assert_eq!(variables["ansible_ssh_timeout"].as_u64(), Some(7));
        let literal =
            Config::parse("[defaults]\nprivate_key_file={{ literal }}/key", source).unwrap();
        literal.apply_inventory(&mut inventory);
        let host = inventory.filter_hosts("all").remove(0);
        let effective = crate::ssh::config::ConnectionConfig::from_host(&host).unwrap();
        assert_eq!(
            effective.get_ssh_private_key(),
            Some("/project/{{ literal }}/key")
        );
    }
    #[test]
    fn unsupported_configuration_never_silently_changes_expectations() {
        for content in [
            "[defaults]\nforks=0",
            "[defaults]\ntimeout=0",
            "[defaults]\nhost_key_checking=maybe",
            "[defaults]\nunknown=x",
            "[ssh_connection]\nssh_args=-x",
            "[privilege_escalation]\nbecome_method=su",
            "[defaults]\ninventory=",
            "[defaults]\ninventory='",
            "[defaults]\ninventory=\"",
        ] {
            assert!(
                Config::parse(content, Path::new("/tmp/ansible.cfg")).is_err(),
                "{content}"
            );
        }
    }

    #[test]
    fn role_paths_keep_config_relative_search_order() {
        let config = Config::parse(
            "[defaults]\nroles_path=vendor/roles:/opt/roles",
            Path::new("/project/ansible.cfg"),
        )
        .unwrap();
        assert_eq!(
            config.role_paths,
            vec![
                PathBuf::from("/project/vendor/roles"),
                PathBuf::from("/opt/roles")
            ]
        );
    }
}
