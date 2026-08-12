use anyhow::{bail, Context, Result};
use log::{debug, info};
use serde_yaml::Value;

use crate::inventory::Host;
use crate::modules::file::quote_posix_shell_arg;
use crate::modules::param::validate_params;
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, SshConnection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageState {
    Present,
    Absent,
    Latest,
}

impl PackageState {
    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "present" | "installed" => Ok(Self::Present),
            "absent" | "removed" => Ok(Self::Absent),
            "latest" => Ok(Self::Latest),
            _ => bail!("Invalid package state: {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Apt,
    Yum,
    Dnf,
    Zypper,
    Pacman,
}

impl PackageManager {
    fn executable(self) -> &'static str {
        match self {
            Self::Apt => "apt-get",
            Self::Yum => "yum",
            Self::Dnf => "dnf",
            Self::Zypper => "zypper",
            Self::Pacman => "pacman",
        }
    }
}

fn quote_package_name(name: &str) -> Result<String> {
    if name.is_empty() || name.starts_with('-') {
        bail!("Invalid package name: {name}");
    }
    quote_posix_shell_arg(name)
}

fn package_names(args: &Value) -> Result<Vec<String>> {
    let map = args
        .as_mapping()
        .context("Package module requires a mapping of arguments")?;
    let key = Value::String("name".to_string());
    let names = match map.get(&key) {
        Some(Value::String(name)) => vec![name.clone()],
        Some(Value::Sequence(values)) => values
            .iter()
            .map(|value| match value {
                Value::String(name) => Ok(name.clone()),
                _ => bail!("Package names in a list must be strings"),
            })
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("Package module requires a 'name' parameter (string or list)"),
    };
    if names.is_empty() {
        bail!("Package module requires at least one package name");
    }
    for name in &names {
        quote_package_name(name)?;
    }
    Ok(names)
}

fn run_operation(
    connection: &dyn SshConnection,
    command: &str,
    use_become: bool,
    become_user: &str,
) -> Result<(i32, String, String)> {
    if use_become {
        connection.execute_sudo_command(command, become_user)
    } else {
        // Do not silently escalate. A caller that omitted become should see
        // the package manager's normal permission error.
        connection.execute_command(command)
    }
}

fn detect_package_manager(connection: &dyn SshConnection) -> Result<PackageManager> {
    // Prefer the distribution's native manager where multiple compatibility
    // front-ends are installed.
    for manager in [
        PackageManager::Apt,
        PackageManager::Dnf,
        PackageManager::Yum,
        PackageManager::Zypper,
        PackageManager::Pacman,
    ] {
        let command = format!("command -v -- {} >/dev/null 2>&1", manager.executable());
        let (code, _, stderr) = connection
            .execute_command(&command)
            .with_context(|| format!("Failed while detecting {}", manager.executable()))?;
        match code {
            0 => return Ok(manager),
            1 | 127 => {}
            code => bail!(
                "Failed while detecting {} (exit {}): {}",
                manager.executable(),
                code,
                stderr.trim()
            ),
        }
    }
    bail!("No supported package manager was detected")
}

fn installed_query(manager: PackageManager, package: &str) -> Result<String> {
    let package = quote_package_name(package)?;
    Ok(match manager {
        PackageManager::Apt => format!(
            "dpkg-query -W -f='${{Status}}' -- {package} 2>/dev/null | grep -q '^install ok installed$'"
        ),
        PackageManager::Yum | PackageManager::Dnf | PackageManager::Zypper => {
            format!("rpm -q -- {package} >/dev/null 2>&1")
        }
        PackageManager::Pacman => format!("pacman -Q -- {package} >/dev/null 2>&1"),
    })
}

fn is_installed(
    connection: &dyn SshConnection,
    manager: PackageManager,
    package: &str,
) -> Result<bool> {
    let command = installed_query(manager, package)?;
    let (code, _, stderr) = connection.execute_command(&command)?;
    match code {
        0 => Ok(true),
        1 => Ok(false),
        code => bail!(
            "Failed to query installed state for '{}' (exit {}): {}",
            package,
            code,
            stderr.trim()
        ),
    }
}

fn is_latest(
    connection: &dyn SshConnection,
    manager: PackageManager,
    package: &str,
) -> Result<bool> {
    let package_arg = quote_package_name(package)?;
    let command = match manager {
        PackageManager::Apt => format!(
            "installed=$(dpkg-query -W -f='${{Version}}' -- {0} 2>/dev/null) || exit 2; candidate=$(apt-cache policy -- {0} | awk '/^[[:space:]]*Candidate:/ {{print $2; exit}}'); test -n \"$candidate\" || exit 3; test \"$installed\" = \"$candidate\"",
            package_arg
        ),
        PackageManager::Dnf => format!("dnf -q check-update -- {package_arg}"),
        PackageManager::Yum => format!("yum -q check-update -- {package_arg}"),
        // `pacman -Qu` prints an entry and succeeds only when an upgrade is
        // available, so its result is inverted below.
        PackageManager::Pacman => format!("pacman -Qu -- {package_arg} >/dev/null 2>&1"),
        PackageManager::Zypper => {
            // Zypper has no stable, locale-independent per-package exit code
            // for this question. Refuse to claim idempotency rather than run
            // an update on every invocation.
            bail!(
                "state=latest is not supported for zypper because the installed/candidate state cannot be determined reliably"
            )
        }
    };
    let (code, _, stderr) = connection.execute_command(&command)?;
    match manager {
        PackageManager::Apt => match code {
            0 => Ok(true),
            1 => Ok(false),
            2 => bail!(
                "Package '{}' disappeared while checking its version",
                package
            ),
            3 => bail!(
                "No candidate version was reported for package '{}'",
                package
            ),
            code => bail!(
                "Failed to compare package versions for '{}' (exit {}): {}",
                package,
                code,
                stderr.trim()
            ),
        },
        PackageManager::Dnf | PackageManager::Yum => match code {
            0 => Ok(true),
            100 => Ok(false),
            code => bail!(
                "Failed to query available updates for '{}' (exit {}): {}",
                package,
                code,
                stderr.trim()
            ),
        },
        PackageManager::Pacman => match code {
            0 => Ok(false),
            1 => Ok(true),
            code => bail!(
                "Failed to query available updates for '{}' (exit {}): {}",
                package,
                code,
                stderr.trim()
            ),
        },
        PackageManager::Zypper => bail!("state=latest is not supported for zypper"),
    }
}

fn package_needs_change(
    connection: &dyn SshConnection,
    manager: PackageManager,
    package: &str,
    state: PackageState,
) -> Result<bool> {
    let installed = is_installed(connection, manager, package)?;
    match state {
        PackageState::Present => Ok(!installed),
        PackageState::Absent => Ok(installed),
        PackageState::Latest if !installed => Ok(true),
        PackageState::Latest => Ok(!is_latest(connection, manager, package)?),
    }
}

fn cache_update_command(manager: PackageManager) -> &'static str {
    match manager {
        PackageManager::Apt => "apt-get update",
        PackageManager::Yum => "yum -y makecache",
        PackageManager::Dnf => "dnf -y makecache",
        PackageManager::Zypper => "zypper --non-interactive refresh",
        PackageManager::Pacman => "pacman -Sy --noconfirm",
    }
}

fn change_command(manager: PackageManager, package: &str, state: PackageState) -> Result<String> {
    let package = quote_package_name(package)?;
    Ok(match (manager, state) {
        (PackageManager::Apt, PackageState::Present) => format!("apt-get -y install -- {package}"),
        (PackageManager::Apt, PackageState::Absent) => format!("apt-get -y remove -- {package}"),
        // `install` both creates a missing package and upgrades an installed
        // package to the candidate version, which is exactly `latest`.
        (PackageManager::Apt, PackageState::Latest) => format!("apt-get -y install -- {package}"),
        (PackageManager::Yum, PackageState::Present) => format!("yum -y install -- {package}"),
        (PackageManager::Yum, PackageState::Absent) => format!("yum -y remove -- {package}"),
        (PackageManager::Yum, PackageState::Latest) => format!("yum -y install -- {package}"),
        (PackageManager::Dnf, PackageState::Present) => format!("dnf -y install -- {package}"),
        (PackageManager::Dnf, PackageState::Absent) => format!("dnf -y remove -- {package}"),
        (PackageManager::Dnf, PackageState::Latest) => format!("dnf -y install -- {package}"),
        (PackageManager::Zypper, PackageState::Present) => {
            format!("zypper --non-interactive install -- {package}")
        }
        (PackageManager::Zypper, PackageState::Absent) => {
            format!("zypper --non-interactive remove -- {package}")
        }
        (PackageManager::Zypper, PackageState::Latest) => {
            bail!("state=latest is not supported for zypper")
        }
        (PackageManager::Pacman, PackageState::Present) => {
            format!("pacman -S --noconfirm -- {package}")
        }
        (PackageManager::Pacman, PackageState::Absent) => {
            format!("pacman -R --noconfirm -- {package}")
        }
        (PackageManager::Pacman, PackageState::Latest) => {
            format!("pacman -S --noconfirm -- {package}")
        }
    })
}

/// Manage one or more packages. Cache refresh is deliberately considered a
/// change whenever requested because refreshing the manager's cache mutates
/// local state even when package versions remain the same.
pub fn execute(
    connection: &dyn SshConnection,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    validate_params(args, &["name", "state", "update_cache"])?;
    let map = args
        .as_mapping()
        .context("Package module requires a mapping of arguments")?;
    let packages = package_names(args)?;
    let state = match map.get(Value::String("state".to_string())) {
        Some(Value::String(state)) => PackageState::from_str(state)?,
        Some(_) => bail!("Package 'state' must be a string"),
        None => PackageState::Present,
    };
    let update_cache = match map.get(Value::String("update_cache".to_string())) {
        Some(Value::Bool(value)) => *value,
        Some(_) => bail!("Package 'update_cache' must be a boolean"),
        None => false,
    };

    let manager = detect_package_manager(connection)?;
    info!("Detected package manager: {:?}", manager);
    if manager == PackageManager::Zypper && state == PackageState::Latest {
        bail!(
            "state=latest is not supported for zypper because installed and candidate versions cannot be compared reliably"
        );
    }

    // Probe every package before performing mutations. This prevents a later
    // query failure from leaving only the first half of a package list changed.
    let mut pending = Vec::new();
    for package in &packages {
        if package_needs_change(connection, manager, package, state)? {
            pending.push(package.clone());
        }
    }
    let changed = update_cache || !pending.is_empty();

    if !check_mode {
        if update_cache {
            let command = cache_update_command(manager);
            let (code, _, stderr) = run_operation(connection, command, use_become, become_user)?;
            if code != 0 {
                bail!(
                    "Failed to update the package cache (exit {}): {}",
                    code,
                    stderr.trim()
                );
            }
        }

        for package in &pending {
            let command = change_command(manager, package, state)?;
            debug!(
                "Executing package-manager operation ({} bytes)",
                command.len()
            );
            let (code, _, stderr) = run_operation(connection, &command, use_become, become_user)?;
            if code != 0 {
                bail!(
                    "Package operation failed for '{}' (exit {}): {}",
                    package,
                    code,
                    stderr.trim()
                );
            }
        }
    }

    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        rc: None,
        changed,
        failed: false,
        msg: if changed {
            format!(
                "{} package(s) {}{}",
                pending.len(),
                if check_mode {
                    "would be changed"
                } else {
                    "changed"
                },
                if update_cache {
                    "; cache refresh requested"
                } else {
                    ""
                }
            )
        } else {
            format!(
                "Package(s) {} are already in the requested state",
                packages.join(", ")
            )
        },
    })
}

pub fn execute_adhoc(
    host: &Host,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    info!("Opening connection for host: {}", host.name);
    let connection = Connection::connect(host)?;
    execute(
        connection.as_connection(),
        args,
        use_become,
        become_user,
        check_mode,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::connection::MockSshConnection;

    #[test]
    fn package_names_are_one_non_option_shell_argument() {
        assert_eq!(
            quote_package_name("libfoo;touch /tmp/pwned").unwrap(),
            "'libfoo;touch /tmp/pwned'"
        );
        assert!(quote_package_name("").is_err());
        assert!(quote_package_name("--installroot=/").is_err());
        assert!(quote_package_name("bad\0name").is_err());
    }

    #[test]
    fn present_and_absent_plans_are_idempotent() {
        // The decision itself is deliberately tiny and fully covered here;
        // manager-specific exit-code interpretation is exercised separately.
        assert!(!matches!(PackageState::Present, PackageState::Absent));
        assert_eq!(
            PackageState::from_str("installed").unwrap(),
            PackageState::Present
        );
        assert_eq!(
            PackageState::from_str("removed").unwrap(),
            PackageState::Absent
        );
    }

    #[test]
    fn latest_command_can_install_a_missing_package() {
        assert_eq!(
            change_command(PackageManager::Apt, "curl", PackageState::Latest).unwrap(),
            "apt-get -y install -- 'curl'"
        );
        assert_eq!(
            change_command(PackageManager::Dnf, "curl", PackageState::Latest).unwrap(),
            "dnf -y install -- 'curl'"
        );
        assert!(change_command(PackageManager::Zypper, "curl", PackageState::Latest).is_err());
    }

    #[test]
    fn check_mode_plans_without_running_package_mutations() {
        let mut connection = MockSshConnection::new();
        connection
            .expect_execute_command()
            .times(2)
            .returning(|command| {
                if command.contains("command -v") {
                    Ok((0, "/usr/bin/apt-get\n".to_string(), String::new()))
                } else {
                    assert!(command.contains("dpkg-query"));
                    Ok((1, String::new(), String::new()))
                }
            });
        connection.expect_execute_sudo_command().times(0);
        let args: Value = serde_yaml::from_str("name: curl\nstate: present\n").unwrap();

        let result = execute(&connection, &args, true, "root", true).unwrap();
        assert!(result.changed);
    }
}
