//! Native inventory output is data, not another template-definition source.
//! A scalar tag keeps that provenance inside the existing typed variable
//! layers without adding parallel host tables or guessing string types.
use anyhow::Result;
use serde_yaml::{
    value::{Tag, TaggedValue},
    Value,
};
pub(crate) fn literal(value: &Value) -> Option<&Value> {
    if let Value::Tagged(tagged) = value {
        if tagged.tag == "rustsible_data" && tagged.value.is_string() {
            return Some(&tagged.value);
        }
    }
    None
}
pub(crate) fn native(value: Value) -> Value {
    match value {
        Value::String(_) => Value::Tagged(Box::new(TaggedValue {
            tag: Tag::new("rustsible_data"),
            value,
        })),
        Value::Mapping(values) => Value::Mapping(
            values
                .into_iter()
                .map(|(key, value)| (key, native(value)))
                .collect(),
        ),
        Value::Sequence(values) => Value::Sequence(values.into_iter().map(native).collect()),
        _ => value,
    }
}
pub(crate) fn materialize(value: &Value) -> Result<Value> {
    if let Some(literal) = literal(value) {
        return Ok(literal.clone());
    }
    if crate::vault::is_value(value) {
        return crate::vault::Vault::current().materialize(value);
    }
    match value {
        Value::Mapping(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), materialize(value)?)))
            .collect::<Result<serde_yaml::Mapping>>()
            .map(Value::Mapping),
        Value::Sequence(values) => values
            .iter()
            .map(materialize)
            .collect::<Result<Vec<_>>>()
            .map(Value::Sequence),
        _ => Ok(value.clone()),
    }
}
