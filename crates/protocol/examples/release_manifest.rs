//! Validate the selected controller and worker bytes before describing a release.
use anyhow::{bail, Context, Result};
use rustsible_protocol::{extension::verify_binary, Manifest, BUILD_ID, VERSION};
use std::{fs, path::PathBuf, process::Command};
fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("Expected release directory")?,
    )
    .canonicalize()?;
    let output = Command::new(root.join("rustsible"))
        .arg("build-info")
        .output()?;
    if !output.status.success() || output.stdout.len() > 4096 {
        bail!("Cannot read controller build identity");
    }
    let controller: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    if controller["build_id"] != BUILD_ID
        || controller["protocol_version"] != VERSION
        || controller["os"] != "linux"
    {
        bail!("Controller does not match this release source/protocol");
    }
    let arch = controller["arch"]
        .as_str()
        .context("Missing controller architecture")?;
    let mut binary = fs::File::open(root.join("rustsible"))?;
    let sha256 = rustsible_protocol::sha256(&mut binary)?;
    verify_binary(&mut binary, &sha256, arch, "Release controller")?;
    let workers: Manifest = serde_json::from_slice(&fs::read(root.join("workers/manifest.json"))?)?;
    workers.validate()?;
    if workers.build_id != BUILD_ID
        || workers.workers.len() != 2
        || !["x86_64", "aarch64"]
            .iter()
            .all(|arch| workers.workers.iter().any(|worker| worker.arch == *arch))
    {
        bail!("Release requires both matching worker architectures");
    }
    for worker in &workers.workers {
        verify_binary(
            &mut fs::File::open(root.join("workers").join(&worker.file))?,
            &worker.sha256,
            &worker.arch,
            "Release worker",
        )?;
    }
    let release = serde_json::json!({"format":1,"source_commit":BUILD_ID,"controller":{"file":"rustsible","sha256":sha256,"build":controller},"workers":workers});
    fs::write(
        root.join("RELEASE.json"),
        serde_json::to_vec_pretty(&release)?,
    )?;
    Ok(())
}
