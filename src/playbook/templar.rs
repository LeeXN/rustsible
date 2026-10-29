use anyhow::{bail, Context, Result};
use minijinja::value::Serde;
use minijinja::{Environment, UndefinedBehavior};
use serde_yaml::Value;
use std::collections::HashMap;

/// Runtime values retain provenance so facts can sit below play/task vars
/// while registered results remain above them, without templating either.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeVariables {
    pub facts: HashMap<String, Value>,
    pub defaults: std::sync::Arc<HashMap<String, Value>>,
    pub registered: HashMap<String, Value>,
    pub included: HashMap<String, Value>,
    pub extra: std::sync::Arc<HashMap<String, Value>>,
    pub globals: std::sync::Arc<HashMap<String, Value>>,
    pub magic: HashMap<String, Value>,
}

impl RuntimeVariables {
    /// Keep precedence in one place for task arguments, vars_files and
    /// hostvars. Only definitions are rendered; facts/register values are data.
    pub(crate) fn layers(
        &self,
        definitions: &HashMap<String, Value>,
        overlays: &[&serde_yaml::Mapping],
        keywords: Option<&HashMap<String, Value>>,
    ) -> Result<(HashMap<String, Value>, HashMap<String, Value>)> {
        let mut baseline = HashMap::new();
        if let Some(keywords) = keywords {
            crate::inventory::host::merge_variable_layer(&mut baseline, keywords);
        }
        crate::inventory::host::merge_variable_layer(&mut baseline, definitions);
        let mut definitions = baseline;
        let mut higher = HashMap::new();
        for overlay in overlays {
            let layer = overlay
                .iter()
                .map(|(name, value)| {
                    Ok((
                        name.as_str()
                            .context("Variable names must be strings")?
                            .to_owned(),
                        value.clone(),
                    ))
                })
                .collect::<Result<HashMap<_, _>>>()?;
            crate::inventory::host::merge_variable_layer(&mut higher, &layer);
        }
        crate::inventory::host::merge_variable_layer(&mut higher, &self.included);
        // Configuration strings are literal data. Only inventory/play/task
        // declarations are template definitions, even when a path has braces.
        let mut data = (*self.defaults).clone();
        for name in definitions.keys() {
            data.remove(name);
        }
        data.extend(self.facts.clone());
        for name in higher.keys() {
            data.remove(name);
        }
        data.extend(self.registered.clone());
        definitions.extend(higher);
        crate::inventory::host::merge_variable_layer(&mut definitions, &self.extra);
        for name in self.extra.keys() {
            data.remove(name);
        }
        data.extend((*self.globals).clone());
        data.extend(self.magic.clone());
        Ok((definitions, data))
    }
}

pub(crate) type TemplateContext = HashMap<String, serde_json::Value>;
pub(crate) const OMIT_SENTINEL: &str = "__rustsible_internal_omit_4f5e7d9b__";

pub(crate) fn environment() -> Environment<'static> {
    let mut environment = Environment::new();
    environment.set_syntax(
        minijinja::syntax::SyntaxConfig::builder()
            .keep_trailing_newline(true)
            .trim_blocks(true)
            .build()
            .expect("Default delimiters form valid template syntax"),
    );
    environment.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    environment.set_undefined_behavior(UndefinedBehavior::Strict);
    environment.set_fuel(Some(1_000_000));
    crate::playbook::filters::register_ansible_filters(&mut environment);
    environment
}

/// An expression returns its native value; a text template returns text.
/// Never feed rendered output back into the engine: it may be untrusted command
/// output, JSON or a literal containing template delimiters.
pub(crate) fn render_value(
    input: &str,
    environment: &mut Environment<'_>,
    context: &TemplateContext,
    force_string: bool,
) -> Result<Value> {
    let source = input;
    if !force_string && source.starts_with("{{") && source.ends_with("}}") {
        // Use the engine's AST instead of guessing where quotes or braces end.
        // The standalone lexer in 3.0.4 panics on mixed templates. Pin the
        // dependency because this AST API is unstable. Expression spans also
        // handle Jinja whitespace-control delimiters without text rewriting.
        let ast = minijinja::machinery::parse(source, "<argument>", environment.syntax().clone())?;
        if let minijinja::machinery::ast::Stmt::Template(template) = ast {
            if let [minijinja::machinery::ast::Stmt::EmitExpr(_)] = template.children.as_slice() {
                let tokens =
                    minijinja::machinery::tokenize(source, false, environment.syntax().clone())
                        .collect::<Result<Vec<_>, _>>()?;
                let start = tokens
                    .first()
                    .context("Missing expression start")?
                    .1
                    .end_offset as usize;
                let end = tokens
                    .last()
                    .context("Missing expression end")?
                    .1
                    .start_offset as usize;
                let expression =
                    environment.compile_expression_owned(source[start..end].to_owned())?;
                let result = expression.eval(Serde(context))?;
                if result.is_undefined() {
                    return Err(minijinja::Error::new(
                        minijinja::ErrorKind::UndefinedError,
                        "Template expression returned an undefined variable",
                    )
                    .into());
                }
                return serde_yaml::to_value(result)
                    .context("Cannot represent template result as YAML");
            }
        }
    }
    Ok(Value::String(
        environment.render_str(input, Serde(context))?,
    ))
}

/// Resolve only definitions, never runtime data. Missing dependencies remain
/// undefined until used, so gathering facts does not eagerly require a play
/// variable that itself depends on those facts. Syntax errors and cycles still
/// fail before any module is invoked.
pub(crate) fn resolve_definitions(
    definitions: &HashMap<String, Value>,
    data: &HashMap<String, Value>,
) -> Result<HashMap<String, Value>> {
    fn references(
        value: &Value,
        environment: &Environment<'_>,
        names: &mut Vec<String>,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 {
            bail!("Variable definitions exceed maximum nesting depth of 64");
        }
        match value {
            Value::String(source) => names.extend(
                environment
                    .template_from_str(source)?
                    .undeclared_variables(false),
            ),
            Value::Sequence(values) => {
                for value in values {
                    references(value, environment, names, depth + 1)?;
                }
            }
            Value::Mapping(values) => {
                for value in values.values() {
                    references(value, environment, names, depth + 1)?;
                }
            }
            Value::Tagged(value) => references(&value.value, environment, names, depth + 1)?,
            _ => {}
        }
        Ok(())
    }
    fn render(
        value: &Value,
        environment: &mut Environment<'_>,
        context: &TemplateContext,
        depth: usize,
    ) -> Result<Value> {
        if depth > 64 {
            bail!("Variable definitions exceed maximum nesting depth of 64");
        }
        match value {
            Value::String(source) => render_value(source, environment, context, false),
            Value::Sequence(values) => values
                .iter()
                .map(|value| render(value, environment, context, depth + 1))
                .collect::<Result<Vec<_>>>()
                .map(Value::Sequence),
            Value::Mapping(values) => values
                .iter()
                .map(|(key, value)| {
                    Ok((key.clone(), render(value, environment, context, depth + 1)?))
                })
                .collect::<Result<serde_yaml::Mapping>>()
                .map(Value::Mapping),
            Value::Tagged(value) => {
                let mut value = (**value).clone();
                value.value = render(&value.value, environment, context, depth + 1)?;
                Ok(Value::Tagged(Box::new(value)))
            }
            _ => Ok(value.clone()),
        }
    }
    fn visit(
        name: &str,
        definitions: &HashMap<String, Value>,
        resolved: &mut HashMap<String, Value>,
        done: &mut std::collections::HashSet<String>,
        stack: &mut Vec<String>,
        environment: &mut Environment<'_>,
    ) -> Result<()> {
        if resolved.contains_key(name) || done.contains(name) {
            return Ok(());
        }
        if stack.iter().any(|item| item == name) {
            stack.push(name.to_owned());
            bail!("Variable reference cycle: {}", stack.join(" -> "));
        }
        if stack.len() >= 64 {
            bail!("Variable reference depth exceeds 64");
        }
        stack.push(name.to_owned());
        let value = &definitions[name];
        let mut dependencies = Vec::new();
        references(value, environment, &mut dependencies, 0)?;
        dependencies.sort();
        dependencies.dedup();
        for dependency in dependencies {
            if definitions.contains_key(&dependency) {
                visit(&dependency, definitions, resolved, done, stack, environment)?;
            }
        }
        let context = create_context(resolved)?;
        match render(value, environment, &context, 0) {
            Ok(value) => {
                resolved.insert(name.to_owned(), value);
            }
            Err(error)
                if error
                    .downcast_ref::<minijinja::Error>()
                    .is_some_and(|error| error.kind() == minijinja::ErrorKind::UndefinedError) => {}
            Err(error) => {
                return Err(error).with_context(|| format!("Cannot resolve variable '{name}'"))
            }
        }
        stack.pop();
        done.insert(name.to_owned());
        Ok(())
    }
    let mut resolved = data.clone();
    let mut environment = environment();
    let mut done = std::collections::HashSet::new();
    let mut names: Vec<_> = definitions.keys().collect();
    names.sort();
    for name in names {
        visit(
            name,
            definitions,
            &mut resolved,
            &mut done,
            &mut Vec::new(),
            &mut environment,
        )?;
    }
    Ok(resolved)
}

pub(crate) fn create_context(vars: &HashMap<String, Value>) -> Result<TemplateContext> {
    let mut context = TemplateContext::new();
    for (name, value) in vars {
        context.insert(
            name.clone(),
            serde_json::to_value(value).with_context(|| {
                format!("Cannot represent variable '{name}' in template context")
            })?,
        );
    }
    context.insert("omit".to_string(), OMIT_SENTINEL.into());
    Ok(context)
}

/// Ansible 2.21 requires boolean condition results. In particular the string
/// "false" must never become true just because it contains characters.
pub(crate) fn evaluate_condition(
    condition: &str,
    environment: &mut Environment<'_>,
    context: &TemplateContext,
) -> Result<bool> {
    match render_value(
        &format!("{{{{ {condition} }}}}"),
        environment,
        context,
        false,
    )? {
        Value::Bool(result) => Ok(result),
        _ => bail!("Condition must evaluate to a boolean"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_types_are_preserved_without_parsing_strings() {
        let context = create_context(&HashMap::from([
            ("text".into(), Value::String("0012".into())),
            ("literal".into(), Value::String("{{ untrusted }}".into())),
        ]))
        .unwrap();
        let mut environment = environment();
        for (input, expected) in [
            (
                "ordinary {{ text }} text",
                Value::String("ordinary 0012 text".into()),
            ),
            ("0012", Value::String("0012".into())),
            ("{{ text }}", Value::String("0012".into())),
            ("{{ literal }}", Value::String("{{ untrusted }}".into())),
            ("{{ true }}", Value::Bool(true)),
            ("{{ 12 }}", Value::Number(12.into())),
            (" {{ 12 }} ", Value::String(" 12 ".into())),
            ("{{ 12 }}\n", Value::String("12\n".into())),
            ("{{- 12 -}}", Value::Number(12.into())),
            (
                "{{ [1, 2] }}",
                Value::Sequence(vec![Value::Number(1.into()), Value::Number(2.into())]),
            ),
            ("{{ 1 }}{{ 2 }}", Value::String("12".into())),
            (
                "{{ {'nested': '}}'} }}",
                serde_yaml::from_str("nested: '}}'").unwrap(),
            ),
            (
                "普通中文 😀 default(1) | password_hash('abc')",
                Value::String("普通中文 😀 default(1) | password_hash('abc')".into()),
            ),
        ] {
            assert_eq!(
                render_value(input, &mut environment, &context, false).unwrap(),
                expected,
                "{input}"
            );
        }
        assert_eq!(
            render_value("{{ 12 }}", &mut environment, &context, true).unwrap(),
            Value::String("12".into())
        );
        assert!(render_value("{{ missing }}", &mut environment, &context, false).is_err());
        assert!(render_value("{{ 1 + }}", &mut environment, &context, false).is_err());
    }

    #[test]
    fn defaults_omit_inline_conditionals_and_filters_share_one_language() {
        let mut environment = environment();
        let context = create_context(&HashMap::new()).unwrap();
        for (source, expected) in [
            ("{{ missing | default('0012') }}", Value::String("0012".into())),
            ("{{ missing | default(omit) }}", Value::String(OMIT_SENTINEL.into())),
            ("{{ 'yes' if true else 'no' }}", Value::String("yes".into())),
            ("{{ [{'n': 'a', 'ok': true}, {'n': 'b', 'ok': false}] | selectattr('ok', 'equalto', true) | map(attribute='n') | list }}", Value::Sequence(vec![Value::String("a".into())])),
        ] {
            assert_eq!(render_value(source, &mut environment, &context, false).unwrap(), expected);
        }
        assert_eq!(
            render_value(
                "{{ missing | default('0012') }}",
                &mut environment,
                &context,
                true
            )
            .unwrap(),
            Value::String("0012".into())
        );
    }

    #[test]
    fn conditions_require_boolean_values() {
        let mut environment = environment();
        let context = create_context(&HashMap::new()).unwrap();
        for source in ["true", "1 == 1", "missing is not defined", "'a' in ['a']"] {
            assert!(evaluate_condition(source, &mut environment, &context).unwrap());
        }
        assert!(!evaluate_condition("false", &mut environment, &context).unwrap());
        for source in ["'false'", "1", "[]", "missing", "{{ true }}"] {
            assert!(
                evaluate_condition(source, &mut environment, &context).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn context_rejects_non_string_mapping_keys() {
        let mapping = serde_yaml::Mapping::from_iter([(
            Value::Sequence(vec![Value::Bool(true)]),
            Value::Bool(true),
        )]);
        assert!(create_context(&HashMap::from([(
            "invalid".into(),
            Value::Mapping(mapping)
        )]))
        .is_err());
    }

    #[test]
    fn definitions_resolve_chains_and_cycles_but_runtime_data_remains_literal() {
        let definitions = HashMap::from([
            ("first".into(), Value::String("{{ second }}".into())),
            ("second".into(), Value::String("{{ third }}".into())),
            ("third".into(), Value::String("0012".into())),
            ("output".into(), Value::String("{{ external }}".into())),
            (
                "deferred".into(),
                Value::String("{{ fact_not_gathered_yet }}".into()),
            ),
        ]);
        let data = HashMap::from([("external".into(), Value::String("{{ untrusted }}".into()))]);
        let resolved = resolve_definitions(&definitions, &data).unwrap();
        assert_eq!(resolved["first"], Value::String("0012".into()));
        assert_eq!(resolved["output"], data["external"]);
        assert!(!resolved.contains_key("deferred"));
        let cycle = HashMap::from([
            ("a".into(), Value::String("{{ b }}".into())),
            ("b".into(), Value::String("{{ a }}".into())),
        ]);
        assert!(resolve_definitions(&cycle, &HashMap::new())
            .unwrap_err()
            .to_string()
            .contains("a -> b -> a"));
        let invalid = HashMap::from([("bad".into(), Value::String("{{ broken".into()))]);
        assert!(resolve_definitions(&invalid, &HashMap::new()).is_err());
    }
}
