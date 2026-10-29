//! OS tools use argument arrays and bounded duplex IO. No command text or
//! secret stdin is copied into diagnostic contexts or process arguments.
use anyhow::{Context, Result};
use rustsible_protocol::{change, ModuleResult};
use std::{ffi::OsStr, os::unix::fs::PermissionsExt, process::Command};
#[derive(Debug)]
pub struct Failure(pub ModuleResult);
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.msg)
    }
}
impl std::error::Error for Failure {}
pub fn available(program: &str) -> bool {
    if program.contains('/') {
        return executable(std::path::Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|path| executable(&path.join(program)))
    })
}
fn executable(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
pub fn query(program: &str, args: &[impl AsRef<OsStr>]) -> Result<ModuleResult> {
    run(program, args, None, false)
}
pub fn run(
    program: &str,
    args: &[impl AsRef<OsStr>],
    input: Option<&[u8]>,
    mutation: bool,
) -> Result<ModuleResult> {
    let mut command = Command::new(program);
    command.args(args).env("LC_ALL", "C");
    // Apt's package configuration must not block waiting for a terminal.
    if program == "apt-get" {
        command.env("DEBIAN_FRONTEND", "noninteractive");
    }
    crate::process::run(command, input, mutation, true).context("Cannot execute required OS tool")
}
pub fn require(mut result: ModuleResult, operation: &str) -> Result<ModuleResult> {
    if result.failed {
        result.msg = format!(
            "{operation} failed (exit {}): {}",
            result
                .rc
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".into()),
            result.stderr.trim()
        );
        return Err(Failure(result).into());
    }
    Ok(result)
}
pub fn mutate(program: &str, args: &[impl AsRef<OsStr>], operation: &str) -> Result<ModuleResult> {
    let result = require(run(program, args, None, true)?, operation)?;
    change::record_change();
    Ok(result)
}
pub fn failed(result: ModuleResult, operation: &str) -> anyhow::Error {
    require(result, operation).expect_err("Expected a failed tool result")
}
