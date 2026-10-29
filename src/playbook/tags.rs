//! Selection happens before task conditions and module work. Dynamic includes
//! select their own tagged children; notified handlers bypass tag selection.
use anyhow::{bail, Result};
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSelection {
    pub only: HashSet<String>,
    pub skip: HashSet<String>,
}
impl Default for TagSelection {
    fn default() -> Self {
        Self {
            only: HashSet::from(["all".into()]),
            skip: HashSet::new(),
        }
    }
}
impl TagSelection {
    pub fn new(only: &[String], skip: &[String]) -> Self {
        let split = |values: &[String]| {
            values
                .iter()
                .flat_map(|value| value.split(','))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<HashSet<_>>()
        };
        let mut only = split(only);
        if only.is_empty() {
            only.insert("all".into());
        }
        Self {
            only,
            skip: split(skip),
        }
    }
    pub(crate) fn literal(&self, tags: &[String]) -> Option<bool> {
        (!tags
            .iter()
            .any(|tag| tag.contains("{{") || tag.contains("{%")))
        .then(|| self.matches(tags))
    }
    pub(crate) fn selected(&self, tags: &[String], vars: &HashMap<String, Value>) -> Result<bool> {
        if let Some(selected) = self.literal(tags) {
            return Ok(selected);
        }
        let mut environment = super::templar::environment();
        let context = super::templar::create_context(vars)?;
        let mut rendered = Vec::new();
        for tag in tags {
            let value = super::templar::render_value(tag, &mut environment, &context, false)?;
            match value {
                Value::String(tag) => rendered.push(tag),
                Value::Sequence(tags) => {
                    for tag in tags {
                        rendered.push(
                            tag.as_str()
                                .ok_or_else(|| anyhow::anyhow!("Tags must resolve to strings"))?
                                .into(),
                        );
                    }
                }
                _ => bail!("Tags must resolve to strings or lists of strings"),
            }
        }
        Ok(self.matches(&rendered))
    }
    fn matches(&self, tags: &[String]) -> bool {
        let tags = if tags.is_empty() {
            HashSet::from(["untagged"])
        } else {
            tags.iter().map(String::as_str).collect()
        };
        let untagged = tags == HashSet::from(["untagged"]);
        let intersects =
            |selected: &HashSet<String>| tags.iter().any(|tag| selected.contains(*tag));
        let selected = tags.contains("always")
            || self.only.contains("all") && !tags.contains("never")
            || intersects(&self.only)
            || self.only.contains("tagged") && !untagged && !tags.contains("never");
        selected
            && if self.skip.contains("all") {
                tags.contains("always") && !self.skip.contains("always")
            } else {
                !intersects(&self.skip) && (!self.skip.contains("tagged") || untagged)
            }
    }
}
pub(crate) fn parse(value: Option<&Value>) -> Result<Vec<String>> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::String(text)) => {
            Ok(text.split(',').map(str::trim).map(str::to_owned).collect())
        }
        Some(Value::Sequence(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| !value.contains('\0'))
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow::anyhow!("tags must be strings"))
            })
            .collect(),
        _ => bail!("tags must be a string or list of strings"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserved_tags_and_skip_precedence_match_reference() {
        for (only, skip, tags, expected) in [
            ("all", "", "never", false),
            ("deploy", "", "always", true),
            ("never", "", "never", true),
            ("tagged", "", "", false),
            ("untagged", "", "", true),
            ("all", "tagged", "", true),
            ("all", "deploy", "always,deploy", false),
            ("deploy", "all", "always,deploy", true),
            ("all", "always", "always", false),
            ("tagged", "", "never,deploy", false),
            ("deploy", "", "never,deploy", true),
        ] {
            let selector = TagSelection::new(&[only.into()], &[skip.into()]);
            let tags = if tags.is_empty() {
                vec![]
            } else {
                tags.split(',').map(str::to_owned).collect()
            };
            assert_eq!(selector.matches(&tags), expected, "{only}/{skip}/{tags:?}");
        }
    }
}
