use crate::tools;
use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    change, param::get_optional_param as optional, registry::Builtin, ModuleResult,
};
use serde_yaml::Value;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Manager {
    Apt,
    Dnf,
    Yum,
    Zypper,
    Pacman,
    Apk,
}
impl Manager {
    fn program(self) -> &'static str {
        match self {
            Self::Apt => "apt-get",
            Self::Dnf => "dnf",
            Self::Yum => "yum",
            Self::Zypper => "zypper",
            Self::Pacman => "pacman",
            Self::Apk => "apk",
        }
    }
}
fn detect() -> Result<Manager> {
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let ids = release
        .lines()
        .filter_map(|line| {
            line.strip_prefix("ID=")
                .or_else(|| line.strip_prefix("ID_LIKE="))
        })
        .map(|value| value.trim_matches('"'))
        .collect::<Vec<_>>()
        .join(" ");
    let preferred = if ids.contains("alpine") {
        Some(Manager::Apk)
    } else if ids.contains("arch") {
        Some(Manager::Pacman)
    } else if ids.contains("suse") {
        Some(Manager::Zypper)
    } else if ids.contains("fedora") || ids.contains("rhel") {
        Some(Manager::Dnf)
    } else if ids.contains("debian") || ids.contains("ubuntu") {
        Some(Manager::Apt)
    } else {
        None
    };
    if let Some(manager) = preferred {
        if tools::available(manager.program()) {
            return Ok(manager);
        }
    }
    [
        Manager::Apt,
        Manager::Dnf,
        Manager::Yum,
        Manager::Zypper,
        Manager::Pacman,
        Manager::Apk,
    ]
    .into_iter()
    .find(|manager| tools::available(manager.program()))
    .context("No supported package manager is available")
}
fn names(args: &Value) -> Result<Vec<String>> {
    let values = match args.get("name") {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Sequence(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .context("Package names must be strings")
                    .map(str::to_owned)
            })
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("Package requires a name or list"),
    };
    for value in &values {
        if value.is_empty()
            || value.starts_with('-')
            || value.chars().any(char::is_control)
            || value.contains(['*', '?', '[', ']'])
        {
            bail!("Package names must be concrete nonempty names/specifications without options or control characters");
        }
    }
    if values.is_empty() {
        bail!("Package list must not be empty");
    }
    Ok(values)
}
fn base(manager: Manager, name: &str) -> &str {
    if manager == Manager::Apt {
        name.split_once('=').map_or(name, |(name, _)| name)
    } else {
        name
    }
}
fn installed(manager: Manager, name: &str) -> Result<Option<String>> {
    let name = base(manager, name);
    let response = match manager {
        Manager::Apt => tools::query(
            "dpkg-query",
            &["-W", "-f=${db:Status-Status}\t${Version}", "--", name],
        )?,
        Manager::Dnf | Manager::Yum | Manager::Zypper => {
            tools::query("rpm", &["-q", "--qf", "%{VERSION}-%{RELEASE}", "--", name])?
        }
        Manager::Pacman => tools::query("pacman", &["-Q", "--", name])?,
        Manager::Apk => {
            let exists = tools::query("apk", &["info", "-e", "--", name])?;
            if exists.rc == Some(1) {
                return Ok(None);
            }
            tools::require(exists, "Read installed APK package")?;
            tools::query("apk", &["list", "--installed", "--", name])?
        }
    };
    match response.rc {
        Some(1) => Ok(None),
        Some(0) => {
            let output = response.stdout.trim();
            if manager == Manager::Apt {
                let (state, version) = output
                    .split_once('\t')
                    .context("Invalid dpkg status response")?;
                return Ok((state == "installed").then(|| version.to_owned()));
            }
            let version = match manager {
                Manager::Pacman => output
                    .split_whitespace()
                    .nth(1)
                    .context("Invalid pacman version response")?,
                Manager::Apk => output
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .strip_prefix(&format!("{name}-"))
                    .context("Invalid APK version response")?,
                _ => output,
            };
            if version.is_empty() {
                bail!("Package database returned an empty version");
            }
            Ok(Some(version.into()))
        }
        _ => Err(tools::failed(response, "Query installed package")),
    }
}
fn latest(manager: Manager, name: &str, current: &str) -> Result<bool> {
    let response = match manager {
        Manager::Apt => {
            if let Some((_, version)) = name.split_once('=') {
                return Ok(current == version);
            }
            let response = tools::require(
                tools::query("apt-cache", &["policy", "--", name])?,
                "Read APT candidate",
            )?;
            let candidate = response
                .stdout
                .lines()
                .find_map(|line| line.trim().strip_prefix("Candidate:"))
                .map(str::trim)
                .filter(|version| !version.is_empty() && *version != "(none)")
                .context("No APT candidate is available in the current cache")?;
            return Ok(current == candidate);
        }
        Manager::Dnf | Manager::Yum => tools::query(
            manager.program(),
            &["-q", "--cacheonly", "check-update", "--", name],
        )?,
        Manager::Pacman => tools::query("pacman", &["-Qu", "--", name])?,
        Manager::Apk => {
            let response = tools::require(
                tools::query("apk", &["version", "-l", "<", "--", name])?,
                "Compare cached APK candidate",
            )?;
            if !response.stderr.trim().is_empty() {
                bail!("APK candidate cache is unavailable; refresh it before a read-only latest prediction");
            }
            return Ok(response.stdout.trim().is_empty());
        }
        Manager::Zypper => {
            bail!("state=latest is not supported for zypper: no stable candidate query is declared")
        }
    };
    match (manager, response.rc) {
        (Manager::Dnf | Manager::Yum, Some(0)) => Ok(true),
        (Manager::Dnf | Manager::Yum, Some(100)) => Ok(false),
        (Manager::Pacman, Some(1)) if response.stderr.trim().is_empty() => Ok(true),
        (Manager::Pacman, Some(0)) => Ok(false),
        _ => Err(tools::failed(response, "Compare package candidate")),
    }
}
fn refresh(manager: Manager) -> Result<()> {
    let argv: &[&str] = match manager {
        Manager::Apt => &["-q", "update"],
        Manager::Dnf | Manager::Yum => &["-q", "makecache"],
        Manager::Pacman => &["-Sy", "--noconfirm"],
        Manager::Zypper => &["--non-interactive", "refresh"],
        Manager::Apk => &["update"],
    };
    tools::mutate(manager.program(), argv, "Refresh package cache")?;
    Ok(())
}
pub fn execute(args: &Value, check: bool) -> Result<ModuleResult> {
    Builtin::Package.validate_parameters(args)?;
    let names = names(args)?;
    let state = optional::<String>(args, "state")?
        .unwrap_or_else(|| "present".into())
        .to_ascii_lowercase();
    let update = optional::<bool>(args, "update_cache")?.unwrap_or(false);
    let manager = detect()?;
    if manager == Manager::Zypper && state == "latest" {
        bail!("state=latest is not supported for zypper");
    }
    if manager == Manager::Apt
        && names.iter().any(|name| {
            name.split_once('=')
                .is_some_and(|(name, version)| name.is_empty() || version.is_empty())
        })
    {
        bail!("Invalid APT version specification");
    }
    let before = names
        .iter()
        .map(|name| installed(manager, name))
        .collect::<Result<Vec<_>>>()?;
    // Cache refresh is intentionally before candidate queries. Check only uses
    // the existing cache and never runs an update/check-update network refresh.
    if update && !check {
        refresh(manager)?;
    }
    let mut needed = Vec::new();
    for (name, current) in names.iter().zip(&before) {
        let change = match state.as_str() {
            "absent" => current.is_some(),
            "present" => {
                current.is_none()
                    || (manager == Manager::Apt
                        && name
                            .split_once('=')
                            .is_some_and(|(_, version)| current.as_deref() != Some(version)))
            }
            "latest" => match current {
                None => true,
                Some(version) => !latest(manager, name, version)?,
            },
            _ => bail!("Invalid package state"),
        };
        if change {
            needed.push(if state == "absent" {
                base(manager, name).to_owned()
            } else {
                name.clone()
            });
        }
    }
    let changed = update || !needed.is_empty();
    if !check && !needed.is_empty() {
        let prefix: &[&str] = match (manager, state.as_str()) {
            (Manager::Apt, "absent") => &["-y", "-q", "remove", "--"],
            (Manager::Apt, _) => &["-y", "-q", "install", "--"],
            (Manager::Dnf | Manager::Yum, "absent") => &["-y", "remove", "--"],
            (Manager::Dnf | Manager::Yum, "latest") => &["-y", "upgrade", "--"],
            (Manager::Dnf | Manager::Yum, _) => &["-y", "install", "--"],
            (Manager::Pacman, "absent") => &["-R", "--noconfirm", "--"],
            (Manager::Pacman, _) => &["-S", "--noconfirm", "--needed", "--"],
            (Manager::Zypper, "absent") => &["--non-interactive", "remove", "--"],
            (Manager::Zypper, _) => &["--non-interactive", "install", "--"],
            (Manager::Apk, "absent") => &["del", "--"],
            (Manager::Apk, "latest") => &["add", "--upgrade", "--"],
            (Manager::Apk, _) => &["add", "--"],
        };
        let mut argv = prefix
            .iter()
            .map(|argument| argument.to_string())
            .collect::<Vec<_>>();
        argv.extend(needed);
        let response = tools::run(manager.program(), &argv, None, true)?;
        if response.failed {
            let after = names
                .iter()
                .map(|name| installed(manager, name))
                .collect::<Result<Vec<_>>>();
            if after.as_ref().is_ok_and(|after| after != &before) {
                change::record_change();
            }
            let mut response = response;
            if after.is_err() {
                response.uncertain = true;
            }
            return Err(tools::failed(
                response,
                "Package transaction; partial changes may remain",
            ));
        }
        change::record_change();
        let after = names
            .iter()
            .map(|name| installed(manager, name))
            .collect::<Result<Vec<_>>>()?;
        if names.iter().zip(&after).any(|(name, version)| {
            if state == "absent" {
                version.is_some()
            } else {
                version.is_none()
                    || (manager == Manager::Apt
                        && name
                            .split_once('=')
                            .is_some_and(|(_, wanted)| version.as_deref() != Some(wanted)))
            }
        }) {
            bail!("Package transaction exited successfully but requested installed state was not observed");
        }
        if state == "latest" {
            for (name, version) in names.iter().zip(&after) {
                if !latest(
                    manager,
                    name,
                    version
                        .as_deref()
                        .context("Missing installed package after update")?,
                )? {
                    bail!("Package manager exited successfully but the requested latest version was not observed");
                }
            }
        }
    }
    Ok(ModuleResult {
        changed,
        msg: if check && update {
            "Package prediction used the existing cache; refresh was not performed"
        } else {
            "Package state is current or updated"
        }
        .into(),
        ..ModuleResult::default()
    })
}
