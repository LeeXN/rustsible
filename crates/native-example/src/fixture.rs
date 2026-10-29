//! Acceptance-only peer; never included in a published example collection.
use anyhow::{Context, Result};
use rustsible_protocol::{extension::*, ModuleResult};
use std::io::Write;
fn main() -> Result<()> {
    let definitions = rustsible_native_example::fixture_catalog();
    if std::env::var_os("RUSTSIBLE_FIXTURE_BAD_HELLO").is_some() {
        let probe: Probe = rustsible_protocol::read_frame(
            &mut std::io::stdin().lock(),
            rustsible_protocol::MAX_REQUEST_BYTES,
        )?;
        let definition = definitions
            .into_iter()
            .find(|definition| definition.name == probe.name)
            .context("Unknown fixture component")?;
        return rustsible_protocol::write_frame(
            &mut std::io::stdout().lock(),
            &Hello {
                api_version: 999,
                wire_version: rustsible_protocol::VERSION,
                definition,
            },
            MAX_RESULT_BYTES,
        );
    }
    if std::env::var_os("RUSTSIBLE_FIXTURE_RAW_STDOUT").is_some() {
        std::io::stdout().write_all(&u32::MAX.to_be_bytes())?;
        return Ok(());
    }
    serve(&definitions, |definition, call| {
        match definition.name.as_str() {
            "fixture.native.crash" => std::process::exit(91),
            "fixture.native.write" => {
                let path = call.args["path"].as_str().context("Missing fixture path")?;
                let content = call.args["content"]
                    .as_str()
                    .context("Missing fixture content")?;
                let before = std::fs::read(path).ok();
                let changed = before.as_deref() != Some(content.as_bytes());
                let mut values = std::collections::HashMap::new();
                if changed && call.diff {
                    values.insert("diff".into(), serde_yaml::to_value(serde_json::json!({"before":before.as_deref().map(|bytes|String::from_utf8_lossy(&bytes[..bytes.len().min(65536)]).to_string()),"after":content.chars().take(65536).collect::<String>(),"native_fixture":true}))?);
                }
                // Only fresh test-owned files reach this acceptance component.
                // The real collection uses builtin atomic file modules.
                if changed && !call.check {
                    std::fs::write(path, content)?;
                }
                Ok(ModuleResult {
                    changed,
                    msg: "Fixture write complete".into(),
                    values,
                    ..Default::default()
                })
            }
            "fixture.native.timeout" => {
                let path = call.args["path"]
                    .as_str()
                    .context("Missing fixture pid path")?;
                if !call.check {
                    std::process::Command::new("setsid")
                        .args([
                            "/bin/sh",
                            "-c",
                            "echo $$ > \"$1\"; exec sleep 60",
                            "fixture",
                            path,
                        ])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()?;
                    std::thread::sleep(std::time::Duration::from_secs(30));
                }
                Ok(ModuleResult::default())
            }
            _ => rustsible_native_example::handle(definition, call),
        }
    })
}
