pub mod command;
pub mod copy;
pub mod debug;
pub mod file;
pub mod get_url;
pub mod lineinfile;
pub mod local;
pub mod package;
pub mod param;
mod python;
pub mod remote;
pub mod service;
pub mod setup;
pub mod shell;
pub mod stat;
pub mod template;
pub mod user;

use anyhow::{Context, Result};
use colored::Colorize;
use log::{debug, info};
use serde_yaml::Value;
use std::collections::HashMap;
use std::time::Instant;

use crate::inventory::Host;
use crate::ssh::connection::{Connection, SshConnection};

/// Result structure for unified handling of module returns
#[derive(Debug, Default)]
pub struct ModuleResult {
    pub stdout: String,
    pub stderr: String,
    pub rc: Option<i32>,
    pub changed: bool,
    pub failed: bool,
    pub msg: String,
    /// Module-specific return values exposed to `register`.
    pub values: HashMap<String, Value>,
}

/// Controls an ad-hoc run. `become_override` is optional so an explicit
/// command-line request can override inventory while an omitted flag still
/// inherits the per-host `ansible_become` setting.
#[derive(Debug, Clone)]
pub struct AdHocOptions {
    pub become_override: Option<bool>,
    pub become_user: Option<String>,
    pub check_mode: bool,
    pub forks: usize,
}

impl Default for AdHocOptions {
    fn default() -> Self {
        Self {
            become_override: None,
            become_user: None,
            check_mode: false,
            forks: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveAdHocOptions {
    use_become: bool,
    become_user: String,
    check_mode: bool,
}

/// Trait for common module execution patterns
pub trait ModuleExecutor {
    /// Execute the module with the given SSH client and arguments
    fn execute(
        connection: &dyn SshConnection,
        args: &Value,
        use_become: bool,
        become_user: &str,
        check_mode: bool,
    ) -> Result<ModuleResult>;

    /// Execute the module in ad-hoc mode for a single host
    fn execute_adhoc(
        host: &Host,
        args: &Value,
        use_become: bool,
        become_user: &str,
        check_mode: bool,
    ) -> Result<ModuleResult> {
        info!("Opening connection for host: {}", host.name);
        let connection = Connection::connect(host)?;
        Self::execute(
            connection.as_connection(),
            args,
            use_become,
            become_user,
            check_mode,
        )
    }

    /// Helper to execute a command on a remote host with proper sudo handling
    fn execute_command(
        connection: &dyn SshConnection,
        cmd: &str,
        use_become: bool,
        become_user: &str,
    ) -> Result<(i32, String, String)> {
        debug!("Executing module command ({} bytes)", cmd.len());

        if use_become {
            connection.execute_sudo_command(cmd, become_user)
        } else {
            connection.execute_command(cmd)
        }
    }

    /// Helper to process command execution results into a ModuleResult
    fn process_command_result(
        exit_code: i32,
        stdout: String,
        stderr: String,
        success_msg: &str,
        error_prefix: &str,
    ) -> Result<ModuleResult> {
        let module_result = ModuleResult {
            stdout,
            stderr: stderr.clone(),
            rc: Some(exit_code),
            changed: true,
            failed: exit_code != 0,
            msg: if exit_code == 0 {
                format!("{} (exit code: {})", success_msg, exit_code)
            } else if stderr.trim().is_empty() {
                format!("{} (exit code: {})", error_prefix, exit_code)
            } else {
                format!(
                    "{} (exit code: {}): {}",
                    error_prefix,
                    exit_code,
                    stderr.trim()
                )
            },
            values: Default::default(),
        };

        if exit_code != 0 {
            return Ok(module_result);
        }

        info!("{}", success_msg);
        Ok(module_result)
    }

    /// Helper to extract a string argument from Value, handling both String and Mapping with "cmd" key
    fn extract_command_arg(args: &Value) -> Result<String> {
        match args {
            Value::String(cmd) => Ok(cmd.clone()),
            Value::Mapping(map) => {
                if map.len() != 1 {
                    return Err(anyhow::anyhow!(
                        "Command modules accept only the 'cmd' parameter"
                    ));
                }
                if let Some(Value::String(cmd)) = map.get(Value::String("cmd".to_string())) {
                    Ok(cmd.clone())
                } else {
                    Err(anyhow::anyhow!("Module requires a valid command string"))
                }
            }
            _ => Err(anyhow::anyhow!("Module requires a valid command string")),
        }
    }
}

/// Run an ad-hoc command on a list of hosts
pub fn run_adhoc(hosts: &[Host], module_name: &str, args: &str) -> Result<()> {
    run_adhoc_with_options(hosts, module_name, args, &AdHocOptions::default())
}

/// Run an ad-hoc command with explicit execution controls.
pub fn run_adhoc_with_options(
    hosts: &[Host],
    module_name: &str,
    args: &str,
    options: &AdHocOptions,
) -> Result<()> {
    if hosts.is_empty() {
        return Err(anyhow::anyhow!(
            "Cannot run an ad-hoc command without any target hosts"
        ));
    }
    if options.forks == 0 {
        return Err(anyhow::anyhow!("--forks must be greater than zero"));
    }
    if options
        .become_user
        .as_deref()
        .is_some_and(|user| user.trim().is_empty() || user.chars().any(char::is_control))
    {
        return Err(anyhow::anyhow!(
            "--become-user must be non-empty and contain no control characters"
        ));
    }

    let module_name = module_name
        .strip_prefix("ansible.builtin.")
        .or_else(|| module_name.strip_prefix("ansible.legacy."))
        .unwrap_or(module_name);

    // Validate the module and parse mapped arguments before starting any
    // connections. Besides failing fast, this deliberately parses the command
    // line only once rather than once per host.
    let module_args = prepare_adhoc_args(module_name, args)?;

    info!(
        "Running ad-hoc module '{}' on {} hosts",
        module_name,
        hosts.len()
    );

    println!("\n{}", "TASK [Execute ad-hoc command]".bold());
    println!("{}\n", "------------------------------".dimmed());

    // Keep the amount of simultaneous SSH/local work bounded. Chunks are
    // joined in their original order so user-visible output and recap remain
    // deterministic even though execution within each chunk is concurrent.
    let mut executions = Vec::with_capacity(hosts.len());
    for chunk in hosts.chunks(options.forks) {
        let chunk_executions = std::thread::scope(|scope| {
            let module_args = &module_args;
            let handles = chunk
                .iter()
                .map(|host| {
                    let effective_options = effective_adhoc_options(host, options);
                    scope.spawn(move || {
                        info!("Running module {} on host {}", module_name, host.name);
                        let start_time = Instant::now();
                        let result = execute_adhoc_module(
                            host,
                            module_name,
                            module_args,
                            &effective_options,
                        );
                        HostExecution {
                            result,
                            duration: start_time.elapsed(),
                        }
                    })
                })
                .collect::<Vec<_>>();

            handles
                .into_iter()
                .map(|handle| {
                    handle.join().unwrap_or_else(|_| HostExecution {
                        result: Err(anyhow::anyhow!("Ad-hoc host worker panicked")),
                        duration: std::time::Duration::ZERO,
                    })
                })
                .collect::<Vec<_>>()
        });
        executions.extend(chunk_executions);
    }

    let mut success_count = 0;
    let mut failed_details = Vec::new();

    for (host, execution) in hosts.iter().zip(&executions) {
        match &execution.result {
            Ok(module_result) if !module_result_failed(module_result) => {
                success_count += 1;
                if let Some(payload) = structured_module_output(module_result)? {
                    println!(
                        "{} | {} => {}",
                        host.name.green(),
                        "SUCCESS".green(),
                        payload
                    );
                } else {
                    println!(
                        "{} | {} | rc={} >>>\n{}",
                        host.name.green(),
                        "SUCCESS".green(),
                        module_result_rc(module_result),
                        if !module_result.stdout.trim().is_empty() {
                            module_result.stdout.trim()
                        } else {
                            module_result.msg.as_str()
                        }
                    );
                }
            }
            Ok(module_result) => {
                let detail = if !module_result.stderr.trim().is_empty() {
                    module_result.stderr.trim().to_string()
                } else if !module_result.msg.trim().is_empty() {
                    module_result.msg.trim().to_string()
                } else {
                    format!("Module failed with rc={}", module_result_rc(module_result))
                };
                failed_details.push(format!("{}: {}", host.name, detail));
                if let Some(payload) = structured_module_output(module_result)? {
                    println!("{} | {} => {}", host.name.red(), "FAILED".red(), payload);
                } else {
                    println!(
                        "{} | {} | rc={} >>>\n{}",
                        host.name.red(),
                        "FAILED".red(),
                        module_result_rc(module_result),
                        &detail
                    );
                }
            }
            Err(e) => {
                failed_details.push(format!("{}: {}", host.name, e));
                println!("{} | {} | rc=1 >>>\n{}", host.name.red(), "FAILED".red(), e);
            }
        }

        println!(
            "\n{}\n",
            format!("Execution time: {:.2?}", execution.duration).dimmed()
        );
    }

    println!("\n{}", "PLAY RECAP".bold());
    println!("{}\n", "----------".dimmed());

    for (host, execution) in hosts.iter().zip(&executions) {
        let successful_result = match &execution.result {
            Ok(result) if !module_result_failed(result) => Some(result),
            _ => None,
        };
        println!(
            "{}: {}={} {}={} {}={}",
            host.name.bold(),
            "ok".green(),
            usize::from(successful_result.is_some()),
            "changed".yellow(),
            usize::from(successful_result.is_some_and(|result| result.changed)),
            "failed".red(),
            usize::from(successful_result.is_none())
        );
    }

    println!(
        "\n{}: {}/{}",
        "SUCCESS RATE".bold(),
        success_count.to_string().green(),
        hosts.len()
    );

    if !failed_details.is_empty() {
        return Err(anyhow::anyhow!(
            "Failed to execute on {} host(s): {}",
            failed_details.len(),
            failed_details.join("; ")
        ));
    }

    Ok(())
}

struct HostExecution {
    result: Result<ModuleResult>,
    duration: std::time::Duration,
}

fn prepare_adhoc_args(module_name: &str, args: &str) -> Result<Value> {
    match module_name {
        "command" | "shell" => parse_command_adhoc_args(args),
        "setup" => parse_mapping_adhoc_args(args, true),
        "copy" | "file" | "template" | "service" | "systemd" | "systemd_service" | "package"
        | "debug" | "lineinfile" | "user" | "stat" | "get_url" => {
            parse_mapping_adhoc_args(args, false)
        }
        _ => Err(anyhow::anyhow!("Unsupported module: {}", module_name)),
    }
}

fn parse_command_adhoc_args(args: &str) -> Result<Value> {
    let trimmed = args.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('"') {
        return serde_json::from_str(trimmed).context("Invalid JSON module arguments");
    }
    Ok(Value::String(args.to_string()))
}

fn parse_mapping_adhoc_args(args: &str, allow_empty: bool) -> Result<Value> {
    let trimmed = args.trim();
    if trimmed.is_empty() && allow_empty {
        return Ok(Value::Mapping(Default::default()));
    }
    if trimmed.starts_with('{') {
        let value: Value =
            serde_json::from_str(trimmed).context("Invalid JSON module arguments")?;
        if value.is_mapping() {
            return Ok(value);
        }
        return Err(anyhow::anyhow!("JSON module arguments must be an object"));
    }
    Ok(Value::Mapping(parse_args(args)?))
}

fn effective_adhoc_options(host: &Host, options: &AdHocOptions) -> EffectiveAdHocOptions {
    let use_become = options
        .become_override
        .unwrap_or_else(|| host.get_become().unwrap_or(false));
    let become_user = options
        .become_user
        .as_deref()
        .or_else(|| host.get_become_user().map(String::as_str))
        .filter(|user| !user.is_empty())
        .unwrap_or("root")
        .to_string();

    EffectiveAdHocOptions {
        use_become,
        become_user,
        check_mode: options.check_mode,
    }
}

fn module_result_rc(result: &ModuleResult) -> i32 {
    result.rc.unwrap_or(if result.failed { 1 } else { 0 })
}

fn module_result_failed(result: &ModuleResult) -> bool {
    result.failed || result.rc.is_some_and(|rc| rc != 0)
}

fn structured_module_output(result: &ModuleResult) -> Result<Option<String>> {
    if result.values.is_empty() {
        return Ok(None);
    }

    let mut payload = serde_json::Map::new();
    for (key, value) in &result.values {
        payload.insert(
            key.clone(),
            serde_json::to_value(value)
                .with_context(|| format!("Failed to serialize module result field '{key}'"))?,
        );
    }
    payload.insert("changed".to_string(), result.changed.into());
    payload.insert("failed".to_string(), module_result_failed(result).into());
    if let Some(rc) = result.rc {
        payload.insert("rc".to_string(), rc.into());
    }
    if !result.stdout.is_empty() {
        payload.insert("stdout".to_string(), result.stdout.clone().into());
    }
    if !result.stderr.is_empty() {
        payload.insert("stderr".to_string(), result.stderr.clone().into());
    }
    if !result.msg.is_empty() {
        payload.insert("msg".to_string(), result.msg.clone().into());
    }

    serde_json::to_string_pretty(&payload)
        .context("Failed to encode structured module result")
        .map(Some)
}

fn execute_adhoc_module(
    host: &Host,
    module_name: &str,
    args: &Value,
    options: &EffectiveAdHocOptions,
) -> Result<ModuleResult> {
    // Arbitrary commands have no generally safe change prediction. Return a
    // check-mode result before opening a transport, while still validating the
    // arguments exactly enough to reject an empty command.
    if options.check_mode && matches!(module_name, "command" | "shell") {
        let command = match module_name {
            "command" => command::CommandModule::extract_command_arg(args)?,
            "shell" => shell::ShellModule::extract_command_arg(args)?,
            _ => unreachable!(),
        };
        let command_args = (module_name == "command")
            .then(|| tokenize_args(&command))
            .transpose()?;
        if command.trim().is_empty() || command_args.as_ref().is_some_and(Vec::is_empty) {
            return Err(anyhow::anyhow!(
                "{} module requires a non-empty command",
                module_name
            ));
        }
        if module_name == "shell" && command.contains('\0') {
            return Err(anyhow::anyhow!("Shell command cannot contain NUL bytes"));
        }
        if let Some(command_args) = command_args {
            for argument in command_args {
                file::quote_posix_shell_arg(&argument)?;
            }
        }
        return Ok(ModuleResult {
            stdout: String::new(),
            stderr: String::new(),
            rc: Some(0),
            changed: false,
            failed: false,
            msg: format!(
                "Check mode: {} was not executed because it has no safe change prediction",
                module_name
            ),
            values: Default::default(),
        });
    }

    match module_name {
        "command" => command::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "shell" => shell::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "copy" => copy::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "file" => file::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "template" => template::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "service" => service::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "systemd" | "systemd_service" => service::execute_systemd_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "package" => package::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "debug" => debug::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "lineinfile" => lineinfile::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "user" => user::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "setup" => setup::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "stat" => stat::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        "get_url" => get_url::execute_adhoc(
            host,
            args,
            options.use_become,
            &options.become_user,
            options.check_mode,
        ),
        _ => Err(anyhow::anyhow!("Unsupported module: {}", module_name)),
    }
}

/// Parse command line arguments in format "key1=value1 key2=value2"
fn parse_args(args_str: &str) -> Result<serde_yaml::Mapping> {
    let mut mapping = serde_yaml::Mapping::new();
    for (index, part) in tokenize_args(args_str)?.into_iter().enumerate() {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Argument {} must use key=value format", index + 1))?;
        if key.is_empty() {
            return Err(anyhow::anyhow!("Argument {} has an empty key", index + 1));
        }

        let key = Value::String(key.to_string());
        if mapping.contains_key(&key) {
            return Err(anyhow::anyhow!("Duplicate argument key"));
        }

        mapping.insert(key, infer_arg_value(value));
    }
    Ok(mapping)
}

/// Split a module argument string using the parts of POSIX shell tokenization
/// that are useful for `key=value` input: whitespace separation, adjacent
/// quoted segments, and backslash escaping. No expansion or command execution
/// is performed.
pub(crate) fn tokenize_args(input: &str) -> Result<Vec<String>> {
    #[derive(Clone, Copy)]
    enum Quote {
        Single,
        Double,
    }

    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quote = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some(Quote::Single) => {
                if ch == '\'' {
                    quote = None;
                } else {
                    token.push(ch);
                }
            }
            Some(Quote::Double) => match ch {
                '"' => quote = None,
                '\\' => {
                    let escaped = chars
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Trailing escape in quoted argument"))?;
                    match escaped {
                        '$' | '`' | '"' | '\\' => token.push(escaped),
                        '\n' => {}
                        other => {
                            // POSIX double quotes preserve a backslash unless
                            // it precedes a character that can be escaped.
                            token.push('\\');
                            token.push(other);
                        }
                    }
                }
                _ => token.push(ch),
            },
            None => match ch {
                ch if ch.is_whitespace() => {
                    if token_started {
                        tokens.push(std::mem::take(&mut token));
                        token_started = false;
                    }
                }
                '\'' => {
                    token_started = true;
                    quote = Some(Quote::Single);
                }
                '"' => {
                    token_started = true;
                    quote = Some(Quote::Double);
                }
                '\\' => {
                    token_started = true;
                    let escaped = chars
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Trailing escape in argument list"))?;
                    if escaped != '\n' {
                        token.push(escaped);
                    }
                }
                _ => {
                    token_started = true;
                    token.push(ch);
                }
            },
        }
    }

    if quote.is_some() {
        return Err(anyhow::anyhow!("Unclosed quote in argument list"));
    }
    if token_started {
        tokens.push(token);
    }

    Ok(tokens)
}

fn infer_arg_value(value: &str) -> Value {
    if value.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if value.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }

    let digits = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let is_decimal_integer = !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit());
    let has_significant_leading_zero = digits.len() > 1 && digits.starts_with('0');

    if is_decimal_integer && !has_significant_leading_zero {
        if let Ok(number) = value.parse::<i64>() {
            return Value::Number(serde_yaml::Number::from(number));
        }
    }

    Value::String(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;

    #[test]
    fn test_module_result_default() {
        let result = ModuleResult::default();
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
        assert_eq!(result.rc, None);
        assert!(!result.changed);
        assert_eq!(result.msg, "");
    }

    #[test]
    fn test_parse_args() {
        let args = "key1=value1 key2=value2";
        let mapping = parse_args(args).unwrap();

        assert_eq!(
            mapping.get(Value::String("key1".to_string())),
            Some(&Value::String("value1".to_string()))
        );

        assert_eq!(
            mapping.get(Value::String("key2".to_string())),
            Some(&Value::String("value2".to_string()))
        );

        assert!(parse_args("invalid_format").is_err());
    }

    #[test]
    fn test_tokenize_shell_like_arguments() {
        assert_eq!(
            tokenize_args(r#"msg="hello world" other='a=b' escaped=hello\ world empty="""#)
                .unwrap(),
            vec![
                "msg=hello world",
                "other=a=b",
                "escaped=hello world",
                "empty=",
            ]
        );

        assert_eq!(
            tokenize_args(r#"msg="say \"hello\"" path="C:\tmp""#).unwrap(),
            vec![r#"msg=say "hello""#, r#"path=C:\tmp"#]
        );
    }

    #[test]
    fn test_parse_args_preserves_modes_and_does_not_infer_floats() {
        let mapping = parse_args("mode=0644 count=42 zero=0 negative=-12 ratio=1.5").unwrap();

        assert_eq!(
            mapping.get(Value::String("mode".to_string())),
            Some(&Value::String("0644".to_string()))
        );
        assert_eq!(
            mapping.get(Value::String("count".to_string())),
            Some(&Value::Number(42.into()))
        );
        assert_eq!(
            mapping.get(Value::String("zero".to_string())),
            Some(&Value::Number(0.into()))
        );
        assert_eq!(
            mapping.get(Value::String("negative".to_string())),
            Some(&Value::Number((-12).into()))
        );
        assert_eq!(
            mapping.get(Value::String("ratio".to_string())),
            Some(&Value::String("1.5".to_string()))
        );
    }

    #[test]
    fn test_parse_args_quoted_content_and_empty_value() {
        let mapping =
            parse_args(r#"msg='hello = world' empty="" enabled=TRUE disabled=false"#).unwrap();

        assert_eq!(
            mapping.get(Value::String("msg".to_string())),
            Some(&Value::String("hello = world".to_string()))
        );
        assert_eq!(
            mapping.get(Value::String("empty".to_string())),
            Some(&Value::String(String::new()))
        );
        assert_eq!(
            mapping.get(Value::String("enabled".to_string())),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            mapping.get(Value::String("disabled".to_string())),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn test_parse_args_rejects_duplicate_and_malformed_arguments() {
        for invalid in [
            "key=one key=two",
            "missing_equals",
            "=empty-key",
            "key='unclosed",
            "key=\"unclosed",
            "key=value\\",
        ] {
            assert!(
                parse_args(invalid).is_err(),
                "accepted invalid argument list"
            );
        }
    }

    #[test]
    fn ad_hoc_accepts_json_and_setup_without_arguments() {
        let args = prepare_adhoc_args(
            "get_url",
            r#"{"url":"https://example.invalid/a","dest":"/tmp/a","force":true}"#,
        )
        .unwrap();
        let mapping = args.as_mapping().unwrap();
        assert_eq!(
            mapping.get(Value::String("force".to_string())),
            Some(&Value::Bool(true))
        );

        assert_eq!(
            prepare_adhoc_args("command", r#"{"cmd":"printf ok"}"#).unwrap(),
            serde_yaml::from_str::<Value>("cmd: printf ok").unwrap()
        );
        assert_eq!(
            prepare_adhoc_args("command", r#""printf ok""#).unwrap(),
            Value::String("printf ok".to_string())
        );
        assert_eq!(
            prepare_adhoc_args("setup", "").unwrap(),
            Value::Mapping(Default::default())
        );
    }

    #[test]
    fn ad_hoc_json_rejects_invalid_or_non_object_mapped_arguments() {
        for (module, args) in [
            ("stat", "{broken"),
            ("stat", r#"["not", "an", "object"]"#),
            ("command", "{broken"),
        ] {
            assert!(
                prepare_adhoc_args(module, args).is_err(),
                "accepted invalid JSON for {module}"
            );
        }
    }

    #[test]
    fn all_supported_structured_ad_hoc_modules_reach_argument_validation() {
        for module in [
            "copy",
            "file",
            "template",
            "service",
            "systemd",
            "systemd_service",
            "package",
            "debug",
            "lineinfile",
            "user",
            "stat",
            "get_url",
        ] {
            assert!(
                prepare_adhoc_args(module, "key=value").is_ok(),
                "{module} was not accepted"
            );
        }
    }

    #[test]
    fn structured_ad_hoc_output_contains_common_and_module_fields() {
        let result = ModuleResult {
            stdout: "payload".to_string(),
            stderr: "warning".to_string(),
            rc: Some(7),
            changed: true,
            failed: false,
            msg: "download failed".to_string(),
            values: HashMap::from([(
                "stat".to_string(),
                serde_yaml::from_str("{exists: true}").unwrap(),
            )]),
        };
        let output = structured_module_output(&result).unwrap().unwrap();
        let payload: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(payload["changed"], true);
        assert_eq!(payload["failed"], true);
        assert_eq!(payload["rc"], 7);
        assert_eq!(payload["stdout"], "payload");
        assert_eq!(payload["stderr"], "warning");
        assert_eq!(payload["msg"], "download failed");
        assert_eq!(payload["stat"]["exists"], true);
        assert!(structured_module_output(&ModuleResult::default())
            .unwrap()
            .is_none());
    }

    #[test]
    fn setup_and_stat_execute_through_ad_hoc_dispatch() {
        let mut host = Host::new("localhost");
        host.set_variable("ansible_connection", "local");
        let options = EffectiveAdHocOptions {
            use_become: false,
            become_user: "root".to_string(),
            check_mode: false,
        };

        let facts = execute_adhoc_module(
            &host,
            "setup",
            &Value::Mapping(Default::default()),
            &options,
        )
        .unwrap();
        assert!(facts.values.contains_key("ansible_facts"));

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing");
        let args = serde_yaml::from_str(&format!("path: {}", path.display())).unwrap();
        let stat = execute_adhoc_module(&host, "stat", &args, &options).unwrap();
        assert_eq!(
            stat.values["stat"]
                .as_mapping()
                .unwrap()
                .get(Value::String("exists".to_string())),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn new_ad_hoc_dispatch_routes_validate_module_specific_arguments() {
        let mut host = Host::new("localhost");
        host.set_variable("ansible_connection", "local");
        let options = EffectiveAdHocOptions {
            use_become: false,
            become_user: "root".to_string(),
            check_mode: true,
        };
        let empty = Value::Mapping(Default::default());

        let stat_error = execute_adhoc_module(&host, "stat", &empty, &options).unwrap_err();
        assert!(stat_error.to_string().contains("path"));
        let get_url_error = execute_adhoc_module(&host, "get_url", &empty, &options).unwrap_err();
        assert!(get_url_error.to_string().contains("url"));
        for module in ["systemd", "systemd_service"] {
            let error = execute_adhoc_module(&host, module, &empty, &options).unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains("systemd") || message.contains("name"),
                "unexpected {module} error: {message}"
            );
        }
    }

    #[test]
    fn test_run_adhoc_rejects_unknown_module_before_connecting() {
        let host = Host::new("unreachable.invalid");
        let error = run_adhoc(&[host], "not_a_module", "key=value").unwrap_err();
        assert!(error.to_string().contains("Unsupported module"));
    }

    #[test]
    fn test_run_adhoc_rejects_zero_hosts() {
        let error = run_adhoc(&[], "debug", "msg=test").unwrap_err();
        assert!(error.to_string().contains("without any target hosts"));
    }

    #[test]
    fn ad_hoc_options_inherit_inventory_and_explicit_values_win() {
        let mut host = Host::new("localhost");
        host.set_variable("ansible_become", "true");
        host.set_variable("ansible_become_user", "inventory-user");

        assert_eq!(
            effective_adhoc_options(&host, &AdHocOptions::default()),
            EffectiveAdHocOptions {
                use_become: true,
                become_user: "inventory-user".to_string(),
                check_mode: false,
            }
        );

        let explicit = AdHocOptions {
            become_override: Some(false),
            become_user: Some("cli-user".to_string()),
            check_mode: true,
            forks: 3,
        };
        assert_eq!(
            effective_adhoc_options(&host, &explicit),
            EffectiveAdHocOptions {
                use_become: false,
                become_user: "cli-user".to_string(),
                check_mode: true,
            }
        );
    }

    #[test]
    fn ad_hoc_rejects_invalid_forks_and_become_user_before_execution() {
        let host = Host::new("unreachable.invalid");
        let zero_forks = AdHocOptions {
            forks: 0,
            ..AdHocOptions::default()
        };
        assert!(run_adhoc_with_options(
            std::slice::from_ref(&host),
            "debug",
            "msg=test",
            &zero_forks
        )
        .unwrap_err()
        .to_string()
        .contains("greater than zero"));

        let invalid_user = AdHocOptions {
            become_user: Some("bad\nuser".to_string()),
            ..AdHocOptions::default()
        };
        assert!(
            run_adhoc_with_options(&[host], "debug", "msg=test", &invalid_user)
                .unwrap_err()
                .to_string()
                .contains("--become-user")
        );
    }

    #[test]
    fn ad_hoc_check_mode_does_not_execute_command_or_shell() {
        let temp = tempfile::tempdir().unwrap();
        let command_marker = temp.path().join("command-marker");
        let shell_marker = temp.path().join("shell-marker");
        let host = Host::new("localhost");
        let options = AdHocOptions {
            check_mode: true,
            forks: 1,
            ..AdHocOptions::default()
        };

        run_adhoc_with_options(
            std::slice::from_ref(&host),
            "command",
            &format!("touch {}", command_marker.display()),
            &options,
        )
        .unwrap();
        run_adhoc_with_options(
            &[host],
            "shell",
            &format!("touch {}", shell_marker.display()),
            &options,
        )
        .unwrap();
        run_adhoc_with_options(
            &[Host::new("unreachable.invalid")],
            "command",
            "true",
            &options,
        )
        .unwrap();

        assert!(!command_marker.exists());
        assert!(!shell_marker.exists());
    }

    #[test]
    fn ad_hoc_rc_uses_module_result_value_and_safe_fallbacks() {
        let nonzero_rc = ModuleResult {
            rc: Some(23),
            ..ModuleResult::default()
        };
        assert_eq!(module_result_rc(&nonzero_rc), 23);
        assert!(module_result_failed(&nonzero_rc));
        assert_eq!(
            module_result_rc(&ModuleResult {
                failed: true,
                ..ModuleResult::default()
            }),
            1
        );
        assert_eq!(module_result_rc(&ModuleResult::default()), 0);
    }

    #[test]
    fn test_adhoc_failed_module_result_is_an_error() {
        let host = Host::new("localhost");
        let result = run_adhoc(&[host], "command", "false");
        assert!(result.is_err());
    }
}
