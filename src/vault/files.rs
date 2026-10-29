//! Vault rewrites use a pinned parent, private same-directory inode and the
//! target installer's protection-metadata policy. Failure before rename leaves
//! the old file intact; no plaintext temporary is created by encrypt/rekey.
use crate::atomic::Target;
use anyhow::{bail, Context, Result};
use std::{
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;
pub struct Input {
    pub bytes: Zeroizing<Vec<u8>>,
    original: Option<Target>,
}
impl Input {
    pub fn read(path: &Path) -> Result<Self> {
        let mut bytes = Zeroizing::new(Vec::new());
        let original = if path == Path::new("-") {
            std::io::stdin()
                .take(super::MAX_ENVELOPE as u64 + 1)
                .read_to_end(&mut bytes)?;
            None
        } else {
            let target = Target::open(path)?;
            target
                .original
                .as_ref()
                .context("Vault input file does not exist")?
                .take(super::MAX_ENVELOPE as u64 + 1)
                .read_to_end(&mut bytes)?;
            target.unchanged()?;
            Some(target)
        };
        if bytes.len() > super::MAX_ENVELOPE {
            bail!("Vault input exceeds 80 MiB");
        }
        Ok(Self { bytes, original })
    }
    pub fn write(&self, input: &Path, output: Option<&Path>, bytes: &[u8]) -> Result<()> {
        if let Some(original) = &self.original {
            original.unchanged()?;
        }
        let output = output.unwrap_or(input);
        if output == Path::new("-") {
            std::io::stdout().lock().write_all(bytes)?;
            return Ok(());
        }
        let target = Target::open(output)?;
        target.write(bytes)
    }
}
