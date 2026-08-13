use anyhow::{bail, Context, Result};
use log::info;
use serde_yaml::Value;

use crate::inventory::Host;
use crate::modules::file::quote_posix_shell_arg;
use crate::modules::param::validate_params;
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, SshConnection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceState {
    Started,
    Stopped,
    Restarted,
    Reloaded,
}

impl ServiceState {
    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "started" => Ok(Self::Started),
            "stopped" => Ok(Self::Stopped),
            "restarted" => Ok(Self::Restarted),
            "reloaded" => Ok(Self::Reloaded),
            _ => bail!("Invalid service state: {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitSystem {
    Systemd,
    SysV,
    Upstart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ServicePlan {
    state_change: bool,
    enable_change: bool,
}

impl ServicePlan {
    fn changed(self) -> bool {
        self.state_change || self.enable_change
    }
}

fn run(
    connection: &dyn SshConnection,
    command: &str,
    use_become: bool,
    become_user: &str,
) -> Result<(i32, String, String)> {
    if use_become {
        connection.execute_sudo_command(command, become_user)
    } else {
        connection.execute_command(command)
    }
}

fn detect_init_system(connection: &dyn SshConnection) -> Result<InitSystem> {
    let checks = [
        (
            "test -d /run/systemd/system && command -v -- systemctl >/dev/null 2>&1",
            InitSystem::Systemd,
        ),
        ("command -v -- initctl >/dev/null 2>&1", InitSystem::Upstart),
        ("command -v -- service >/dev/null 2>&1", InitSystem::SysV),
    ];
    for (command, system) in checks {
        let (code, _, stderr) = connection
            .execute_command(command)
            .context("Failed while detecting the init system")?;
        match code {
            0 => return Ok(system),
            1 | 127 => {}
            code => bail!(
                "Failed while detecting the init system (exit {}): {}",
                code,
                stderr.trim()
            ),
        }
    }
    bail!("No supported init system was detected")
}

fn active_state(
    connection: &dyn SshConnection,
    init: InitSystem,
    service: &str,
    use_become: bool,
    become_user: &str,
) -> Result<bool> {
    let service = quote_posix_shell_arg(service)?;
    let command = match init {
        InitSystem::Systemd => format!("systemctl is-active --quiet -- {service}"),
        InitSystem::SysV => format!("service {service} status >/dev/null 2>&1"),
        InitSystem::Upstart => format!("initctl status {service}"),
    };
    let (code, stdout, stderr) = run(connection, &command, use_become, become_user)?;
    match init {
        InitSystem::Systemd => match code {
            0 => Ok(true),
            3 => Ok(false),
            code => bail!(
                "Failed to query service state (exit {}): {}",
                code,
                stderr.trim()
            ),
        },
        InitSystem::SysV => match code {
            0 => Ok(true),
            1 | 3 => Ok(false),
            code => bail!(
                "Failed to query service state (exit {}): {}",
                code,
                stderr.trim()
            ),
        },
        InitSystem::Upstart => {
            if code != 0 {
                bail!(
                    "Failed to query service state (exit {}): {}",
                    code,
                    stderr.trim()
                );
            }
            if stdout.contains("start/running") {
                Ok(true)
            } else if stdout.contains("stop/waiting") {
                Ok(false)
            } else {
                bail!("Unrecognized upstart status response")
            }
        }
    }
}

fn enabled_state(
    connection: &dyn SshConnection,
    init: InitSystem,
    service: &str,
    use_become: bool,
    become_user: &str,
) -> Result<bool> {
    if init != InitSystem::Systemd {
        bail!("The 'enabled' parameter is supported only for systemd services");
    }
    let service = quote_posix_shell_arg(service)?;
    let command = format!("systemctl is-enabled -- {service}");
    let (code, stdout, stderr) = run(connection, &command, use_become, become_user)?;
    let state = stdout.trim();
    match (code, state) {
        (0, "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias") => Ok(true),
        (0, "static" | "indirect" | "generated" | "transient") => Ok(false),
        (1, "disabled" | "masked" | "masked-runtime" | "static" | "indirect") => Ok(false),
        _ => bail!(
            "Failed to query whether the service is enabled (exit {}): {}",
            code,
            if stderr.trim().is_empty() {
                state
            } else {
                stderr.trim()
            }
        ),
    }
}

fn plan_service(
    state: Option<ServiceState>,
    active: Option<bool>,
    enable_change: bool,
) -> Result<ServicePlan> {
    let state_change = match state {
        Some(ServiceState::Started) => {
            !active.context("Internal service plan is missing the active state")?
        }
        Some(ServiceState::Stopped) => {
            active.context("Internal service plan is missing the active state")?
        }
        Some(ServiceState::Restarted | ServiceState::Reloaded) => true,
        None => false,
    };
    Ok(ServicePlan {
        state_change,
        enable_change,
    })
}

fn state_command(init: InitSystem, service: &str, state: ServiceState) -> Result<String> {
    let service = quote_posix_shell_arg(service)?;
    let verb = match state {
        ServiceState::Started => "start",
        ServiceState::Stopped => "stop",
        ServiceState::Restarted => "restart",
        ServiceState::Reloaded => "reload",
    };
    Ok(match init {
        InitSystem::Systemd => format!("systemctl {verb} -- {service}"),
        InitSystem::SysV => format!("service {service} {verb}"),
        InitSystem::Upstart => format!("initctl {verb} {service}"),
    })
}

pub fn execute(
    connection: &dyn SshConnection,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    validate_params(args, &["name", "state", "enabled"])?;
    let map = args
        .as_mapping()
        .context("Service module requires a mapping of arguments")?;
    let name = match map.get(Value::String("name".to_string())) {
        Some(Value::String(value)) if !value.is_empty() && !value.starts_with('-') => value,
        Some(Value::String(_)) => bail!("Service name cannot be empty or begin with '-'"),
        _ => bail!("Service module requires a string 'name' parameter"),
    };
    let state = match map.get(Value::String("state".to_string())) {
        Some(Value::String(value)) => Some(ServiceState::from_str(value)?),
        Some(_) => bail!("Service 'state' must be a string"),
        None => None,
    };
    let enabled = match map.get(Value::String("enabled".to_string())) {
        Some(Value::Bool(value)) => Some(*value),
        Some(_) => bail!("Service 'enabled' must be a boolean"),
        None => None,
    };
    if state.is_none() && enabled.is_none() {
        bail!("Service module requires at least one of 'state' or 'enabled'");
    }

    let init = detect_init_system(connection)?;
    info!("Detected init system: {:?}", init);
    if enabled.is_some() && init != InitSystem::Systemd {
        bail!("The 'enabled' parameter is supported only for systemd services");
    }

    let active = state
        .map(|_| active_state(connection, init, name, use_become, become_user))
        .transpose()?;
    let enable_change = match enabled {
        Some(desired) => enabled_state(connection, init, name, use_become, become_user)? != desired,
        None => false,
    };
    let plan = plan_service(state, active, enable_change)?;

    if !check_mode {
        if enable_change {
            let desired = enabled.context("Internal service enablement plan error")?;
            let name_arg = quote_posix_shell_arg(name)?;
            let command = format!(
                "systemctl {} -- {}",
                if desired { "enable" } else { "disable" },
                name_arg
            );
            let (code, _, stderr) = run(connection, &command, use_become, become_user)?;
            if code != 0 {
                bail!("Failed to change service enablement: {}", stderr.trim());
            }
        }
        if plan.state_change {
            let command = state_command(
                init,
                name,
                state.context("Internal service state plan error")?,
            )?;
            let (code, _, stderr) = run(connection, &command, use_become, become_user)?;
            if code != 0 {
                bail!("Failed to change service state: {}", stderr.trim());
            }
        }
    }

    Ok(ModuleResult {
        stdout: String::new(),
        stderr: String::new(),
        rc: None,
        changed: plan.changed(),
        failed: false,
        msg: if plan.changed() {
            format!(
                "Service {} {}",
                name,
                if check_mode {
                    "would be changed"
                } else {
                    "changed"
                }
            )
        } else {
            format!("Service {} is already in the requested state", name)
        },
        values: Default::default(),
    })
}

pub fn execute_systemd(
    connection: &dyn SshConnection,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    validate_params(args, &["name", "state", "enabled", "daemon_reload"])?;
    let daemon_reload = args
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String("daemon_reload".to_string())))
        .map(|value| match value {
            Value::Bool(value) => Ok(*value),
            _ => bail!("Systemd 'daemon_reload' must be a boolean"),
        })
        .transpose()?
        .unwrap_or(false);

    match detect_init_system(connection) {
        Ok(InitSystem::Systemd) => {}
        Ok(_) => bail!("The systemd module requires a Linux host running systemd"),
        Err(error) => bail!(
            "The systemd module requires a Linux host running systemd: {}",
            error
        ),
    }
    if daemon_reload && !check_mode {
        let (code, _, stderr) = run(
            connection,
            "systemctl daemon-reload",
            use_become,
            become_user,
        )?;
        if code != 0 {
            bail!("Failed to reload the systemd manager: {}", stderr.trim());
        }
    }

    let mut service_args = args
        .as_mapping()
        .cloned()
        .context("Systemd module requires a mapping of arguments")?;
    service_args.remove(Value::String("daemon_reload".to_string()));
    let mut result = execute(
        connection,
        &Value::Mapping(service_args),
        use_become,
        become_user,
        check_mode,
    )?;
    result.changed |= daemon_reload;
    if daemon_reload && !result.msg.contains("daemon") {
        result.msg.push_str(if check_mode {
            "; systemd daemon would be reloaded"
        } else {
            "; systemd daemon reloaded"
        });
    }
    Ok(result)
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

pub fn execute_systemd_adhoc(
    host: &Host,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    info!("Opening connection for host: {}", host.name);
    let connection = Connection::connect(host)?;
    execute_systemd(
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
    fn started_and_stopped_are_idempotent() {
        assert!(
            !plan_service(Some(ServiceState::Started), Some(true), false)
                .unwrap()
                .changed()
        );
        assert!(
            plan_service(Some(ServiceState::Started), Some(false), false)
                .unwrap()
                .changed()
        );
        assert!(
            !plan_service(Some(ServiceState::Stopped), Some(false), false)
                .unwrap()
                .changed()
        );
        assert!(plan_service(Some(ServiceState::Stopped), Some(true), false)
            .unwrap()
            .changed());
    }

    #[test]
    fn restart_reload_and_enablement_are_changes() {
        assert!(
            plan_service(Some(ServiceState::Restarted), Some(true), false)
                .unwrap()
                .changed()
        );
        assert!(
            plan_service(Some(ServiceState::Reloaded), Some(false), false)
                .unwrap()
                .changed()
        );
        assert!(plan_service(Some(ServiceState::Started), Some(true), true)
            .unwrap()
            .changed());
    }

    #[test]
    fn enablement_only_does_not_plan_a_state_transition() {
        let plan = plan_service(None, None, true).unwrap();
        assert!(plan.enable_change);
        assert!(!plan.state_change);
    }

    #[test]
    fn service_name_is_shell_quoted() {
        assert_eq!(
            state_command(InitSystem::Systemd, "demo;false", ServiceState::Started).unwrap(),
            "systemctl start -- 'demo;false'"
        );
    }

    #[test]
    fn enablement_only_check_mode_neither_queries_nor_changes_runtime_state() {
        let mut connection = MockSshConnection::new();
        connection
            .expect_execute_command()
            .times(2)
            .returning(|command| {
                assert!(!command.contains("is-active"));
                assert!(!command.contains(" start "));
                if command.contains("/run/systemd/system") {
                    Ok((0, String::new(), String::new()))
                } else {
                    assert!(command.contains("is-enabled"));
                    Ok((1, "disabled\n".to_string(), String::new()))
                }
            });
        connection.expect_execute_sudo_command().times(0);
        let args: Value = serde_yaml::from_str("name: demo\nenabled: true\n").unwrap();

        let result = execute(&connection, &args, false, "", true).unwrap();
        assert!(result.changed);
    }

    #[test]
    fn systemd_alias_reloads_manager_before_applying_service_state() {
        let mut connection = MockSshConnection::new();
        connection
            .expect_execute_command()
            .times(5)
            .returning(|command| {
                if command.contains("/run/systemd/system")
                    || command == "systemctl daemon-reload"
                    || command.contains("is-active")
                {
                    Ok((0, String::new(), String::new()))
                } else if command.contains("is-enabled") {
                    Ok((0, "enabled\n".to_string(), String::new()))
                } else {
                    panic!("unexpected systemd command: {command}")
                }
            });
        let args: Value = serde_yaml::from_str(
            "name: demo.service\nstate: started\nenabled: true\ndaemon_reload: true\n",
        )
        .unwrap();

        let result = execute_systemd(&connection, &args, false, "root", false).unwrap();
        assert!(result.changed);
        assert!(result.msg.contains("daemon"));
    }

    #[test]
    fn systemd_alias_rejects_non_systemd_hosts() {
        let mut connection = MockSshConnection::new();
        connection
            .expect_execute_command()
            .times(3)
            .returning(|command| {
                if command.contains("service >/dev/null") {
                    Ok((0, String::new(), String::new()))
                } else {
                    Ok((1, String::new(), String::new()))
                }
            });
        let args: Value = serde_yaml::from_str("name: demo\nstate: started\n").unwrap();

        let error = execute_systemd(&connection, &args, false, "root", false).unwrap_err();
        assert!(error
            .to_string()
            .contains("requires a Linux host running systemd"));
    }
}
