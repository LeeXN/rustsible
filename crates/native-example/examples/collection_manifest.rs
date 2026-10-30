//! Explicit authoring tool: inspect ELF headers and hash bytes without running
//! any collection executable. Both architectures can share one catalogue.
use anyhow::{bail, Context, Result};
use rustsible_protocol::extension::{Artifact, Collection, Entry};
use std::{collections::BTreeMap, fs, path::PathBuf};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(
        args.next()
            .context("Expected output collection and arch=program arguments")?,
    );
    let mut artifacts = BTreeMap::new();
    fs::create_dir_all(root.join("bin"))?;
    for argument in args {
        let (arch, source) = argument.split_once('=').context("Expected arch=program")?;
        if artifacts.contains_key(arch) {
            bail!("Duplicate architecture");
        }
        let path = format!("bin/example-{arch}");
        let destination = root.join(&path);
        fs::copy(source, &destination)?;
        let mut file = fs::File::open(destination)?;
        let sha256 = rustsible_protocol::sha256(&mut file)?;
        rustsible_protocol::extension::verify_binary(&mut file, &sha256, arch, "Example program")?;
        artifacts.insert(arch.to_owned(), Artifact { path, sha256 });
    }
    if artifacts.is_empty() {
        bail!("At least one architecture is required");
    }
    let metadata = Collection {
        format: 1,
        namespace: "example".into(),
        name: "native".into(),
        version: "0.1.0".into(),
        extensions: rustsible_native_example::catalog()
            .into_iter()
            .map(|definition| Entry {
                definition,
                artifacts: artifacts.clone(),
            })
            .collect(),
    };
    metadata.validate()?;
    fs::write(
        root.join("rustsible-collection.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(())
}
