use anyhow::{bail, Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;

use super::ModuleResult;
use crate::ssh::config::ConnectionConfig;

pub use rustsible_protocol::registry::{Builtin, CheckSupport};

pub struct Invocation {
    pub module: Builtin,
    args: Value,
    check: bool,
}

impl Invocation {
    pub fn prepare(name: &str, args: &Value, check: bool) -> Result<Self> {
        let module = Builtin::lookup(name)?;
        let args = if matches!(module, Builtin::Command | Builtin::Shell) {
            if let Some(raw) = args.get("_raw_params") {
                if args.as_mapping().is_none_or(|map| map.len() != 1) {
                    bail!("Raw command cannot be combined with other parameters");
                }
                raw.clone()
            } else {
                args.clone()
            }
        } else {
            args.clone()
        };
        module.validate_capabilities(check, false, false)?;
        module.validate_parameters(&args)?;
        Ok(Self {
            module,
            args,
            check,
        })
    }
    pub fn needs_connection(&self) -> bool {
        self.module != Builtin::Debug
            && !(self.check && self.module.check_support() == CheckSupport::Skip)
    }
    pub(crate) fn execute(
        mut self,
        config: Option<&ConnectionConfig>,
        vars: &HashMap<String, Value>,
        runtime: &crate::worker::Runtime,
    ) -> ModuleResult {
        self.check |= super::result::is_check_active();
        let _changes = super::result::ChangeScope::enter(self.check);
        if self.check && self.module.check_support() == CheckSupport::Skip {
            return ModuleResult {
                skipped: true,
                msg: format!(
                    "Check mode: {} was not executed because it has no safe change prediction",
                    self.module.name()
                ),
                ..ModuleResult::default()
            };
        }
        let result = (|| {
            if self.module == Builtin::Debug {
                return super::debug::execute_without_connection(&self.args);
            }
            let config = config.context("Target module needs a connection configuration")?;
            if matches!(self.module, Builtin::Copy | Builtin::Template) {
                let mut args = self.args.clone();
                let mut source = if self.module == Builtin::Template {
                    crate::worker::Source::bytes(super::template::render(&args, vars)?.into_bytes())
                } else if let Some(src) = args.get("src").and_then(Value::as_str) {
                    crate::worker::Source::file(src)?
                } else {
                    crate::worker::Source::bytes(
                        args.get("content")
                            .and_then(Value::as_str)
                            .context("Copy requires string content")?
                            .as_bytes()
                            .to_vec(),
                    )
                };
                if let Some(mapping) = args.as_mapping_mut() {
                    for name in ["src", "content", "vars"] {
                        mapping.remove(Value::String(name.into()));
                    }
                }
                return runtime.execute_with_source(
                    config,
                    self.module.name(),
                    args,
                    self.check,
                    Some(&mut source),
                );
            }
            runtime.execute(config, self.module.name(), self.args.clone(), self.check)
        })();
        match result {
            Ok(mut result) => {
                result.failed |= result.rc.is_some_and(|rc| rc != 0);
                result.changed |= super::result::known_changed();
                result
            }
            Err(error) => crate::modules::result::from_error(
                error,
                self.module.check_support() != CheckSupport::ReadOnly && !self.check,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_capabilities_have_one_contract() {
        for module in Builtin::ALL {
            for name in [
                module.name().to_string(),
                format!("ansible.builtin.{}", module.name()),
                format!("ansible.legacy.{}", module.name()),
            ] {
                assert_eq!(Builtin::lookup(&name).unwrap(), module);
            }
            assert!(module.validate_capabilities(true, false, false).is_ok());
            assert!(module.validate_capabilities(false, true, false).is_err());
            assert!(module.validate_capabilities(false, false, true).is_err());
        }
        assert_eq!(
            Builtin::lookup("ansible.builtin.systemd_service").unwrap(),
            Builtin::Systemd
        );
        assert!(Builtin::lookup("custom.python_module").is_err());
    }
    #[test]
    fn invalid_parameters_are_rejected_without_opening_a_connection() {
        for (module, yaml) in [
            ("copy", "content: x\ndest: /tmp/x\nunknown: true"),
            ("systemd", "name: demo\nstate: invalid\ndaemon_reload: true"),
            ("systemd", "daemon_reload: true\nenabled: true"),
            ("command", "cmd: echo x\nunknown: x"),
            ("file", "path: /tmp/x\nmode: '888'"),
            ("user", "name: x\nuid: -1"),
            (
                "lineinfile",
                "path: /tmp/x\nline: x\ninsertafter: EOF\ninsertbefore: BOF",
            ),
        ] {
            let args = serde_yaml::from_str(yaml).unwrap();
            for check in [false, true] {
                assert!(
                    Invocation::prepare(module, &args, check).is_err(),
                    "{module}"
                );
            }
        }
        let invocation =
            Invocation::prepare("shell", &Value::String("echo x".into()), true).unwrap();
        assert!(!invocation.needs_connection());
        assert_eq!(
            invocation
                .execute(None, &HashMap::new(), &crate::worker::Runtime::default())
                .outcome(),
            super::super::Outcome::Skipped
        );
    }
}
