use anyhow::{bail, Context, Result};
use serde_yaml::{Mapping, Value};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub(crate) use rustsible_protocol::param::validate_variable_name as validate_name;

pub(crate) fn parse(content: &str) -> Result<Mapping> {
    let value: Value = serde_yaml::from_str(content)?;
    if value.is_null() {
        return Ok(Mapping::new());
    }
    let mapping = value.as_mapping().context("Variables must be a mapping")?;
    for key in mapping.keys() {
        validate_name(
            key.as_str().context("Variable names must be strings")?,
            "Variables",
        )?;
    }
    Ok(mapping.clone())
}

pub(crate) fn load(path: &Path) -> Result<Mapping> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("Cannot open variable file {}", path.display()))?;
    if !file.metadata()?.is_file() {
        bail!("Variable file must be a regular file: {}", path.display());
    }
    let mut content = String::new();
    file.read_to_string(&mut content)
        .with_context(|| format!("Cannot read variable file {}", path.display()))?;
    parse(&content).with_context(|| format!("Invalid variable file {}", path.display()))
}

pub(crate) fn extra(entries: &[String]) -> Result<std::collections::HashMap<String, Value>> {
    let mut result = std::collections::HashMap::new();
    for entry in entries {
        let mapping = if let Some(path) = entry.strip_prefix('@') {
            load(Path::new(path))?
        } else if entry.trim_start().starts_with('{') {
            parse(entry)?
        } else {
            let mut mapping = Mapping::new();
            for token in rustsible_protocol::tokenize_args(entry)? {
                let (name, value) = token
                    .split_once('=')
                    .context("Extra variables require name=value or a mapping")?;
                validate_name(name, "Extra variables")?;
                mapping.insert(Value::String(name.into()), Value::String(value.into()));
            }
            if mapping.is_empty() {
                bail!("Extra variables must not be empty");
            }
            mapping
        };
        let layer = mapping
            .into_iter()
            .map(|(name, value)| (name.as_str().expect("Validated variable").to_owned(), value))
            .collect();
        crate::inventory::host::merge_variable_layer(&mut result, &layer);
    }
    Ok(result)
}

pub(crate) fn merge(target: &mut Mapping, layer: Mapping) {
    let to_variables = |mapping: &Mapping| {
        mapping
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().expect("Validated variable name").to_owned(),
                    value.clone(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>()
    };
    let mut variables = to_variables(target);
    crate::inventory::host::merge_variable_layer(&mut variables, &to_variables(&layer));
    *target = variables
        .into_iter()
        .map(|(name, value)| (Value::String(name), value))
        .collect();
}

/// Match Ansible's extension preference and lexical directory ordering. Files
/// are content only; executable flags never run a program. Directory symlink
/// loops fail before the loader can recurse indefinitely.
pub(crate) fn named_files(directory: &Path, name: &str) -> Result<Vec<PathBuf>> {
    for extension in ["", ".yml", ".yaml", ".json"] {
        let path = directory.join(format!("{name}{extension}"));
        if path.exists() {
            return if path.is_dir() {
                directory_files(&path, &mut Vec::new())
            } else {
                Ok(vec![path])
            };
        }
    }
    Ok(Vec::new())
}

fn directory_files(directory: &Path, stack: &mut Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let canonical = directory.canonicalize()?;
    if stack.contains(&canonical) || stack.len() >= 64 {
        bail!(
            "Variable directory loop or depth limit at {}",
            directory.display()
        );
    }
    stack.push(canonical);
    let mut entries = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    let mut files = Vec::new();
    for path in entries {
        let name = path
            .file_name()
            .context("Variable path has no name")?
            .to_string_lossy();
        if name.starts_with('.') || name.ends_with('~') {
            continue;
        }
        if path.is_dir() && path.extension().is_none() {
            files.extend(directory_files(&path, stack)?);
        } else if path.is_file()
            && path
                .extension()
                .is_none_or(|extension| matches!(extension.to_str(), Some("yml" | "yaml" | "json")))
        {
            files.push(path);
        }
    }
    stack.pop();
    Ok(files)
}

pub(crate) fn include(args: &Value) -> Result<rustsible_protocol::ModuleResult> {
    let files = if let Some(file) = args.get("file").and_then(Value::as_str) {
        vec![PathBuf::from(file)]
    } else {
        directory_files(
            Path::new(
                args.get("dir")
                    .and_then(Value::as_str)
                    .context("include_vars needs a directory")?,
            ),
            &mut Vec::new(),
        )?
    };
    let mut variables = Mapping::new();
    for file in &files {
        merge(&mut variables, load(file)?);
    }
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        variables = Mapping::from_iter([(Value::String(name.into()), Value::Mapping(variables))]);
    }
    Ok(rustsible_protocol::ModuleResult {
        values: std::collections::HashMap::from([
            ("ansible_facts".into(), Value::Mapping(variables)),
            (
                "ansible_included_var_files".into(),
                serde_yaml::to_value(files)?,
            ),
        ]),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variable_paths_use_preferred_extension_and_sorted_recursive_content() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("web.yml"), "winner: preferred").unwrap();
        fs::write(temp.path().join("web.yaml"), "winner: later").unwrap();
        let files = named_files(temp.path(), "web").unwrap();
        assert_eq!(files, vec![temp.path().join("web.yml")]);
        assert_eq!(
            load(&files[0]).unwrap()["winner"].as_str(),
            Some("preferred")
        );
        fs::create_dir(temp.path().join("web")).unwrap();
        for (name, value) in [
            ("a.yml", "a: 1"),
            ("z", "z: true"),
            (".hidden.yml", "bad"),
            ("a.yml~", "bad"),
            ("skip.txt", "bad"),
        ] {
            fs::write(temp.path().join("web").join(name), value).unwrap();
        }
        fs::create_dir(temp.path().join("web/nested")).unwrap();
        fs::write(
            temp.path().join("web/nested/data.json"),
            "{\"nested\": [1,2]}",
        )
        .unwrap();
        let files = named_files(temp.path(), "web").unwrap();
        assert_eq!(
            files
                .iter()
                .map(|file| file.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            ["a.yml", "data.json", "z"]
        );
        assert!(load(&files[1]).unwrap()["nested"].is_sequence());
        assert!(named_files(temp.path(), "missing").unwrap().is_empty());
    }
    #[test]
    fn malformed_variables_and_directory_cycles_fail_without_executing_files() {
        for content in ["- list", "42", "invalid-name: x", "1: value", "x: ["] {
            assert!(parse(content).is_err(), "{content}");
        }
        assert!(parse("").unwrap().is_empty());
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("web")).unwrap();
        std::os::unix::fs::symlink(temp.path().join("web"), temp.path().join("web/back")).unwrap();
        assert!(named_files(temp.path(), "web")
            .unwrap_err()
            .to_string()
            .contains("loop"));
        assert!(load(&temp.path().join("web")).is_err());
        assert!(load(&temp.path().join("missing")).is_err());
    }
    #[test]
    fn extra_variables_preserve_key_value_strings_and_structured_file_types() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("vars.yml");
        fs::write(
            &path,
            "enabled: true\nitems: [one, two]\nansible_user: first",
        )
        .unwrap();
        let variables = extra(&[
            format!("@{}", path.display()),
            "number=0012 flag=false message='two words'".into(),
            r#"{"number": 7, "ansible_ssh_user": "last"}"#.into(),
        ])
        .unwrap();
        assert_eq!(variables["number"].as_i64(), Some(7));
        assert_eq!(variables["flag"].as_str(), Some("false"));
        assert_eq!(variables["message"].as_str(), Some("two words"));
        assert_eq!(variables["enabled"].as_bool(), Some(true));
        assert!(variables["items"].is_sequence());
        assert_eq!(variables["ansible_user"].as_str(), Some("last"));
        for value in ["", "bad", "bad-name=x", "{not: [valid}"] {
            assert!(extra(&[value.into()]).is_err());
        }
    }
}
