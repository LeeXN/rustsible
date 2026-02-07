use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use serde_yaml::Value;

use crate::inventory::Host;
use crate::modules::ModuleResult;
use crate::ssh::connection::SshClient;

/// Package states supported by the module
#[derive(Debug, Clone, PartialEq)]
enum PackageState {
    Present,
    Absent,
    Latest,
}

impl PackageState {
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "present" | "installed" => Ok(PackageState::Present),
            "absent" | "removed" => Ok(PackageState::Absent),
            "latest" => Ok(PackageState::Latest),
            _ => Err(anyhow::anyhow!("Invalid package state: {}", s)),
        }
    }
}

/// Package manager types
#[derive(Debug, Clone, PartialEq)]
enum PackageManager {
    Apt,
    Yum,
    Dnf,
    Zypper,
    Pacman,
}

/// Execute package module with the given arguments
pub fn execute(
    ssh_client: &SshClient,
    args: &Value,
    use_become: bool,
    become_user: &str,
) -> Result<ModuleResult> {
    let map = match args {
        Value::Mapping(map) => map,
        _ => {
            return Err(anyhow::anyhow!(
                "Package module requires a mapping of arguments"
            ))
        }
    };

    // Get package name - support both string and sequence (list) formats
    let packages = match map.get(&Value::String("name".to_string())) {
        Some(Value::String(name)) => vec![name.clone()],
        Some(Value::Sequence(names)) => {
            let mut package_names = Vec::new();
            for name_value in names {
                if let Value::String(name) = name_value {
                    package_names.push(name.clone());
                } else {
                    return Err(anyhow::anyhow!("Package names in list must be strings"));
                }
            }
            if package_names.is_empty() {
                return Err(anyhow::anyhow!(
                    "Package module requires at least one package name"
                ));
            }
            package_names
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Package module requires a 'name' parameter (string or list)"
            ))
        }
    };

    // Get desired state
    let state_str = match map.get(&Value::String("state".to_string())) {
        Some(Value::String(state)) => state,
        _ => "present", // Default to present if not specified
    };

    let state = PackageState::from_str(state_str).context("Failed to parse package state")?;

    // Check if we should update package cache
    let update_cache = match map.get(&Value::String("update_cache".to_string())) {
        Some(Value::Bool(update)) => *update,
        _ => false, // Default to false if not specified
    };

    // Detect package manager
    let pkg_manager = detect_package_manager(ssh_client, use_become, become_user)?;
    info!("Detected package manager: {:?}", pkg_manager);

    // Update package cache if requested
    if update_cache {
        let update_cmd = match pkg_manager {
            PackageManager::Apt => "apt-get update",
            PackageManager::Yum => "yum check-update || true", // yum check-update returns 100 if updates are available
            PackageManager::Dnf => "dnf check-update || true",
            PackageManager::Zypper => "zypper refresh",
            PackageManager::Pacman => "pacman -Sy",
        };

        info!("Updating package cache with command: {}", update_cmd);

        let result = if use_become {
            ssh_client.execute_sudo_command(update_cmd, become_user)?
        } else {
            ssh_client.execute_sudo_command(update_cmd, "root")?
        };

        let (exit_code, _, stderr) = result;
        if !stderr.trim().is_empty() {
            warn!("Update cache stderr: {}", stderr);
        }

        // Special handling for yum/dnf check-update which returns 100 when updates are available
        if exit_code != 0 && exit_code != 100 {
            error!(
                "Failed to update package cache with exit code: {}",
                exit_code
            );
            return Err(anyhow::anyhow!(
                "Failed to update package cache with exit code: {}",
                exit_code
            ));
        }
    }

    // Process each package in the list
    for package_name in packages.clone() {
        info!("Processing package: {}", package_name);

        // Build the command based on the detected package manager
        let command = match (pkg_manager.clone(), &state) {
            (PackageManager::Apt, PackageState::Present) => {
                format!("apt-get -y install {}", package_name)
            }
            (PackageManager::Apt, PackageState::Absent) => {
                format!("apt-get -y remove {}", package_name)
            }
            (PackageManager::Apt, PackageState::Latest) => {
                format!("apt-get -y install --only-upgrade {}", package_name)
            }

            (PackageManager::Yum, PackageState::Present) => {
                format!("yum -y install {}", package_name)
            }
            (PackageManager::Yum, PackageState::Absent) => {
                format!("yum -y remove {}", package_name)
            }
            (PackageManager::Yum, PackageState::Latest) => {
                format!("yum -y update {}", package_name)
            }

            (PackageManager::Dnf, PackageState::Present) => {
                format!("dnf -y install {}", package_name)
            }
            (PackageManager::Dnf, PackageState::Absent) => {
                format!("dnf -y remove {}", package_name)
            }
            (PackageManager::Dnf, PackageState::Latest) => {
                format!("dnf -y update {}", package_name)
            }

            (PackageManager::Zypper, PackageState::Present) => {
                format!("zypper --non-interactive install {}", package_name)
            }
            (PackageManager::Zypper, PackageState::Absent) => {
                format!("zypper --non-interactive remove {}", package_name)
            }
            (PackageManager::Zypper, PackageState::Latest) => {
                format!("zypper --non-interactive update {}", package_name)
            }

            (PackageManager::Pacman, PackageState::Present) => {
                format!("pacman -S --noconfirm {}", package_name)
            }
            (PackageManager::Pacman, PackageState::Absent) => {
                format!("pacman -R --noconfirm {}", package_name)
            }
            (PackageManager::Pacman, PackageState::Latest) => {
                format!("pacman -Syu --noconfirm {}", package_name)
            }
        };

        info!("Executing package command: {}", command);

        // Check if package is already in desired state
        if should_skip_operation(
            ssh_client,
            &package_name,
            &state,
            pkg_manager.clone(),
            use_become,
            become_user,
        )? {
            info!(
                "Package '{}' is already in desired state '{}', skipping",
                package_name, state_str
            );
            continue;
        }

        // Run the command with privilege escalation (always needed for package operations)
        let result = if use_become {
            ssh_client.execute_sudo_command(&command, become_user)?
        } else {
            // Force become for package operations
            ssh_client.execute_sudo_command(&command, "root")?
        };

        let (exit_code, stdout, stderr) = result;

        if !stdout.trim().is_empty() {
            debug!("Command stdout: {}", stdout);
        }

        if !stderr.trim().is_empty() {
            warn!("Command stderr: {}", stderr);
        }

        if exit_code != 0 {
            error!("Package command failed with exit code: {}", exit_code);
            return Err(anyhow::anyhow!(
                "Package command failed for '{}' with exit code: {}",
                package_name,
                exit_code
            ));
        }

        info!("Package '{}' is now in state '{}'", package_name, state_str);
    }
    let state_str = match state {
        PackageState::Present => "installed",
        PackageState::Absent => "removed",
        PackageState::Latest => "updated",
    };

    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        failed: false,
        msg: format!(
            "Package(s) {} state changed to {}",
            packages.join(", "),
            state_str
        ),
    })
}

/// Execute package module in ad-hoc mode
pub fn execute_adhoc(host: &Host, package_args: &Value) -> Result<ModuleResult> {
    info!("Connecting to host: {}", host.name);
    let ssh_client = SshClient::connect(host)?;

    // 执行包操作
    execute(&ssh_client, package_args, false, "")?;

    // 获取包名和状态用于输出消息
    let map = match package_args {
        Value::Mapping(map) => map,
        _ => {
            return Err(anyhow::anyhow!(
                "Package module requires a mapping of arguments"
            ))
        }
    };

    // 尝试构建信息消息，包括包名和状态
    let package_info = match map.get(&Value::String("name".to_string())) {
        Some(Value::String(name)) => name.clone(),
        Some(Value::Sequence(names)) => {
            let mut package_names = Vec::new();
            for name_value in names {
                if let Value::String(name) = name_value {
                    package_names.push(name.clone());
                }
            }
            package_names.join(", ")
        }
        _ => "packages".to_string(),
    };

    let state = match map.get(&Value::String("state".to_string())) {
        Some(Value::String(state)) => state.clone(),
        _ => "present".to_string(),
    };

    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        changed: true,
        failed: false,
        msg: format!("Package(s) {} state changed to {}", package_info, state),
    })
}

/// Detect the package manager used by the remote host
fn detect_package_manager(
    ssh_client: &SshClient,
    use_become: bool,
    become_user: &str,
) -> Result<PackageManager> {
    // Check for common package managers in order of preference
    let checks = vec![
        ("which apt-get", PackageManager::Apt),
        ("which dnf", PackageManager::Dnf),
        ("which yum", PackageManager::Yum),
        ("which zypper", PackageManager::Zypper),
        ("which pacman", PackageManager::Pacman),
    ];

    for (cmd, manager) in checks {
        let result = if use_become {
            ssh_client.execute_sudo_command(cmd, become_user)
        } else {
            ssh_client.execute_command(cmd)
        };

        if let Ok((exit_code, _, _)) = result {
            if exit_code == 0 {
                return Ok(manager);
            }
        }
    }

    // Default to apt if nothing else is found
    warn!("No package manager detected, defaulting to apt");
    Ok(PackageManager::Apt)
}

/// Check if the package is already in the desired state
fn should_skip_operation(
    ssh_client: &SshClient,
    package_name: &str,
    desired_state: &PackageState,
    pkg_manager: PackageManager,
    use_become: bool,
    become_user: &str,
) -> Result<bool> {
    // Build command to check if package is installed
    let check_cmd = match pkg_manager {
        PackageManager::Apt => format!(
            "dpkg-query -W -f='${{Status}}' {} 2>/dev/null | grep -q 'ok installed'",
            package_name
        ),
        PackageManager::Yum | PackageManager::Dnf => {
            format!("rpm -q {} >/dev/null 2>&1", package_name)
        }
        PackageManager::Zypper => format!("rpm -q {} >/dev/null 2>&1", package_name),
        PackageManager::Pacman => format!("pacman -Q {} >/dev/null 2>&1", package_name),
    };

    let result = if use_become {
        ssh_client.execute_sudo_command(&check_cmd, become_user)
    } else {
        ssh_client.execute_command(&check_cmd)
    };

    let is_installed = match result {
        Ok((exit_code, _, _)) => exit_code == 0,
        Err(_) => false,
    };

    // Compare current state with desired state
    match desired_state {
        PackageState::Present => {
            // If we want it present and it's already installed, skip
            Ok(is_installed)
        }
        PackageState::Absent => {
            // If we want it absent and it's not installed, skip
            Ok(!is_installed)
        }
        PackageState::Latest => {
            // For latest, we need more checks
            if !is_installed {
                // If not installed at all, don't skip
                return Ok(false);
            }

            // Check if package is already at latest version
            // This varies by package manager, and requires more complex logic
            // For simplicity, we'll always update when "latest" is requested
            Ok(false)
        }
    }
}
