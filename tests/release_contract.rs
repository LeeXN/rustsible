use anyhow::Result;
use std::process::Command;
#[test]
fn build_identity_is_json_and_does_not_load_inventory_workers_or_configuration() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let invalid = temporary.path().join("invalid.cfg");
    std::fs::write(&invalid, "[unknown]\nunsupported=true\n")?;
    let output = Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .args(["--config", invalid.to_str().unwrap(), "build-info"])
        .env("ANSIBLE_CONFIG", &invalid)
        .current_dir(temporary.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["build_id"], rustsible_protocol::BUILD_ID);
    assert_eq!(value["protocol_version"], rustsible_protocol::VERSION);
    assert_eq!(value["arch"], std::env::consts::ARCH);
    assert!(output.stderr.is_empty());
    Ok(())
}
