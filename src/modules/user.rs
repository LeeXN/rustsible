use anyhow::{bail, Context, Result};
use log::info;
use serde::de::DeserializeOwned;
use serde_yaml::Value;
use std::collections::BTreeSet;

use crate::inventory::Host;
use crate::modules::file::quote_posix_shell_arg;
use crate::modules::param::get_param;
use crate::modules::ModuleResult;
use crate::ssh::connection::{Connection, SshConnection};

#[derive(Debug, Clone, PartialEq, Eq)]
struct AccountState {
    uid: i64,
    gid: i64,
    comment: String,
    home: String,
    shell: String,
}

#[derive(Debug, Clone)]
struct DesiredAccount {
    uid: Option<i64>,
    gid: Option<i64>,
    primary_group: Option<String>,
    groups: Option<Vec<String>>,
    append: bool,
    home: Option<String>,
    shell: Option<String>,
    comment: Option<String>,
    password: Option<String>,
    create_home: bool,
    system: bool,
}

const SUPPORTED_PARAMETERS: &[&str] = &[
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
];

fn validate_parameters(args: &Value) -> Result<()> {
    let map = args
        .as_mapping()
        .context("User arguments must be a mapping")?;
    for key in map.keys() {
        let Value::String(key) = key else {
            bail!("User parameter names must be strings");
        };
        if !SUPPORTED_PARAMETERS.contains(&key.as_str()) {
            bail!("Unsupported user parameter: {key}");
        }
    }
    Ok(())
}

fn optional<T: DeserializeOwned>(args: &Value, name: &str) -> Result<Option<T>> {
    let map = args
        .as_mapping()
        .context("User arguments must be a mapping")?;
    match map.get(Value::String(name.to_string())) {
        Some(value) => serde_yaml::from_value(value.clone())
            .with_context(|| format!("Invalid value for user parameter '{name}'"))
            .map(Some),
        None => Ok(None),
    }
}

fn parse_groups(args: &Value) -> Result<Option<Vec<String>>> {
    let map = args
        .as_mapping()
        .context("User arguments must be a mapping")?;
    let Some(value) = map.get(Value::String("groups".to_string())) else {
        return Ok(None);
    };
    let groups = match value {
        Value::String(value) => {
            if value.is_empty() {
                Vec::new()
            } else {
                value.split(',').map(str::to_string).collect()
            }
        }
        Value::Sequence(values) => values
            .iter()
            .map(|value| match value {
                Value::String(group) => Ok(group.clone()),
                _ => bail!("Every user group must be a string"),
            })
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("User 'groups' must be a string or a list of strings"),
    };
    for group in &groups {
        if group.is_empty() || group.contains([',', '\n', '\r', '\0']) || group.starts_with('-') {
            bail!("Invalid group name");
        }
    }
    Ok(Some(groups))
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

fn run_mutation(
    connection: &dyn SshConnection,
    command: &str,
    use_become: bool,
    become_user: &str,
    operation: &str,
) -> Result<()> {
    let (code, _, stderr) = run(connection, command, use_become, become_user)?;
    if code != 0 {
        bail!("Failed to {operation} (exit {code}): {}", stderr.trim());
    }
    Ok(())
}

fn parse_passwd_line(line: &str, expected_name: &str) -> Result<AccountState> {
    let fields = line
        .trim_end_matches(['\r', '\n'])
        .splitn(7, ':')
        .collect::<Vec<_>>();
    if fields.len() != 7 || fields[0] != expected_name {
        bail!("Unexpected passwd database response for user {expected_name}");
    }
    Ok(AccountState {
        uid: fields[2]
            .parse()
            .with_context(|| format!("Invalid uid in passwd entry for {expected_name}"))?,
        gid: fields[3]
            .parse()
            .with_context(|| format!("Invalid gid in passwd entry for {expected_name}"))?,
        comment: fields[4].to_string(),
        home: fields[5].to_string(),
        shell: fields[6].to_string(),
    })
}

fn account_state(
    connection: &dyn SshConnection,
    name: &str,
    use_become: bool,
    become_user: &str,
) -> Result<Option<AccountState>> {
    let command = format!("getent passwd {}", quote_posix_shell_arg(name)?);
    let (code, stdout, stderr) = run(connection, &command, use_become, become_user)?;
    match code {
        0 => Ok(Some(parse_passwd_line(&stdout, name)?)),
        2 => Ok(None),
        code => bail!(
            "Failed to query user {} (exit {}): {}",
            name,
            code,
            stderr.trim()
        ),
    }
}

fn primary_group_name(
    connection: &dyn SshConnection,
    gid: i64,
    use_become: bool,
    become_user: &str,
) -> Result<String> {
    let command = format!("getent group {}", quote_posix_shell_arg(&gid.to_string())?);
    let (code, stdout, stderr) = run(connection, &command, use_become, become_user)?;
    if code != 0 {
        bail!(
            "Failed to resolve primary group {} (exit {}): {}",
            gid,
            code,
            stderr.trim()
        );
    }
    stdout
        .split(':')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .context("Group database returned an invalid primary group")
}

fn resolve_group_gid(
    connection: &dyn SshConnection,
    group: &str,
    use_become: bool,
    become_user: &str,
) -> Result<i64> {
    if group.is_empty() || group.starts_with('-') || group.contains([':', '\n', '\r', '\0']) {
        bail!("Invalid primary group name");
    }
    let command = format!("getent group {}", quote_posix_shell_arg(group)?);
    let (code, stdout, stderr) = run(connection, &command, use_become, become_user)?;
    if code != 0 {
        bail!(
            "Failed to resolve primary group '{}' (exit {}): {}",
            group,
            code,
            stderr.trim()
        );
    }
    let fields = stdout
        .trim_end_matches(['\r', '\n'])
        .splitn(4, ':')
        .collect::<Vec<_>>();
    if fields.len() != 4 {
        bail!("Group database returned an invalid entry for '{group}'");
    }
    fields[2]
        .parse::<i64>()
        .with_context(|| format!("Group database returned an invalid gid for '{group}'"))
}

fn supplementary_groups(
    connection: &dyn SshConnection,
    name: &str,
    primary_gid: i64,
    use_become: bool,
    become_user: &str,
) -> Result<BTreeSet<String>> {
    let command = format!("id -nG -- {}", quote_posix_shell_arg(name)?);
    let (code, stdout, stderr) = run(connection, &command, use_become, become_user)?;
    if code != 0 {
        bail!(
            "Failed to query groups for {} (exit {}): {}",
            name,
            code,
            stderr.trim()
        );
    }
    let primary = primary_group_name(connection, primary_gid, use_become, become_user)?;
    let mut groups = stdout
        .split_whitespace()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    groups.remove(&primary);
    Ok(groups)
}

fn shadow_hash(
    connection: &dyn SshConnection,
    name: &str,
    use_become: bool,
    become_user: &str,
) -> Result<String> {
    let command = format!("getent shadow {}", quote_posix_shell_arg(name)?);
    let (code, stdout, stderr) = run(connection, &command, use_become, become_user)?;
    if code != 0 {
        bail!(
            "Cannot compare the password hash for user {} (exit {}): {}. Use become with an account allowed to read the shadow database",
            name,
            code,
            stderr.trim()
        );
    }
    let mut fields = stdout.trim_end_matches(['\r', '\n']).split(':');
    let returned_name = fields.next().unwrap_or_default();
    let hash = fields.next().unwrap_or_default();
    if returned_name != name || hash.is_empty() {
        bail!(
            "Cannot compare the password hash for user {}: the shadow database returned no usable hash",
            name
        );
    }
    Ok(hash.to_string())
}

fn groups_need_change(current: &BTreeSet<String>, desired: &[String], append: bool) -> bool {
    let desired = desired.iter().cloned().collect::<BTreeSet<_>>();
    if append {
        !desired.is_subset(current)
    } else {
        current != &desired
    }
}

fn attribute_changes(
    current: &AccountState,
    desired: &DesiredAccount,
    desired_primary_gid: Option<i64>,
    primary_group_argument: Option<&str>,
) -> Result<Vec<String>> {
    let mut arguments = Vec::new();
    if let Some(uid) = desired.uid {
        if uid < 0 {
            bail!("uid cannot be negative");
        }
        if current.uid != uid {
            arguments.extend([
                "--uid".to_string(),
                quote_posix_shell_arg(&uid.to_string())?,
            ]);
        }
    }
    if let Some(gid) = desired_primary_gid {
        if gid < 0 {
            bail!("gid cannot be negative");
        }
        if current.gid != gid {
            arguments.extend([
                "--gid".to_string(),
                quote_posix_shell_arg(
                    primary_group_argument
                        .context("Internal primary-group plan is missing its argument")?,
                )?,
            ]);
        }
    }
    for (flag, actual, wanted) in [
        ("--home", current.home.as_str(), desired.home.as_deref()),
        ("--shell", current.shell.as_str(), desired.shell.as_deref()),
        (
            "--comment",
            current.comment.as_str(),
            desired.comment.as_deref(),
        ),
    ] {
        if let Some(wanted) = wanted {
            if actual != wanted {
                arguments.extend([flag.to_string(), quote_posix_shell_arg(wanted)?]);
            }
        }
    }
    Ok(arguments)
}

fn create_command(
    name: &str,
    desired: &DesiredAccount,
    primary_group_argument: Option<&str>,
) -> Result<String> {
    let mut command = vec!["useradd".to_string()];
    if let Some(uid) = desired.uid {
        if uid < 0 {
            bail!("uid cannot be negative");
        }
        command.extend([
            "--uid".to_string(),
            quote_posix_shell_arg(&uid.to_string())?,
        ]);
    }
    if let Some(group) = primary_group_argument {
        command.extend(["--gid".to_string(), quote_posix_shell_arg(group)?]);
    }
    for (flag, value) in [
        ("--home-dir", desired.home.as_deref()),
        ("--shell", desired.shell.as_deref()),
        ("--comment", desired.comment.as_deref()),
    ] {
        if let Some(value) = value {
            command.extend([flag.to_string(), quote_posix_shell_arg(value)?]);
        }
    }
    command.push(
        if desired.create_home {
            "--create-home"
        } else {
            "--no-create-home"
        }
        .to_string(),
    );
    if desired.system {
        command.push("--system".to_string());
    }
    command.extend(["--".to_string(), quote_posix_shell_arg(name)?]);
    Ok(command.join(" "))
}

fn groups_command(name: &str, groups: &[String], append: bool) -> Result<String> {
    let mut command = vec!["usermod".to_string()];
    if append {
        command.push("--append".to_string());
    }
    command.extend([
        "--groups".to_string(),
        quote_posix_shell_arg(&groups.join(","))?,
        "--".to_string(),
        quote_posix_shell_arg(name)?,
    ]);
    Ok(command.join(" "))
}

fn chpasswd_record(name: &str, password_hash: &str) -> Result<Vec<u8>> {
    if name.contains([':', '\n', '\r', '\0']) || password_hash.contains([':', '\n', '\r', '\0']) {
        bail!("User name and password hash must form one chpasswd record");
    }
    let mut record = format!("{}:{}", name, password_hash).into_bytes();
    record.push(b'\n');
    Ok(record)
}

fn set_password(
    connection: &dyn SshConnection,
    name: &str,
    password: &str,
    use_become: bool,
    become_user: &str,
) -> Result<()> {
    let record = chpasswd_record(name, password)?;
    let (code, _, stderr) = if use_become {
        connection.execute_sudo_command_with_input("chpasswd -e", become_user, &record)?
    } else {
        connection.execute_command_with_input("chpasswd -e", &record)?
    };
    if code != 0 {
        bail!("Failed to set password for {}: {}", name, stderr.trim());
    }
    Ok(())
}

pub fn execute(
    connection: &dyn SshConnection,
    args: &Value,
    use_become: bool,
    become_user: &str,
    check_mode: bool,
) -> Result<ModuleResult> {
    validate_parameters(args)?;
    let name = get_param::<String>(args, "name")?;
    if name.is_empty() || name.starts_with('-') || name.contains([':', '\n', '\r', '\0']) {
        bail!("Invalid user name");
    }
    let state = optional::<String>(args, "state")?.unwrap_or_else(|| "present".to_string());
    if !matches!(state.as_str(), "present" | "absent") {
        bail!("Invalid user state: {state}");
    }
    let remove = optional::<bool>(args, "remove")?.unwrap_or(false);
    let requested_gid = optional::<i64>(args, "gid")?;
    let primary_group = optional::<String>(args, "group")?;
    if requested_gid.is_some() && primary_group.is_some() {
        bail!("User parameters 'group' and 'gid' are mutually exclusive");
    }
    let desired = DesiredAccount {
        uid: optional(args, "uid")?,
        gid: requested_gid,
        primary_group,
        groups: parse_groups(args)?,
        append: optional::<bool>(args, "append")?.unwrap_or(false),
        home: optional(args, "home")?,
        shell: optional(args, "shell")?,
        comment: optional(args, "comment")?,
        password: optional(args, "password")?,
        create_home: optional::<bool>(args, "create_home")?.unwrap_or(true),
        system: optional::<bool>(args, "system")?.unwrap_or(false),
    };

    info!("Managing a user account");
    let current = account_state(connection, &name, use_become, become_user)?;
    if state == "absent" {
        let changed = current.is_some();
        if changed && !check_mode {
            let command = format!(
                "userdel {}-- {}",
                if remove { "--remove " } else { "" },
                quote_posix_shell_arg(&name)?
            );
            run_mutation(connection, &command, use_become, become_user, "remove user")?;
        }
        return Ok(ModuleResult {
            stdout: String::new(),
            stderr: String::new(),
            rc: None,
            changed,
            failed: false,
            msg: if changed {
                format!(
                    "User {} {}",
                    name,
                    if check_mode {
                        "would be removed"
                    } else {
                        "removed"
                    }
                )
            } else {
                format!("User {} is already absent", name)
            },
        });
    }

    let primary_group_argument = desired
        .primary_group
        .clone()
        .or_else(|| desired.gid.map(|gid| gid.to_string()));
    let desired_primary_gid = match desired.primary_group.as_deref() {
        Some(group) => Some(resolve_group_gid(
            connection,
            group,
            use_become,
            become_user,
        )?),
        None => desired.gid,
    };
    let creating = current.is_none();
    let attribute_arguments = current
        .as_ref()
        .map(|current| {
            attribute_changes(
                current,
                &desired,
                desired_primary_gid,
                primary_group_argument.as_deref(),
            )
        })
        .transpose()?
        .unwrap_or_default();
    let groups_changed = match (&current, desired.groups.as_deref()) {
        (Some(current), Some(groups)) => {
            let actual =
                supplementary_groups(connection, &name, current.gid, use_become, become_user)?;
            groups_need_change(&actual, groups, desired.append)
        }
        (None, Some(groups)) => !groups.is_empty(),
        (_, None) => false,
    };
    let password_changed = match (&current, desired.password.as_deref()) {
        (Some(_), Some(password)) => {
            shadow_hash(connection, &name, use_become, become_user)? != password
        }
        (None, Some(_)) => true,
        (_, None) => false,
    };
    let changed = creating || !attribute_arguments.is_empty() || groups_changed || password_changed;

    if changed && !check_mode {
        if creating {
            let command = create_command(&name, &desired, primary_group_argument.as_deref())?;
            run_mutation(connection, &command, use_become, become_user, "create user")?;
        } else if !attribute_arguments.is_empty() {
            let command = format!(
                "usermod {} -- {}",
                attribute_arguments.join(" "),
                quote_posix_shell_arg(&name)?
            );
            run_mutation(connection, &command, use_become, become_user, "modify user")?;
        }
        if groups_changed {
            let groups = desired
                .groups
                .as_deref()
                .context("Internal user group plan error")?;
            let command = groups_command(&name, groups, desired.append)?;
            run_mutation(
                connection,
                &command,
                use_become,
                become_user,
                "change user groups",
            )?;
        }
        if password_changed {
            set_password(
                connection,
                &name,
                desired
                    .password
                    .as_deref()
                    .context("Internal user password plan error")?,
                use_become,
                become_user,
            )?;
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
                "User {} {}",
                name,
                if check_mode {
                    "would be changed"
                } else {
                    "changed"
                }
            )
        } else {
            format!("User {} is already in the requested state", name)
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

    fn current() -> AccountState {
        AccountState {
            uid: 1000,
            gid: 1000,
            comment: "Alice".to_string(),
            home: "/home/alice".to_string(),
            shell: "/bin/sh".to_string(),
        }
    }

    fn desired() -> DesiredAccount {
        DesiredAccount {
            uid: Some(1000),
            gid: Some(1000),
            primary_group: None,
            groups: None,
            append: false,
            home: Some("/home/alice".to_string()),
            shell: Some("/bin/sh".to_string()),
            comment: Some("Alice".to_string()),
            password: None,
            create_home: true,
            system: false,
        }
    }

    #[test]
    fn passwd_state_and_attributes_are_compared() {
        assert_eq!(
            parse_passwd_line("alice:x:1000:1000:Alice:/home/alice:/bin/sh\n", "alice").unwrap(),
            current()
        );
        assert!(
            attribute_changes(&current(), &desired(), Some(1000), Some("1000"))
                .unwrap()
                .is_empty()
        );
        let mut changed = desired();
        changed.shell = Some("/bin/bash".to_string());
        assert_eq!(
            attribute_changes(&current(), &changed, Some(1000), Some("1000")).unwrap(),
            vec!["--shell", "'/bin/bash'"]
        );
    }

    #[test]
    fn group_append_and_replace_plans_are_idempotent() {
        let current = ["adm".to_string(), "wheel".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(!groups_need_change(&current, &["adm".to_string()], true));
        assert!(groups_need_change(&current, &["adm".to_string()], false));
        assert!(!groups_need_change(
            &current,
            &["wheel".to_string(), "adm".to_string()],
            false
        ));
    }

    #[test]
    fn password_record_is_not_shell_interpolated() {
        assert_eq!(
            chpasswd_record("alice", "$6$salt$hash").unwrap(),
            b"alice:$6$salt$hash\n"
        );
        assert!(chpasswd_record("alice\nroot", "hash").is_err());
        assert!(chpasswd_record("alice", "hash\nroot:hash").is_err());
        assert!(chpasswd_record("alice", "hash:extra-field").is_err());
    }

    #[test]
    fn primary_group_generates_gid_arguments() {
        let mut desired = desired();
        desired.gid = None;
        desired.primary_group = Some("operators".to_string());
        assert_eq!(
            attribute_changes(&current(), &desired, Some(2000), Some("operators")).unwrap(),
            vec!["--gid", "'operators'"]
        );
        assert!(create_command("alice", &desired, Some("operators"))
            .unwrap()
            .contains("--gid 'operators'"));
    }

    #[test]
    fn check_mode_only_reads_when_attributes_differ() {
        let mut connection = MockSshConnection::new();
        connection
            .expect_execute_command()
            .times(1)
            .returning(|command| {
                assert!(command.starts_with("getent passwd "));
                Ok((
                    0,
                    "alice:x:1000:1000:Alice:/home/alice:/bin/sh\n".to_string(),
                    String::new(),
                ))
            });
        connection.expect_execute_sudo_command().times(0);
        let args: Value =
            serde_yaml::from_str("name: alice\nstate: present\nshell: /bin/bash\ncomment: Alice\n")
                .unwrap();

        let result = execute(&connection, &args, false, "", true).unwrap();
        assert!(result.changed);
    }

    #[test]
    fn misspelled_parameters_are_rejected() {
        let args: Value = serde_yaml::from_str("name: alice\nsheel: /bin/bash\n").unwrap();
        assert!(validate_parameters(&args)
            .unwrap_err()
            .to_string()
            .contains("sheel"));
    }
}
