use std::{fs, path::PathBuf, sync::OnceLock};

/// Tests use the release directory contract and freeze a binary snapshot so a
/// later cargo invocation cannot replace a worker while it is being uploaded.
pub fn worker_bundle() -> PathBuf {
    static BUNDLE: OnceLock<PathBuf> = OnceLock::new();
    BUNDLE
        .get_or_init(|| {
            let executable = std::env::current_exe().unwrap();
            let debug = executable.parent().unwrap().parent().unwrap();
            let directory = debug
                .join("test-workers")
                .join(std::process::id().to_string());
            fs::create_dir_all(&directory).unwrap();
            let name = format!("rustsible-worker-linux-{}", std::env::consts::ARCH);
            let bytes = fs::read(debug.join("rustsible-worker"))
                .expect("Build workers with cargo build --workspace --locked before testing");
            fs::write(directory.join(&name), &bytes).unwrap();
            let manifest = rustsible_protocol::Manifest {
                version: rustsible_protocol::VERSION,
                build_id: rustsible_protocol::BUILD_ID.into(),
                workers: vec![rustsible_protocol::Artifact {
                    arch: std::env::consts::ARCH.into(),
                    file: name,
                    sha256: rustsible_protocol::sha256(&mut bytes.as_slice()).unwrap(),
                }],
            };
            fs::write(
                directory.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            directory
        })
        .clone()
}
