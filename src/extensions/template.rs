use super::*;
use minijinja::{
    value::{from_args, Kwargs, Rest, Serde, Value as JinjaValue},
    Environment, Error, ErrorKind,
};
use serde_yaml::{Mapping, Value};
fn kwargs(values: &Kwargs) -> std::result::Result<Value, Error> {
    let mut mapping = Mapping::new();
    for key in values.args() {
        let value: JinjaValue = values.get(key)?;
        mapping.insert(
            Value::String(key.into()),
            serde_yaml::to_value(value).map_err(|_| {
                Error::new(
                    ErrorKind::InvalidOperation,
                    "Cannot encode native keyword argument",
                )
            })?,
        );
    }
    values.assert_all_used()?;
    Ok(Value::Mapping(mapping))
}
fn execute(
    registry: &Registry,
    name: &str,
    kind: Kind,
    args: Value,
) -> std::result::Result<JinjaValue, Error> {
    let result = registry.invoke(name, kind, args).map_err(|_| {
        Error::new(
            ErrorKind::InvalidOperation,
            format!("Native {kind:?} provider {name} failed"),
        )
    })?;
    let value = result.values.get("value").ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidOperation,
            "Native provider omitted its value",
        )
    })?;
    Ok(JinjaValue::from(Serde(value)))
}
pub(crate) fn register(environment: &mut Environment<'static>) {
    let registry = Registry::current();
    let lookup_registry = registry.clone();
    environment.add_function(
        "lookup",
        move |name: String, args: Rest<JinjaValue>| -> std::result::Result<JinjaValue, Error> {
            let (args, keywords) = from_args::<(&[JinjaValue], Kwargs)>(&args)?;
            let terms = serde_yaml::to_value(args).map_err(|_| {
                Error::new(
                    ErrorKind::InvalidOperation,
                    "Cannot encode native lookup terms",
                )
            })?;
            execute(
                &lookup_registry,
                &name,
                Kind::Lookup,
                Value::Mapping(Mapping::from_iter([
                    (Value::String("terms".into()), terms),
                    (Value::String("keywords".into()), kwargs(&keywords)?),
                ])),
            )
        },
    );
    for provider in registry.filters() {
        let name = provider.definition.name.clone();
        let callback_name = name.clone();
        let registry = registry.clone();
        environment.add_filter(
            name,
            move |value: JinjaValue,
                  args: Rest<JinjaValue>|
                  -> std::result::Result<JinjaValue, Error> {
                let (args, keywords) = from_args::<(&[JinjaValue], Kwargs)>(&args)?;
                let subject = serde_yaml::to_value(value).map_err(|_| {
                    Error::new(
                        ErrorKind::InvalidOperation,
                        "Cannot encode native filter subject",
                    )
                })?;
                let arguments = serde_yaml::to_value(args).map_err(|_| {
                    Error::new(
                        ErrorKind::InvalidOperation,
                        "Cannot encode native filter arguments",
                    )
                })?;
                execute(
                    &registry,
                    &callback_name,
                    Kind::Filter,
                    Value::Mapping(Mapping::from_iter([
                        (Value::String("value".into()), subject),
                        (Value::String("arguments".into()), arguments),
                        (Value::String("keywords".into()), kwargs(&keywords)?),
                    ])),
                )
            },
        );
    }
}
