use crate::tools;
use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    change,
    param::{get_optional_param as optional, get_param},
    registry::Builtin,
    ModuleResult,
};
use serde_yaml::Value;
use std::collections::BTreeSet;
#[derive(Debug)]
struct Account {
    uid: u32,
    gid: u32,
    comment: String,
    home: String,
    shell: String,
}
fn query(database: &str, key: &str) -> Result<Option<String>> {
    let result = tools::query("getent", &[database, key])?;
    match result.rc {
        Some(0) => Ok(Some(result.stdout.trim_end_matches(['\r', '\n']).into())),
        Some(2) => Ok(None),
        _ => Err(tools::failed(result, "Query account database")),
    }
}
fn account(name: &str) -> Result<Option<Account>> {
    query("passwd", name)?
        .map(|line| {
            let fields: Vec<_> = line.splitn(7, ':').collect();
            if fields.len() != 7 || fields[0] != name {
                bail!("Unexpected passwd database response");
            }
            Ok(Account {
                uid: fields[2].parse()?,
                gid: fields[3].parse()?,
                comment: fields[4].into(),
                home: fields[5].into(),
                shell: fields[6].into(),
            })
        })
        .transpose()
}
fn group(value: &str) -> Result<u32> {
    group_entry(value)?
        .map(|(_, gid)| gid)
        .context("Requested primary/supplementary group does not exist")
}
fn group_entry(value: &str) -> Result<Option<(String, u32)>> {
    validate_name(value)?;
    query("group", value)?
        .map(|entry| {
            let fields = entry.splitn(4, ':').collect::<Vec<_>>();
            if fields.len() != 4 {
                bail!("Invalid group database response");
            }
            Ok((fields[0].to_owned(), fields[2].parse()?))
        })
        .transpose()
}
fn validate_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(char::is_control)
        || value.contains([':', ','])
    {
        bail!("Invalid account/group name");
    }
    Ok(())
}
fn groups(args: &Value) -> Result<Option<Vec<String>>> {
    match args.get("groups") {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(if text.is_empty() {
            Vec::new()
        } else {
            text.split(',').map(str::to_owned).collect()
        })),
        Some(Value::Sequence(values)) => Ok(Some(
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .context("Every group must be a string")
                        .map(str::to_owned)
                })
                .collect::<Result<_>>()?,
        )),
        _ => bail!("groups must be a string or string list"),
    }
}
fn shadow(name: &str) -> Result<String> {
    query("shadow", name)?
        .and_then(|entry| entry.split(':').nth(1).map(str::to_owned))
        .context("Cannot read the account's shadow entry; use an authorized execution identity")
}
fn password(name: &str, hash: &str) -> Result<()> {
    change::ensure_mutation_allowed()?;
    let record = format!("{name}:{hash}\n");
    let mut result = tools::run("chpasswd", &["-e"], Some(record.as_bytes()), true)?;
    // Even an OS tool failure must not reflect a password hash supplied on
    // stdin. All other account tools receive only public attributes in argv.
    if !hash.is_empty() {
        result.stdout = result.stdout.replace(hash, "<redacted>");
        result.stderr = result.stderr.replace(hash, "<redacted>");
    }
    tools::require(result, "Set account password")?;
    change::record_change();
    Ok(())
}
pub fn execute(args: &Value, check: bool) -> Result<ModuleResult> {
    Builtin::User.validate_parameters(args)?;
    let name = get_param::<String>(args, "name")?;
    validate_name(&name)?;
    let state = optional::<String>(args, "state")?
        .unwrap_or_else(|| "present".into())
        .to_ascii_lowercase();
    let uid = optional::<u32>(args, "uid")?;
    let gid = optional::<u32>(args, "gid")?;
    let primary = optional::<String>(args, "group")?;
    if gid.is_some() && primary.is_some() {
        bail!("group and gid are mutually exclusive");
    }
    let desired_groups = groups(args)?;
    if let Some(groups) = &desired_groups {
        for group in groups {
            validate_name(group)?;
        }
    }
    let append = optional::<bool>(args, "append")?.unwrap_or(false);
    let home = optional::<String>(args, "home")?;
    let shell = optional::<String>(args, "shell")?;
    let comment = optional::<String>(args, "comment")?;
    let hash = optional::<String>(args, "password")?;
    let create_home = optional::<bool>(args, "create_home")?.unwrap_or(true);
    let system = optional::<bool>(args, "system")?.unwrap_or(false);
    let remove = optional::<bool>(args, "remove")?.unwrap_or(false);
    for value in [&home, &shell, &comment].into_iter().flatten() {
        if value.contains([':', '\r', '\n', '\0']) {
            bail!("Account attributes cannot contain record delimiters");
        }
    }
    if hash
        .as_ref()
        .is_some_and(|hash| hash.contains([':', '\r', '\n', '\0']))
    {
        bail!("Password must form a single chpasswd record");
    }
    if home
        .as_ref()
        .is_some_and(|home| !std::path::Path::new(home).is_absolute())
    {
        bail!("Account home must be an absolute path");
    }
    let current = account(&name)?;
    if state == "absent" {
        if current.is_some() && !check {
            let mut argv = Vec::new();
            if remove {
                argv.push("--remove".into());
            }
            argv.extend(["--".into(), name.clone()]);
            tools::mutate("userdel", &argv, "Remove user")?;
        }
        return Ok(result(
            current.is_some(),
            "Account is absent or would be removed",
        ));
    }
    // Resolve every referenced group before useradd/usermod can change state.
    let mut primary_arg = primary.clone().or_else(|| gid.map(|gid| gid.to_string()));
    if primary_arg.is_none() && current.is_none() && query("group", &name)?.is_some() {
        primary_arg = Some(name.clone());
    }
    let desired_gid = primary_arg.as_ref().map(|value| group(value)).transpose()?;
    let group_ids = desired_groups
        .as_ref()
        .map(|groups| {
            groups
                .iter()
                .map(|name| group(name))
                .collect::<Result<BTreeSet<_>>>()
        })
        .transpose()?;
    if hash.is_some() && !tools::available("chpasswd") {
        bail!("chpasswd is required to set account passwords");
    }
    let creating = current.is_none();
    let mut argv = Vec::<String>::new();
    if creating {
        if let Some(uid) = uid {
            argv.extend(["--uid".into(), uid.to_string()]);
        }
        if let Some(group) = primary_arg.clone() {
            argv.extend(["--gid".into(), group]);
        }
        for (flag, value) in [
            ("--home-dir", &home),
            ("--shell", &shell),
            ("--comment", &comment),
        ] {
            if let Some(value) = value {
                argv.extend([flag.into(), value.clone()]);
            }
        }
        if let Some(groups) = &desired_groups {
            argv.extend(["--groups".into(), groups.join(",")]);
        }
        argv.push(
            if create_home {
                "--create-home"
            } else {
                "--no-create-home"
            }
            .into(),
        );
        if system {
            argv.push("--system".into());
        }
    } else {
        let old = current.as_ref().context("Missing account state")?;
        if uid.is_some_and(|uid| uid != old.uid) {
            argv.extend(["--uid".into(), uid.context("Missing uid")?.to_string()]);
        }
        if desired_gid.is_some_and(|gid| gid != old.gid) {
            argv.extend([
                "--gid".into(),
                primary_arg.clone().context("Missing primary group")?,
            ]);
        }
        for (flag, wanted, actual) in [
            ("--home", &home, &old.home),
            ("--shell", &shell, &old.shell),
            ("--comment", &comment, &old.comment),
        ] {
            if let Some(wanted) = wanted {
                if wanted != actual {
                    argv.extend([flag.into(), wanted.clone()]);
                }
            }
        }
        if let (Some(names), Some(wanted)) = (&desired_groups, group_ids) {
            let response = tools::require(
                tools::query("id", &["-G", "--", &name])?,
                "Read supplementary groups",
            )?;
            let mut actual = response
                .stdout
                .split_whitespace()
                .map(str::parse::<u32>)
                .collect::<std::result::Result<BTreeSet<_>, _>>()?;
            actual.remove(&old.gid);
            let new_primary = desired_gid.unwrap_or(old.gid);
            actual.remove(&new_primary);
            let wanted = wanted
                .into_iter()
                .filter(|gid| *gid != new_primary)
                .collect::<BTreeSet<_>>();
            if (append && !wanted.is_subset(&actual)) || (!append && wanted != actual) {
                if append {
                    argv.push("--append".into());
                }
                argv.extend(["--groups".into(), names.join(",")]);
            }
        }
    }
    let password_changed = match (&current, hash.as_deref()) {
        (Some(_), Some(hash)) => shadow(&name)? != hash,
        (None, Some(_)) => true,
        _ => false,
    };
    let missing_home = current.as_ref().is_some_and(|old| {
        create_home && !std::path::Path::new(home.as_deref().unwrap_or(&old.home)).exists()
    });
    if missing_home && !tools::available("mkhomedir_helper") {
        bail!("mkhomedir_helper is required to create a missing home for an existing account");
    }
    let changed = creating || !argv.is_empty() || password_changed || missing_home;
    if changed && !check {
        if creating || !argv.is_empty() {
            argv.extend(["--".into(), name.clone()]);
            tools::mutate(
                if creating { "useradd" } else { "usermod" },
                &argv,
                "Update account",
            )?;
        }
        if password_changed {
            password(&name, hash.as_deref().context("Missing password plan")?)?;
        }
        if missing_home {
            tools::mutate("mkhomedir_helper", &[name.as_str()], "Create account home")?;
        }
    }
    Ok(result(changed, "Account is current or updated"))
}
fn result(changed: bool, msg: &str) -> ModuleResult {
    ModuleResult {
        changed,
        msg: msg.into(),
        ..ModuleResult::default()
    }
}

pub fn execute_group(args: &Value, check: bool) -> Result<ModuleResult> {
    Builtin::Group.validate_parameters(args)?;
    let name = get_param::<String>(args, "name")?;
    validate_name(&name)?;
    if name.chars().any(char::is_whitespace) {
        bail!("Group name cannot contain whitespace");
    }
    let state = optional::<String>(args, "state")?
        .unwrap_or_else(|| "present".into())
        .to_ascii_lowercase();
    let gid = optional::<u32>(args, "gid")?;
    let system = optional::<bool>(args, "system")?.unwrap_or(false);
    let current = group_entry(&name)?;
    if current.as_ref().is_some_and(|(actual, _)| actual != &name) {
        bail!("Group name resolved to a numeric ID alias; use the actual group name");
    }
    let changed = if state == "absent" {
        current.is_some()
    } else {
        current.is_none()
            || gid.is_some_and(|gid| current.as_ref().is_some_and(|(_, old)| *old != gid))
    };
    let operation = if state == "absent" {
        "groupdel"
    } else if current.is_none() {
        "groupadd"
    } else {
        "groupmod"
    };
    let mut argv = Vec::new();
    if state == "present" {
        if current.is_none() && system {
            argv.push("--system".into());
        }
        if let Some(gid) = gid {
            if changed {
                if group_entry(&gid.to_string())?.is_some_and(|(actual, _)| actual != name) {
                    bail!("Requested group ID is already in use");
                }
                argv.extend(["--gid".into(), gid.to_string()]);
            }
        }
    }
    argv.extend(["--".into(), name.clone()]);
    if changed {
        // Ordinary SSH login PATH often omits administrative directories even
        // for read-only prediction. Explicit PATH tools still take priority.
        let executable = if tools::available(operation) {
            Some(operation.to_owned())
        } else {
            ["/usr/sbin", "/sbin"]
                .iter()
                .map(|directory| format!("{directory}/{operation}"))
                .find(|path| tools::available(path))
        }
        .with_context(|| format!("Required group account tool is unavailable: {operation}"))?;
        if !check {
            tools::mutate(&executable, &argv, "Manage group")?;
        }
    }
    let mut final_gid = if state == "absent" {
        current.as_ref().map(|(_, gid)| *gid)
    } else {
        gid.or_else(|| current.as_ref().map(|(_, gid)| *gid))
    };
    if changed && !check {
        let observed = group_entry(&name)?;
        if state == "absent" {
            if observed.is_some() {
                bail!("Group removal could not be confirmed");
            }
        } else {
            let (actual_name, actual_gid) =
                observed.context("Group creation/modification could not be confirmed")?;
            if actual_name != name || gid.is_some_and(|gid| gid != actual_gid) {
                bail!("Group identity does not match the requested state");
            }
            final_gid = Some(actual_gid);
        }
    }
    let mut result = result(changed, "Group is current or would be updated");
    result.values.insert("name".into(), Value::String(name));
    result.values.insert("state".into(), Value::String(state));
    if let Some(gid) = final_gid {
        result
            .values
            .insert("gid".into(), Value::Number(gid.into()));
    }
    Ok(result)
}
