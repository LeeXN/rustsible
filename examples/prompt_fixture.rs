//! Acceptance-only PTY driver; fixture passwords arrive on stdin, never argv.
#[path = "../tests/support/terminal.rs"]
mod terminal;
use anyhow::{bail, Context, Result};
use std::{io::Read, process::Command};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let binary = args
        .next()
        .context("Expected controller and its arguments")?;
    let mut input = String::new();
    std::io::stdin().take(65537).read_to_string(&mut input)?;
    if input.len() > 65536 {
        bail!("Fixture input exceeded bound");
    }
    let passwords = input.lines().map(str::as_bytes).collect::<Vec<_>>();
    let output = terminal::run(Command::new(binary).args(args), &passwords)?;
    if !output.echo_restored {
        bail!("Prompt did not restore terminal echo");
    }
    print!("{}", output.transcript);
    if !output.status.success() {
        bail!("Controller fixture failed");
    }
    Ok(())
}
