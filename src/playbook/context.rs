//! YAML containers stay lazy in the template context. A !vault scalar becomes
//! an ordinary string only when accessed; its plaintext is never compiled as
//! another template. A separate error flag prevents failed decryption from
//! being mistaken for an undefined variable by tests/default filters.
use anyhow::{bail, Result};
use minijinja::value::{Enumerator, Object, ObjectRepr, Value as JinjaValue};
use serde_yaml::Value;
use std::{
    collections::HashMap,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[derive(Clone)]
pub(crate) struct TemplateContext {
    value: JinjaValue,
    failed: Arc<AtomicBool>,
    vault: Arc<crate::vault::Vault>,
}
impl TemplateContext {
    pub fn new(vars: &HashMap<String, Value>) -> Result<Self> {
        let vault = crate::vault::Vault::current();
        for value in vars.values() {
            validate(value, 0)?;
        }
        let mut mapping = vars
            .iter()
            .map(|(key, value)| (Value::String(key.clone()), value.clone()))
            .collect::<serde_yaml::Mapping>();
        mapping.insert(
            Value::String("omit".into()),
            Value::String(super::templar::OMIT_SENTINEL.into()),
        );
        let failed = Arc::new(AtomicBool::new(false));
        let value = JinjaValue::from_object(YamlObject {
            data: Value::Mapping(mapping),
            vault: vault.clone(),
            failed: failed.clone(),
        });
        Ok(Self {
            value,
            failed,
            vault,
        })
    }
    pub(crate) fn value(&self) -> JinjaValue {
        self.value.clone()
    }
    pub(crate) fn checked<T>(&self, result: Result<T>) -> Result<T> {
        if self.failed.load(Ordering::Relaxed) {
            bail!("Vault value could not be decrypted");
        }
        self.vault.protect(result)
    }
    pub(crate) fn render_named(
        &self,
        environment: &minijinja::Environment<'_>,
        name: &str,
        source: &str,
    ) -> Result<String> {
        self.checked(
            environment
                .render_named_str(name, source, self.value())
                .map_err(Into::into),
        )
    }
}
impl fmt::Debug for TemplateContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TemplateContext(<values redacted>)")
    }
}
struct YamlObject {
    data: Value,
    vault: Arc<crate::vault::Vault>,
    failed: Arc<AtomicBool>,
}
impl fmt::Debug for YamlObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TemplateValues(<values redacted>)")
    }
}
impl YamlObject {
    fn convert(&self, value: &Value) -> JinjaValue {
        if let Some(value) = crate::data::literal(value) {
            return jinja(value);
        }
        if crate::vault::is_value(value) {
            return match self.vault.materialize(value) {
                Ok(value) => jinja(&value),
                Err(_) => {
                    self.failed.store(true, Ordering::Relaxed);
                    JinjaValue::UNDEFINED
                }
            };
        }
        if matches!(value, Value::Mapping(_) | Value::Sequence(_)) {
            JinjaValue::from_object(Self {
                data: value.clone(),
                vault: self.vault.clone(),
                failed: self.failed.clone(),
            })
        } else {
            jinja(value)
        }
    }
}
impl Object for YamlObject {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        if self.data.is_sequence() {
            ObjectRepr::Seq
        } else {
            ObjectRepr::Map
        }
    }
    fn get_value(self: &Arc<Self>, key: &JinjaValue) -> Option<JinjaValue> {
        match &self.data {
            Value::Mapping(values) => values
                .get(serde_yaml::to_value(key).ok()?)
                .map(|value| self.convert(value)),
            Value::Sequence(values) => values.get(key.as_usize()?).map(|value| self.convert(value)),
            _ => None,
        }
    }
    fn enumerate(self: &Arc<Self>) -> Enumerator {
        match &self.data {
            Value::Mapping(values) => Enumerator::Values(values.keys().map(jinja).collect()),
            Value::Sequence(values) => Enumerator::Seq(values.len()),
            _ => Enumerator::Empty,
        }
    }
}
fn validate(value: &Value, depth: usize) -> Result<()> {
    if depth > 64 {
        bail!("Template context nesting exceeds 64");
    }
    match value {
        Value::Mapping(values) => {
            for (key, value) in values {
                if !matches!(
                    key,
                    Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                ) {
                    bail!("Template mapping keys must be scalar values");
                }
                validate(value, depth + 1)?;
            }
        }
        Value::Sequence(values) => {
            for value in values {
                validate(value, depth + 1)?;
            }
        }
        Value::Tagged(_)
            if !crate::vault::is_value(value) && crate::data::literal(value).is_none() =>
        {
            bail!("Unsupported YAML tag in template variables")
        }
        _ => {}
    }
    Ok(())
}

fn jinja(value: &Value) -> JinjaValue {
    JinjaValue::from(minijinja::value::Serde(value))
}
