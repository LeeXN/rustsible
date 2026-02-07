use anyhow::Result;
use rustsible::inventory;
use rustsible::playbook;
use std::path::PathBuf;

#[test]
fn test_integration_simple_playbook_local() -> Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let inventory_path = manifest_dir.join("tests/fixtures/inventory/simple.ini");
    let playbook_path = manifest_dir.join("tests/fixtures/playbooks/simple.yml");

    // Ensure fixtures exist
    if !inventory_path.exists() || !playbook_path.exists() {
        // Skip if fixtures are missing (e.g. in CI without them)
        return Ok(());
    }

    println!("Loading inventory from: {:?}", inventory_path);
    let inventory = inventory::parse(inventory_path.to_str().unwrap())?;

    println!("Executing playbook: {:?}", playbook_path);
    playbook::execute(playbook_path.to_str().unwrap(), &inventory)?;

    Ok(())
}
