use std::{env, path::PathBuf, process::Command};
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn main() {
    println!("cargo:rerun-if-env-changed=RUSTSIBLE_BUILD_ID");
    for name in [
        "HEAD".to_string(),
        git(&["symbolic-ref", "-q", "HEAD"]).unwrap_or_default(),
    ] {
        if !name.is_empty() {
            if let Some(path) = git(&["rev-parse", "--git-path", &name]) {
                let path = PathBuf::from(path);
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    let build = env::var("RUSTSIBLE_BUILD_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .expect("Build from a Git checkout or set RUSTSIBLE_BUILD_ID for a source archive");
    assert!(
        build.len() <= 128
            && build
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)),
        "Invalid build identifier"
    );
    println!("cargo:rustc-env=RUSTSIBLE_BUILD_ID={build}");
}
