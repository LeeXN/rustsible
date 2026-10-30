//! Archive decoding validates every path/link and complete content before any
//! business installation. The caller supplies private scratch and installs the
//! returned entries using its own atomic transaction boundary.
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read, Seek},
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path, PathBuf},
};
pub const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
pub struct Entry {
    pub staged: Option<PathBuf>,
    pub mode: u32,
}

pub fn relative_path(path: &Path) -> Result<PathBuf> {
    let bytes = path.to_str().context("Archive paths must be UTF-8")?;
    if bytes.len() > 4096 || bytes.contains(['\\', ':', '\0']) {
        bail!("Unsafe or oversized archive path");
    }
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => output.push(name),
            Component::CurDir => {}
            _ => bail!("Archive path escapes destination"),
        }
    }
    Ok(output)
}
pub fn decode(mut input: File, scratch: &Path) -> Result<BTreeMap<PathBuf, Entry>> {
    let mut magic = [0u8; 4];
    let count = input.read(&mut magic)?;
    input.rewind()?;
    let mut entries = BTreeMap::new();
    let mut total = 0;
    if count >= 2 && magic[..2] == *b"PK" {
        let mut archive = zip::ZipArchive::new(input).context("Invalid ZIP archive")?;
        if archive.len() > MAX_ENTRIES {
            bail!("Too many archive entries");
        }
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).context("Cannot read ZIP entry")?;
            if entry.is_symlink()
                || entry.unix_mode().is_some_and(|mode| {
                    mode & libc::S_IFMT != 0
                        && !matches!(mode & libc::S_IFMT, libc::S_IFREG | libc::S_IFDIR)
                })
            {
                bail!("Archive links and special files are unsupported");
            }
            let path = relative_path(Path::new(entry.name()?.as_ref()))?;
            let directory = entry.is_dir();
            if directory && entry.size() != 0 {
                bail!("Archive directory contains file data");
            }
            let mode = entry
                .unix_mode()
                .unwrap_or(if directory { 0o755 } else { 0o644 })
                & 0o777;
            add(
                &mut entries,
                path,
                directory,
                mode,
                &mut entry,
                scratch,
                &mut total,
            )?;
        }
    } else {
        let decoded;
        let source = if count >= 2 && magic[..2] == [0x1f, 0x8b] {
            decoded = scratch.join("decoded.tar");
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&decoded)?;
            let bytes = io::copy(
                &mut flate2::read::MultiGzDecoder::new(input).take(MAX_TOTAL + 1),
                &mut file,
            )
            .context("Invalid or truncated gzip archive")?;
            if bytes > MAX_TOTAL {
                bail!("Expanded archive exceeds 8 GiB");
            }
            file.rewind()?;
            file
        } else {
            input
        };
        let mut archive = tar::Archive::new(source);
        for entry in archive.entries().context("Invalid tar archive")? {
            let mut entry = entry.context("Invalid or truncated tar entry")?;
            let kind = entry.header().entry_type();
            if !kind.is_file() && !kind.is_dir() {
                bail!("Archive links and special files are unsupported");
            }
            if kind.is_dir() && entry.size() != 0 {
                bail!("Archive directory contains file data");
            }
            let path = relative_path(&entry.path()?)?;
            let mode = entry.header().mode()? & 0o777;
            add(
                &mut entries,
                path,
                kind.is_dir(),
                mode,
                &mut entry,
                scratch,
                &mut total,
            )?;
        }
    }
    Ok(entries)
}
fn add(
    entries: &mut BTreeMap<PathBuf, Entry>,
    path: PathBuf,
    is_dir: bool,
    mode: u32,
    input: &mut impl Read,
    scratch: &Path,
    total: &mut u64,
) -> Result<()> {
    if path.as_os_str().is_empty() {
        if is_dir {
            return Ok(());
        }
        bail!("Archive file needs a name");
    }
    if entries.len() >= MAX_ENTRIES {
        bail!("Too many archive entries");
    }
    if let Some(previous) = entries.get_mut(&path) {
        if is_dir && previous.staged.is_none() {
            previous.mode = mode;
            return Ok(());
        }
        bail!("Duplicate archive file path");
    }
    let mut parent = path.parent();
    while let Some(ancestor) = parent.filter(|path| !path.as_os_str().is_empty()) {
        if entries
            .get(ancestor)
            .is_some_and(|entry| entry.staged.is_some())
        {
            bail!("Archive file used as a parent directory");
        }
        entries.entry(ancestor.to_owned()).or_insert(Entry {
            staged: None,
            mode: 0o755,
        });
        if entries.len() >= MAX_ENTRIES {
            bail!("Too many archive entries");
        }
        parent = ancestor.parent();
    }
    if !is_dir && entries.keys().any(|existing| existing.starts_with(&path)) {
        bail!("Archive file conflicts with a directory");
    }
    let staged = if is_dir {
        None
    } else {
        let file = scratch.join(format!("entry-{}", entries.len()));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)?;
        let copied = io::copy(
            &mut input.take(MAX_TOTAL.saturating_sub(*total) + 1),
            &mut output,
        )
        .context("Invalid, truncated or corrupt archive content")?;
        *total = total.checked_add(copied).context("Archive size overflow")?;
        if *total > MAX_TOTAL {
            bail!("Expanded archive exceeds 8 GiB");
        }
        Some(file)
    };
    entries.insert(path, Entry { staged, mode });
    Ok(())
}
