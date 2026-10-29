use crate::tools;
use anyhow::{bail, Context, Result};
use rustsible_protocol::{param::get_optional_param as optional, registry::Builtin, ModuleResult};
use serde_yaml::Value;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Init {
    Systemd,
    OpenRc,
    Upstart,
    SysV,
}
fn detect() -> Result<Init> {
    if std::path::Path::new("/run/systemd/system").is_dir() && tools::available("systemctl") {
        return Ok(Init::Systemd);
    }
    if tools::available("rc-service") {
        return Ok(Init::OpenRc);
    }
    if tools::available("initctl") {
        return Ok(Init::Upstart);
    }
    if tools::available("service") {
        return Ok(Init::SysV);
    }
    bail!("No supported service manager is available")
}
fn active(init: Init, name: &str) -> Result<bool> {
    let response = match init {
        Init::Systemd => tools::query("systemctl", &["is-active", "--quiet", "--", name])?,
        Init::OpenRc => tools::query("rc-service", &[name, "status"])?,
        Init::SysV => tools::query("service", &[name, "status"])?,
        Init::Upstart => tools::query("initctl", &["status", name])?,
    };
    match (init, response.rc) {
        (_, Some(0)) => {
            if init == Init::Upstart {
                Ok(response.stdout.contains("start/running"))
            } else {
                Ok(true)
            }
        }
        (Init::Systemd | Init::OpenRc, Some(3)) | (Init::SysV, Some(1 | 3)) => Ok(false),
        _ => Err(tools::failed(response, "Read service state")),
    }
}
fn enabled(init: Init, name: &str) -> Result<bool> {
    if init == Init::OpenRc {
        let response = tools::require(
            tools::query("rc-update", &["show", "-v", "default"])?,
            "Read enabled services",
        )?;
        return Ok(response.stdout.lines().any(|line| {
            line.split_once('|').is_some_and(|(service, levels)| {
                service.trim() == name && levels.split_whitespace().any(|level| level == "default")
            })
        }));
    }
    let response = tools::query("systemctl", &["is-enabled", "--", name])?;
    match response.rc {
        Some(0) => Ok(!matches!(
            response.stdout.trim(),
            "enabled-runtime" | "indirect" | "alias"
        )),
        Some(1) => Ok(false),
        _ => Err(tools::failed(response, "Read service enablement")),
    }
}
pub fn execute(args: &Value, check: bool, systemd: bool) -> Result<ModuleResult> {
    (if systemd {
        Builtin::Systemd
    } else {
        Builtin::Service
    })
    .validate_parameters(args)?;
    let name = optional::<String>(args, "name")?;
    let state = optional::<String>(args, "state")?.map(|state| state.to_ascii_lowercase());
    let enable = optional::<bool>(args, "enabled")?;
    let reload = if systemd {
        optional::<bool>(args, "daemon_reload")?.unwrap_or(false)
    } else {
        false
    };
    if name.as_ref().is_some_and(|name| {
        name.is_empty()
            || name.starts_with('-')
            || name.chars().any(char::is_whitespace)
            || name.chars().any(char::is_control)
    }) {
        bail!("Invalid service name");
    }
    let init = detect()?;
    if systemd && init != Init::Systemd {
        bail!("systemd requires an active systemd service manager");
    }
    if enable.is_some() && !matches!(init, Init::Systemd | Init::OpenRc) {
        bail!("enabled is supported only with systemd or OpenRC");
    }
    // Parameter/type validation is complete before daemon-reload. A newly
    // installed unit may only become visible to subsequent queries after it.
    if reload && !check {
        tools::mutate(
            "systemctl",
            &["daemon-reload"],
            "Reload systemd unit definitions",
        )?;
    }
    let Some(name) = name else {
        return Ok(result(
            reload,
            "Service manager definitions reloaded or predicted",
        ));
    };
    if init == Init::OpenRc {
        tools::require(
            tools::query("rc-service", &["-e", &name])?,
            "Locate OpenRC service",
        )?;
    }
    let enable_change = enable
        .map(|desired| enabled(init, &name).map(|actual| actual != desired))
        .transpose()?
        .unwrap_or(false);
    let current = state.as_ref().map(|_| active(init, &name)).transpose()?;
    let state_change = match state.as_deref() {
        None => false,
        Some("started") => !current.context("Missing service state")?,
        Some("stopped") => current.context("Missing service state")?,
        Some("restarted" | "reloaded") => true,
        _ => bail!("Invalid service state"),
    };
    if !check {
        if enable_change {
            let desired = enable.context("Missing enablement plan")?;
            if init == Init::Systemd {
                tools::mutate(
                    "systemctl",
                    &[if desired { "enable" } else { "disable" }, "--", &name],
                    "Update service enablement",
                )?;
            } else {
                tools::mutate(
                    "rc-update",
                    &[if desired { "add" } else { "del" }, &name, "default"],
                    "Update service enablement",
                )?;
            }
        }
        if state_change {
            let operation = match state.as_deref() {
                Some("started") => "start",
                Some("stopped") => "stop",
                Some("restarted") => "restart",
                Some("reloaded") => "reload",
                _ => unreachable!(),
            };
            match init {
                Init::Systemd => {
                    tools::mutate(
                        "systemctl",
                        &[operation, "--", &name],
                        "Change service state",
                    )?;
                }
                Init::OpenRc => {
                    tools::mutate(
                        "rc-service",
                        &[name.as_str(), operation],
                        "Change service state",
                    )?;
                }
                Init::SysV => {
                    tools::mutate(
                        "service",
                        &[name.as_str(), operation],
                        "Change service state",
                    )?;
                }
                Init::Upstart => {
                    tools::mutate("initctl", &[operation, &name], "Change service state")?;
                }
            }
        }
    }
    let mut result = result(
        reload || enable_change || state_change,
        "Service is current or updated",
    );
    if let Some(state) = state {
        result.values.insert("state".into(), state.into());
    }
    if let Some(enabled) = enable {
        result.values.insert("enabled".into(), enabled.into());
    }
    Ok(result)
}
fn result(changed: bool, msg: &str) -> ModuleResult {
    ModuleResult {
        changed,
        msg: msg.into(),
        ..ModuleResult::default()
    }
}
