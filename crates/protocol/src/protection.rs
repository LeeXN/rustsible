//! Linux protection metadata is shared by target installation and controller
//! Vault rewrites so ACLs, xattrs and ownership follow the same atomic policy.
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::{self, File},
    io,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, PermissionsExt},
    },
};
#[derive(Default, Clone, Copy)]
pub struct Protection {
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}
pub fn preserve(
    output: &mut File,
    source: Option<&File>,
    attributes: Protection,
    default_mode: u32,
) -> Result<()> {
    crate::change::ensure_mutation_allowed()?;
    let metadata = source.map(File::metadata).transpose()?;
    let old = metadata.as_ref();
    let current = output.metadata()?;
    let uid = attributes
        .uid
        .or_else(|| old.map(MetadataExt::uid))
        .unwrap_or(current.uid());
    let gid = attributes
        .gid
        .or_else(|| old.map(MetadataExt::gid))
        .unwrap_or(current.gid());
    if (uid, gid) != (current.uid(), current.gid())
        && unsafe { libc::fchown(output.as_raw_fd(), uid, gid) } != 0
    {
        return Err(io::Error::last_os_error()).context("Cannot preserve replacement ownership");
    }
    let protection = if let Some(original) = source {
        xattrs(original)?
    } else {
        xattrs(output)?
    };
    for name in xattrs(output)?.keys() {
        if !protection.contains_key(name) {
            let name = CString::new(name.as_slice())?;
            if unsafe { libc::fremovexattr(output.as_raw_fd(), name.as_ptr()) } != 0 {
                return Err(io::Error::last_os_error())
                    .context("Cannot remove inherited replacement metadata");
            }
        }
    }
    // Writing and chown can clear set-id and capabilities. Restore all
    // protection attributes after content/ownership, then apply an explicit
    // mode last so ACL masks follow the requested mode just as chmod does.
    for (name, value) in &protection {
        let name = CString::new(name.as_slice())?;
        if unsafe {
            libc::fsetxattr(
                output.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error())
                .context("Cannot preserve file protection metadata");
        }
    }
    let mode = attributes
        .mode
        .or_else(|| old.map(|metadata| metadata.mode() & 0o7777))
        .unwrap_or(default_mode);
    output.set_permissions(fs::Permissions::from_mode(mode))?;
    let prepared = output.metadata()?;
    if (prepared.uid(), prepared.gid(), prepared.mode() & 0o7777) != (uid, gid, mode) {
        bail!("Filesystem did not preserve replacement ownership/mode");
    }
    let installed_protection = xattrs(output)?;
    for (name, value) in &protection {
        let installed = installed_protection.get(name);
        if (attributes.mode.is_some() || old.is_none()) && name == b"system.posix_acl_access" {
            // chmod may change the ACL's base entries and mask, but must
            // never discard a named user's/group's protection entry.
            if named_acl_entries(value)?
                != installed
                    .map(|value| named_acl_entries(value))
                    .transpose()?
                    .unwrap_or_default()
            {
                bail!("Filesystem did not preserve named ACL entries");
            }
        } else if installed != Some(value) {
            bail!("Filesystem did not preserve replacement protection metadata");
        }
    }
    Ok(())
}

fn named_acl_entries(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    if bytes.len() < 4 || bytes[..4] != 2u32.to_le_bytes() || !(bytes.len() - 4).is_multiple_of(8) {
        bail!("Unsupported Linux POSIX ACL encoding");
    }
    let mut named = Vec::new();
    for entry in bytes[4..].as_chunks::<8>().0 {
        match u16::from_le_bytes([entry[0], entry[1]]) {
            2 | 8 => named.push(entry.as_slice()),
            1 | 4 | 16 | 32 => {}
            _ => bail!("Unsupported Linux POSIX ACL entry"),
        }
    }
    Ok(named)
}

fn xattrs(file: &File) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    let list = xattr_bytes(
        |buffer, size| unsafe { libc::flistxattr(file.as_raw_fd(), buffer.cast(), size) },
        true,
    )?;
    let mut output = BTreeMap::new();
    for name in list
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let terminated = CString::new(name)?;
        let value = xattr_bytes(
            |buffer, size| unsafe {
                libc::fgetxattr(file.as_raw_fd(), terminated.as_ptr(), buffer.cast(), size)
            },
            false,
        )?;
        output.insert(name.to_vec(), value);
    }
    Ok(output)
}
fn xattr_bytes(
    mut read: impl FnMut(*mut u8, usize) -> isize,
    unsupported_is_empty: bool,
) -> Result<Vec<u8>> {
    for _ in 0..8 {
        let size = read(std::ptr::null_mut(), 0);
        if size < 0 {
            let error = io::Error::last_os_error();
            if unsupported_is_empty && error.raw_os_error() == Some(libc::ENOTSUP) {
                return Ok(Vec::new());
            }
            return Err(error).context("Cannot inspect protection metadata");
        }
        if size > 65536 {
            bail!("File protection metadata exceeds Linux's supported bound");
        }
        let mut data = vec![0; size as usize];
        let length = read(data.as_mut_ptr(), data.len());
        if length >= 0 {
            if length as usize > data.len() {
                continue;
            }
            data.truncate(length as usize);
            return Ok(data);
        }
        if io::Error::last_os_error().raw_os_error() != Some(libc::ERANGE) {
            return Err(io::Error::last_os_error()).context("Cannot read protection metadata");
        }
    }
    bail!("File protection metadata changed repeatedly during inspection")
}
