//! Build the same offline directory consumed by production and acceptance tests.
use anyhow::{bail, Context, Result};
use rustsible_protocol::{Artifact, Manifest, BUILD_ID, VERSION};
use std::{fs, path::PathBuf};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let directory = PathBuf::from(
        args.next()
            .context("Expected output directory and arch=worker arguments")?,
    );
    fs::create_dir_all(&directory)?;
    let mut workers = Vec::new();
    for argument in args {
        let (arch, path) = argument.split_once('=').context("Expected arch=worker")?;
        if !matches!(arch, "x86_64" | "aarch64") {
            bail!("Unsupported worker architecture");
        }
        let name = format!("rustsible-worker-linux-{arch}");
        let destination = directory.join(&name);
        fs::copy(path, &destination).context("Cannot snapshot worker artifact")?;
        workers.push(Artifact {
            arch: arch.into(),
            file: name,
            sha256: rustsible_protocol::sha256(&mut fs::File::open(destination)?)?,
        });
    }
    workers.sort_by(|a, b| a.arch.cmp(&b.arch));
    let manifest = Manifest {
        version: VERSION,
        build_id: BUILD_ID.into(),
        workers,
    };
    manifest.validate()?;
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}
