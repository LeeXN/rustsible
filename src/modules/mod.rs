pub mod debug;
pub mod param;
pub mod registry;
pub(crate) mod result;
pub use result::{ExecutionFailure, Outcome};
pub mod template;

use anyhow::{Context, Result};
use colored::Colorize;
use log::info;
use serde_yaml::Value;
use std::collections::HashMap;
use std::time::Instant;

use crate::inventory::Host;

pub use rustsible_protocol::ModuleResult;

/// Controls an ad-hoc run. `become_override` is optional so an explicit
/// command-line request can override inventory while an omitted flag still
/// inherits the per-host `ansible_become` setting.
#[derive(Debug, Clone)]
pub struct AdHocOptions {
    pub become_override: Option<bool>,
    pub become_user: Option<String>,
    pub check_mode: bool,
    pub forks: usize,
    pub worker_dir: Option<std::path::PathBuf>,
}

impl Default for AdHocOptions {
    fn default() -> Self {
        Self {
            become_override: None,
            become_user: None,
            check_mode: false,
            forks: 5,
            worker_dir: None,
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

    let module_name = registry::Builtin::lookup(module_name)?.name();
    // Validate the module and parse mapped arguments before starting any
    // connections. Besides failing fast, this deliberately parses the command
    // line only once rather than once per host.
    let module_args = prepare_adhoc_args(module_name, args)?;
    registry::Invocation::prepare(module_name, &module_args, options.check_mode)?;

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
    let runtime = crate::worker::Runtime::new(options.worker_dir.clone());
    let mut executions = Vec::with_capacity(hosts.len());
    for chunk in hosts.chunks(options.forks) {
        let chunk_executions = std::thread::scope(|scope| {
            let module_args = &module_args;
            let runtime = &runtime;
            let handles = chunk
                .iter()
                .map(|host| {
                    scope.spawn(move || {
                        info!("Running module {} on host {}", module_name, host.name);
                        let start_time = Instant::now();
                        let result = execute_adhoc_with_runtime(
                            host,
                            module_name,
                            module_args,
                            options,
                            runtime,
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
                        if module_result.skipped {
                            "SKIPPED".yellow()
                        } else {
                            "SUCCESS".green()
                        },
                        payload
                    );
                } else {
                    println!(
                        "{} | {} | rc={} >>>\n{}",
                        host.name.green(),
                        if module_result.skipped {
                            "SKIPPED".yellow()
                        } else {
                            "SUCCESS".green()
                        },
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
                    println!(
                        "{} | {} => {}",
                        host.name.red(),
                        module_result.outcome().as_str().to_uppercase().red(),
                        payload
                    );
                } else {
                    println!(
                        "{} | {} | rc={} >>>\n{}",
                        host.name.red(),
                        module_result.outcome().as_str().to_uppercase().red(),
                        module_result_rc(module_result),
                        detail
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
        let outcome = execution
            .result
            .as_ref()
            .map(ModuleResult::outcome)
            .unwrap_or(Outcome::Failed);
        println!(
            "{}: ok={} changed={} failed={} unreachable={} uncertain={} skipped={}",
            host.name.bold(),
            usize::from(outcome == Outcome::Ok),
            usize::from(successful_result.is_some_and(|result| result.changed)),
            usize::from(outcome == Outcome::Failed),
            usize::from(outcome == Outcome::Unreachable),
            usize::from(outcome == Outcome::Uncertain),
            usize::from(outcome == Outcome::Skipped)
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
    let module = registry::Builtin::lookup(module_name)?;
    match module {
        registry::Builtin::Command | registry::Builtin::Shell => parse_command_adhoc_args(args),
        _ => parse_mapping_adhoc_args(args, module == registry::Builtin::Setup),
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

fn adhoc_config(
    host: &Host,
    options: &AdHocOptions,
) -> Result<crate::ssh::config::ConnectionConfig> {
    let mut overrides = HashMap::new();
    if let Some(value) = options.become_override {
        overrides.insert("ansible_become".into(), Value::Bool(value));
    }
    if let Some(value) = &options.become_user {
        overrides.insert("ansible_become_user".into(), Value::String(value.clone()));
    }
    crate::ssh::config::ConnectionConfig::from_host_with_overrides(host, &overrides)
}

fn module_result_rc(result: &ModuleResult) -> i32 {
    result.rc.unwrap_or(if result.failed { 1 } else { 0 })
}

fn module_result_failed(result: &ModuleResult) -> bool {
    matches!(
        result.outcome(),
        Outcome::Failed | Outcome::Unreachable | Outcome::Uncertain
    )
}

fn structured_module_output(result: &ModuleResult) -> Result<Option<String>> {
    if result.values.is_empty() && !result.skipped && !result.unreachable && !result.uncertain {
        return Ok(None);
    }

    serde_json::to_string_pretty(&result.as_value())
        .context("Failed to encode structured module result")
        .map(Some)
}

#[cfg(test)]
fn execute_adhoc_module(
    host: &Host,
    module_name: &str,
    args: &Value,
    options: &AdHocOptions,
) -> Result<ModuleResult> {
    execute_adhoc_with_runtime(
        host,
        module_name,
        args,
        options,
        &crate::worker::Runtime::new(options.worker_dir.clone()),
    )
}

fn execute_adhoc_with_runtime(
    host: &Host,
    module_name: &str,
    args: &Value,
    options: &AdHocOptions,
    runtime: &crate::worker::Runtime,
) -> Result<ModuleResult> {
    let invocation = registry::Invocation::prepare(module_name, args, options.check_mode)?;
    let config = invocation
        .needs_connection()
        .then(|| adhoc_config(host, options))
        .transpose()?;
    Ok(invocation.execute(config.as_ref(), &HashMap::new(), runtime))
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

pub(crate) use rustsible_protocol::tokenize_args;

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
            skipped: false,
            unreachable: false,
            uncertain: false,
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
        let options = AdHocOptions {
            become_override: None,
            become_user: None,
            check_mode: false,
            forks: 5,
            ..AdHocOptions::default()
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
        let options = AdHocOptions {
            become_override: None,
            become_user: None,
            check_mode: true,
            forks: 5,
            ..AdHocOptions::default()
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

        let inherited = adhoc_config(&host, &AdHocOptions::default()).unwrap();
        assert!(inherited.use_become);
        assert_eq!(inherited.become_user, "inventory-user");

        let explicit = AdHocOptions {
            become_override: Some(false),
            become_user: Some("cli-user".to_string()),
            check_mode: true,
            forks: 3,
            ..AdHocOptions::default()
        };
        let config = adhoc_config(&host, &explicit).unwrap();
        assert!(!config.use_become);
        assert_eq!(config.become_user, "cli-user");
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
                skipped: false,
                unreachable: false,
                uncertain: false,
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
#[test]
fn adhoc_uses_resolved_inventory_connection_variables() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("resolved");
    let mut host = Host::new("must-not-connect.invalid");
    host.set_variable("transport", "local");
    host.set_variable("ansible_connection", "{{ transport }}");
    let args = serde_json::json!({"content": "resolved", "dest": destination}).to_string();
    run_adhoc_with_options(&[host], "copy", &args, &AdHocOptions::default()).unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"resolved");
}
