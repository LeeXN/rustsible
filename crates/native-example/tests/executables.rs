//! Exercise the installed entry points as protocol peers. This also makes Cargo
//! build both standalone executables in isolated coverage target directories.
use anyhow::{Context, Result};
use rustsible_protocol::{
    extension::{Call, Hello, Kind, Probe, Reply, API_VERSION, MAX_RESULT_BYTES},
    read_frame, write_frame, MAX_REQUEST_BYTES,
};
use std::process::{Command, Stdio};
#[test]
fn standalone_example_and_acceptance_peer_speak_the_declared_sdk_protocol() -> Result<()> {
    for program in [
        env!("CARGO_BIN_EXE_rustsible-example-extension"),
        env!("CARGO_BIN_EXE_rustsible-extension-fixture"),
    ] {
        let mut child = Command::new(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().context("Missing executable input")?;
        let mut output = child.stdout.take().context("Missing executable output")?;
        write_frame(
            &mut input,
            &Probe {
                api_version: API_VERSION,
                name: "example.native.probe".into(),
                kind: Kind::Module,
            },
            MAX_REQUEST_BYTES,
        )?;
        let hello: Hello = read_frame(&mut output, MAX_RESULT_BYTES)?;
        let definition = rustsible_native_example::catalog()
            .into_iter()
            .find(|definition| definition.name == "example.native.probe")
            .unwrap();
        hello.validate(&definition)?;
        write_frame(
            &mut input,
            &Call::new(
                &definition,
                serde_yaml::from_str("path: /proc/self/status")?,
                true,
                false,
                1000,
            ),
            MAX_REQUEST_BYTES,
        )?;
        let result: Reply = read_frame(&mut output, MAX_RESULT_BYTES)?;
        assert!(!result.validate()?.failed);
        drop(input);
        assert!(child.wait()?.success());
    }
    Ok(())
}
