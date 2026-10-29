//! Git never invokes a shell or installs missing tools. New clones are prepared
//! beside their destination; existing checkouts reject local changes before an
//! update and check mode only asks the remote for its advertised revision.
use crate::{
    filesystem::{rename_no_replace, Location},
    tools,
};
use anyhow::{bail, Context, Result};
use rustsible_protocol::{change, registry::Builtin, ModuleResult, Request};
use serde_yaml::Value;
use std::{fs, path::Path};

pub fn git(request: &Request) -> Result<ModuleResult> {
    Builtin::Git.validate_parameters(&request.args)?;
    if !tools::available("git") {
        bail!("Required target tool git is not installed");
    }
    let args = &request.args;
    let repo = args
        .get("repo")
        .and_then(Value::as_str)
        .context("Git requires repo")?;
    if repo.contains("://") {
        let url = reqwest::Url::parse(repo).context("Invalid Git URL")?;
        if !matches!(url.scheme(), "http" | "https" | "ssh" | "git" | "file")
            || url.password().is_some()
            || matches!(url.scheme(), "http" | "https") && !url.username().is_empty()
        {
            bail!("Unsupported Git URL or credentials in URL; use target SSH keys or a credential helper");
        }
    }
    let dest = args
        .get("dest")
        .and_then(Value::as_str)
        .context("Git requires dest")?;
    let version = args
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("HEAD");
    let update = args.get("update").and_then(Value::as_bool).unwrap_or(true);
    let location = Location::open(dest)?;
    location.ensure_safe_parent()?;
    let existing = location.metadata()?;
    if existing.as_ref().is_some_and(|metadata| !metadata.is_dir()) {
        bail!("Git destination must be a real directory, not a symbolic link");
    }
    // Child tools close CLOEXEC directory descriptors; address this live
    // module's pinned descriptor instead of the child's /proc/self/fd.
    let tool_path = |path: std::path::PathBuf| -> Result<std::path::PathBuf> {
        Ok(
            std::path::PathBuf::from(format!("/proc/{}", std::process::id()))
                .join(path.strip_prefix("/proc/self")?),
        )
    };
    let path = tool_path(location.path())?;
    let path_str = path.to_str().context("Git destination requires UTF-8")?;
    let before = if existing.is_some() {
        if !fs::symlink_metadata(path.join(".git")).is_ok_and(|metadata| metadata.is_dir()) {
            bail!("Git destination must be an independent checkout with a real .git directory");
        }
        let origin = query(Some(path_str), &["remote", "get-url", "origin"])?;
        if origin.trim() != repo {
            bail!("Existing checkout origin differs from repo");
        }
        Some(
            query(Some(path_str), &["rev-parse", "--verify", "HEAD^{commit}"])?
                .trim()
                .to_owned(),
        )
    } else {
        None
    };
    if before.is_some() && !update {
        return Ok(git_result(false, before.clone(), before));
    }
    let desired = advertised(repo, version)?;
    let changed = before.as_ref() != Some(&desired);
    if !changed || request.check {
        return Ok(git_result(changed, before, Some(desired)));
    }
    change::ensure_mutation_allowed()?;
    if before.is_some() {
        if !query(
            Some(path_str),
            &["status", "--porcelain", "--untracked-files=normal"],
        )?
        .is_empty()
        {
            bail!("Git checkout has local changes; refusing to overwrite them");
        }
        require(
            Some(path_str),
            &["fetch", "--no-tags", "origin", &desired],
            true,
        )?;
        change::record_change();
        require(
            Some(path_str),
            &["checkout", "--detach", &desired, "--"],
            true,
        )?;
    } else {
        let stage = crate::staging::create(&location.directory)?;
        let prepared = tool_path(stage.path().join("checkout"))?;
        let prepared_str = prepared.to_str().context("Invalid Git staging path")?;
        require(
            None,
            &[
                "clone",
                "--no-checkout",
                "--no-local",
                "--",
                repo,
                prepared_str,
            ],
            true,
        )?;
        if query(
            Some(prepared_str),
            &["rev-parse", "--verify", &format!("{desired}^{{commit}}")],
        )
        .is_err()
        {
            require(
                Some(prepared_str),
                &["fetch", "--no-tags", "origin", &desired],
                true,
            )?;
        }
        require(
            Some(prepared_str),
            &["checkout", "--detach", &desired, "--"],
            true,
        )?;
        if location.metadata()?.is_some() {
            bail!("Git destination appeared while cloning");
        }
        rename_no_replace(&prepared, &location.path())?;
        change::record_change();
        location.sync()?;
    }
    if query(Some(path_str), &["rev-parse", "--verify", "HEAD^{commit}"])?.trim() != desired {
        bail!("Git did not retain the requested revision");
    }
    Ok(git_result(true, before, Some(desired)))
}
fn advertised(repo: &str, version: &str) -> Result<String> {
    if matches!(version.len(), 40 | 64) && version.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(version.to_ascii_lowercase());
    }
    let branch = format!("refs/heads/{version}");
    let tag = format!("refs/tags/{version}");
    let peeled = format!(
        "{}^{{}}",
        if version.starts_with("refs/tags/") {
            version
        } else {
            &tag
        }
    );
    let text = query(
        None,
        &[
            "ls-remote",
            "--exit-code",
            "--",
            repo,
            version,
            &branch,
            &tag,
            &peeled,
        ],
    )?;
    let values = text
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect::<Vec<_>>();
    // Prefer peeled annotated tags; ambiguous branch/tag short names are refused.
    if values.iter().any(|(_, name)| *name == branch) && values.iter().any(|(_, name)| *name == tag)
    {
        bail!("Ambiguous Git branch/tag; use a full ref or commit");
    }
    let revision = values
        .iter()
        .find(|(_, name)| *name == peeled)
        .or_else(|| {
            values
                .iter()
                .find(|(_, name)| *name == version || *name == branch || *name == tag)
        })
        .map(|(revision, _)| *revision)
        .context("Git revision was not advertised; use HEAD, a branch, tag or full commit")?;
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("Git returned an invalid revision");
    }
    Ok(revision.into())
}
fn run(directory: Option<&str>, args: &[&str], mutation: bool) -> Result<ModuleResult> {
    let mut command = std::process::Command::new("git");
    command
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.ext.allow=never",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    if let Some(directory) = directory {
        command.arg("-C").arg(Path::new(directory));
    }
    command.args(args);
    crate::process::run(command, None, mutation, true)
}
fn require(directory: Option<&str>, args: &[&str], mutation: bool) -> Result<String> {
    Ok(tools::require(run(directory, args, mutation)?, "Git operation")?.stdout)
}
fn query(directory: Option<&str>, args: &[&str]) -> Result<String> {
    require(directory, args, false)
}
fn git_result(changed: bool, before: Option<String>, after: Option<String>) -> ModuleResult {
    ModuleResult {
        changed,
        msg: "Git checkout is at the requested revision".into(),
        values: std::collections::HashMap::from([
            (
                "before".into(),
                before.map(Value::String).unwrap_or(Value::Null),
            ),
            (
                "after".into(),
                after.map(Value::String).unwrap_or(Value::Null),
            ),
        ]),
        ..Default::default()
    }
}

pub fn unarchive(
    request: &Request,
    scratch: &Path,
    payload: Option<&Path>,
) -> Result<ModuleResult> {
    crate::archive::execute(request, scratch, payload)
}
