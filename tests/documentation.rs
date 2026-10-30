mod support;
use anyhow::{Context, Result};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
type Snapshot = std::collections::BTreeMap<std::path::PathBuf, (Vec<u8>, u32, u32, u32)>;
fn snapshot(root: &Path) -> Result<Snapshot> {
    use std::os::unix::fs::MetadataExt;
    fn walk(base: &Path, dir: &Path, values: &mut Snapshot) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            values.insert(
                path.strip_prefix(base)?.to_owned(),
                (
                    if metadata.is_file() {
                        fs::read(&path)?
                    } else {
                        Vec::new()
                    },
                    metadata.mode(),
                    metadata.uid(),
                    metadata.gid(),
                ),
            );
            if metadata.is_dir() {
                walk(base, &path, values)?
            }
        }
        Ok(())
    }
    let mut values = std::collections::BTreeMap::new();
    walk(root, root, &mut values)?;
    Ok(values)
}
fn run(book: &Path, root: &Path, flags: &[&str]) -> Result<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_rustsible"))
        .arg("--worker-dir")
        .arg(support::worker_bundle())
        .arg("playbook")
        .arg(book)
        .arg("-i")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/quickstart/inventory.yml"))
        .arg("-e")
        .arg(format!(
            "demo_root={} demo_output={}",
            root.display(),
            root.join("role").display()
        ))
        .args(flags)
        .output()?)
}
fn success(output: Output) -> Result<String> {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}
#[test]
fn handbook_local_recipes_preserve_check_state_and_are_idempotent() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    for example in [
        "quickstart/playbook.yml",
        "files/playbook.yml",
        "variables/playbook.yml",
    ] {
        let book = repo.join("examples").join(example);
        let before = snapshot(root)?;
        success(run(&book, root, &["--check", "--diff"])?)?;
        assert_eq!(before, snapshot(root)?);
        success(run(&book, root, &[])?)?;
        let repeated = success(run(&book, root, &[])?)?;
        assert!(
            repeated.contains("changed=0 unreachable=0 failed=0"),
            "{example}: {repeated}"
        );
    }
    assert_eq!(
        fs::read(root.join("app/config"))?,
        b"application=rustsible\nenabled=true\n"
    );
    assert_eq!(fs::read(root.join("app/applied"))?, b"applied");
    assert_eq!(
        fs::read(root.join("files/config"))?,
        b"name=rustsible\nenabled=true\n"
    );
    assert_eq!(fs::read(root.join("managed-lines"))?, b"enabled=true\n");
    assert_eq!(fs::read(root.join("one"))?, b"one");
    assert_eq!(fs::read(root.join("two"))?, b"two");
    success(run(
        &repo.join("examples/quickstart/cleanup.yml"),
        root,
        &[],
    )?)?;
    assert!(!root.join("app").exists());
    Ok(())
}
#[test]
fn all_module_reference_playbooks_validate_without_connecting() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temporary = tempfile::tempdir()?;
    for module in rustsible_protocol::registry::Builtin::ALL {
        let page = fs::read_to_string(
            repo.join("docs/en/reference/modules")
                .join(format!("{}.md", module.name())),
        )?;
        let block = page
            .split("```yaml\n")
            .nth(1)
            .context("Missing complete module example")?
            .split("```")
            .next()
            .unwrap();
        let file = temporary.path().join(format!("{}.yml", module.name()));
        fs::write(&file, block)?;
        success(run(&file, temporary.path(), &["--syntax-check"])?)?;
    }
    let invalid = run(
        &repo.join("tests/fixtures/playbooks/unsupported-recurse.yml"),
        temporary.path(),
        &["--syntax-check"],
    )?;
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("recurse"));
    Ok(())
}
