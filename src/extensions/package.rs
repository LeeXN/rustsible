use super::*;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    io,
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, PermissionsExt},
        },
    },
};
const INDEX: &str = "FILES.json";
const MAX_INDEX: usize = 64 * 1024 * 1024;
fn list(root: &Path, relative: &Path, files: &mut Vec<PathBuf>, pack: bool) -> Result<()> {
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let path = relative.join(entry.file_name());
        if pack
            && relative.as_os_str().is_empty()
            && matches!(
                entry.file_name().to_str(),
                Some(".git" | "target" | RECEIPT | INDEX)
            )
        {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() || !kind.is_file() && !kind.is_dir() {
            bail!("Native collection links/devices are unsupported");
        }
        if kind.is_dir() {
            list(root, &path, files, pack)?;
        } else {
            rustsible_protocol::extension::relative_path(
                path.to_str().context("Collection paths must be UTF-8")?,
            )?;
            files.push(path);
            if files.len() > 100000 {
                bail!("Collection exceeds 100000 files");
            }
        }
    }
    Ok(())
}
struct HashReader {
    file: File,
    hash: Sha256,
}
impl Read for HashReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.file.read(output)?;
        self.hash.update(&output[..count]);
        Ok(count)
    }
}
pub fn pack(root: &Path, output: &Path) -> Result<()> {
    rustsible_protocol::change::ensure_mutation_allowed()?;
    if output.exists() {
        bail!("Collection package output already exists");
    }
    let metadata: Collection = read_json(&root.join(MANIFEST))?;
    metadata.validate()?;
    super::reject_python(root)?;
    for entry in &metadata.extensions {
        let native = Native {
            definition: entry.definition.clone(),
            root: root.to_owned(),
            artifacts: entry.artifacts.clone(),
        };
        for arch in native.artifacts.keys() {
            native.open(arch)?;
        }
    }
    let expected_manifest = rustsible_protocol::sha256(&mut File::open(root.join(MANIFEST))?)?;
    let mut files = Vec::new();
    list(root, Path::new(""), &mut files, true)?;
    files.sort();
    if files.iter().any(|path| python_plugin(path)) {
        bail!("Python collection modules/plugins are unsupported");
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".rustsible-package-")
        .tempfile_in(parent)?;
    let gzip =
        flate2::write::GzEncoder::new(temporary.as_file_mut(), flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    let mut hashes = BTreeMap::new();
    let mut total = 0u64;
    for relative in &files {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join(relative))?;
        let original = file.metadata()?;
        if !original.is_file() {
            bail!("Collection source changed into a non-regular file");
        }
        total = total
            .checked_add(original.len())
            .context("Package size overflow")?;
        if total > rustsible_native::archive::MAX_TOTAL {
            bail!("Collection exceeds 8 GiB");
        }
        let mut reader = HashReader {
            file,
            hash: Sha256::new(),
        };
        let mut header = tar::Header::new_gnu();
        header.set_size(original.len());
        header.set_mode(original.mode() & 0o777);
        header.set_cksum();
        archive.append_data(&mut header, relative, &mut reader)?;
        let mut extra = [0u8; 1];
        if reader.read(&mut extra)? != 0 {
            bail!("Collection source grew during packaging");
        }
        let after = reader.file.metadata()?;
        if (
            original.ino(),
            original.len(),
            original.mtime(),
            original.mtime_nsec(),
            original.ctime(),
            original.ctime_nsec(),
        ) != (
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        ) {
            bail!("Collection source changed during packaging");
        }
        hashes.insert(
            relative.to_str().unwrap().to_owned(),
            format!("{:x}", reader.hash.finalize()),
        );
    }
    if hashes.get(MANIFEST) != Some(&expected_manifest) {
        bail!("Collection manifest changed during packaging");
    }
    for entry in &metadata.extensions {
        for artifact in entry.artifacts.values() {
            if !hashes
                .get(&artifact.path)
                .is_some_and(|actual: &String| actual.eq_ignore_ascii_case(&artifact.sha256))
            {
                bail!("Collection native artifact changed during packaging");
            }
        }
    }
    let index = serde_json::to_vec(&hashes)?;
    if index.len() > MAX_INDEX {
        bail!("Collection file index exceeds 64 MiB");
    }
    let mut header = tar::Header::new_gnu();
    header.set_size(index.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    archive.append_data(&mut header, INDEX, index.as_slice())?;
    archive.into_inner()?.finish()?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(output)
        .map_err(|_| anyhow::anyhow!("Cannot publish collection package atomically"))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
fn index(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_INDEX as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_INDEX {
        bail!("Collection index exceeds 64 MiB");
    }
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid collection file index"))
}
fn verify_previous(path: &Path, id: &str) -> Result<()> {
    let receipt: Receipt = read_json_limit(&path.join(RECEIPT), MAX_INDEX)
        .context("Refusing to replace an unmanaged collection directory")?;
    if receipt.format != 1 || receipt.collection != id {
        bail!("Existing collection receipt identity mismatch");
    }
    let mut allowed = std::collections::HashSet::new();
    for file in receipt.files.keys() {
        let relative = rustsible_protocol::extension::relative_path(file)?;
        for parent in relative
            .ancestors()
            .skip(1)
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            allowed.insert(parent.to_owned());
        }
    }
    fn directories(
        root: &Path,
        relative: &Path,
        allowed: &std::collections::HashSet<PathBuf>,
    ) -> Result<()> {
        for entry in fs::read_dir(root.join(relative))? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let child = relative.join(entry.file_name());
                if !allowed.contains(&child) {
                    bail!("Refusing to replace unmanaged collection directories");
                }
                directories(root, &child, allowed)?;
            }
        }
        Ok(())
    }
    directories(path, Path::new(""), &allowed)?;
    let mut actual = Vec::new();
    list(path, Path::new(""), &mut actual, false)?;
    for relative in actual {
        if relative == Path::new(RECEIPT) {
            continue;
        }
        let name = relative.to_str().unwrap();
        if receipt.files.get(name)
            != Some(&rustsible_protocol::sha256(&mut File::open(
                path.join(relative),
            )?)?)
        {
            bail!("Refusing to replace modified/unmanaged collection files");
        }
    }
    for (relative, digest) in &receipt.files {
        let relative = rustsible_protocol::extension::relative_path(relative)?;
        if rustsible_protocol::sha256(&mut File::open(path.join(relative))?)? != *digest {
            bail!("Existing collection is incomplete or modified");
        }
    }
    Ok(())
}
struct Validated {
    _scratch: tempfile::TempDir,
    entries: BTreeMap<PathBuf, rustsible_native::archive::Entry>,
    metadata: Collection,
    hashes: BTreeMap<String, String>,
}
fn unpack(package: &Path) -> Result<Validated> {
    let input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(package)?;
    if !input.metadata()?.is_file()
        || input.metadata()?.len() > rustsible_native::archive::MAX_TOTAL
    {
        bail!("Collection package must be a regular file no larger than 8 GiB");
    }
    let scratch = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()?;
    let entries = rustsible_native::archive::decode(input, scratch.path())?;
    let file = |name: &str| -> Result<&Path> {
        entries
            .get(Path::new(name))
            .and_then(|entry| entry.staged.as_deref())
            .context("Collection package lacks required metadata")
    };
    let metadata: Collection = read_json(file(MANIFEST)?)?;
    metadata.validate()?;
    let hashes = index(file(INDEX)?)?;
    for relative in entries.keys() {
        if python_plugin(relative) {
            bail!("Python collection modules/plugins are unsupported");
        }
    }
    for (name, entry) in &entries {
        let Some(staged) = &entry.staged else {
            continue;
        };
        let name = name.to_str().context("Invalid collection path")?;
        if name == INDEX {
            continue;
        }
        if name == RECEIPT
            || hashes.get(name) != Some(&rustsible_protocol::sha256(&mut File::open(staged)?)?)
        {
            bail!("Collection file checksum mismatch or undeclared file");
        }
    }
    for (relative, digest) in &hashes {
        rustsible_protocol::extension::relative_path(relative)?;
        if relative == INDEX
            || relative == RECEIPT
            || rustsible_protocol::sha256(&mut File::open(file(relative)?)?)? != *digest
        {
            bail!("Invalid collection file declaration");
        }
    }
    for entry in &metadata.extensions {
        for (arch, artifact) in &entry.artifacts {
            let mut file = File::open(file(&artifact.path)?)?;
            rustsible_protocol::extension::verify_binary(
                &mut file,
                &artifact.sha256,
                arch,
                "Native extension",
            )?;
        }
    }
    Ok(Validated {
        _scratch: scratch,
        entries,
        metadata,
        hashes,
    })
}
pub fn verify(package: &Path) -> Result<Collection> {
    Ok(unpack(package)?.metadata)
}
pub fn install(package: &Path, root: &Path) -> Result<()> {
    rustsible_protocol::change::ensure_mutation_allowed()?;
    let Validated {
        _scratch,
        entries,
        metadata,
        hashes,
    } = unpack(package)?;
    let namespace = root.join("ansible_collections").join(&metadata.namespace);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&namespace)?;
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(&namespace)?;
    rustsible_native::protection::ensure_safe_directory(&parent)?;
    let stage = tempfile::Builder::new()
        .prefix(".rustsible-collection-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(format!("/proc/self/fd/{}", parent.as_raw_fd()))?;
    let prepared = stage.path().join("content");
    fs::create_dir(&prepared)?;
    fs::set_permissions(&prepared, fs::Permissions::from_mode(0o700))?;
    for (relative, entry) in &entries {
        let Some(source) = &entry.staged else {
            continue;
        };
        if relative == Path::new(INDEX) {
            continue;
        }
        let target = prepared.join(relative);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(target.parent().unwrap())?;
        fs::copy(source, &target)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
        File::open(&target)?.sync_all()?;
    }
    for entry in &metadata.extensions {
        for artifact in entry.artifacts.values() {
            fs::set_permissions(
                prepared.join(&artifact.path),
                fs::Permissions::from_mode(0o700),
            )?;
        }
    }
    super::reject_python(&prepared)?;
    let manifest_sha256 = rustsible_protocol::sha256(&mut File::open(prepared.join(MANIFEST))?)?;
    let receipt = Receipt {
        format: 1,
        collection: metadata.id(),
        manifest_sha256: manifest_sha256.clone(),
        files: hashes,
    };
    fs::write(prepared.join(RECEIPT), serde_json::to_vec(&receipt)?)?;
    fs::set_permissions(prepared.join(RECEIPT), fs::Permissions::from_mode(0o600))?;
    let mut trust = super::trust::Trust::write(root)?;
    let destination =
        PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd())).join(&metadata.name);
    let exists = match fs::symlink_metadata(&destination) {
        Ok(stat) => {
            if !stat.is_dir() {
                bail!("Existing collection destination must be a directory");
            }
            verify_previous(&destination, &metadata.id())?;
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let from = CString::new(prepared.as_os_str().as_bytes())?;
    let name = CString::new(metadata.name.as_str())?;
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            parent.as_raw_fd(),
            name.as_ptr(),
            if exists { 2u32 } else { 1u32 },
        )
    } != 0
    {
        return Err(io::Error::last_os_error())
            .context("Safe atomic collection directory replacement failed");
    }
    rustsible_protocol::change::record_change();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(format!("/proc/self/fd/{}", parent.as_raw_fd()))?
        .sync_all()?;
    trust
        .authorize(root, &metadata.id(), &manifest_sha256)
        .context("Collection installed, but native authorization registration failed")?;
    Ok(())
}

fn python_plugin(path: &Path) -> bool {
    if path.extension().is_none_or(|extension| extension != "py") {
        return false;
    }
    let parts = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    matches!(
        parts.first().map(|part| part.as_ref()),
        Some("plugins" | "library")
    ) || parts.len() > 3
        && parts[0] == "roles"
        && matches!(
            parts[2].as_ref(),
            "library"
                | "filter_plugins"
                | "lookup_plugins"
                | "action_plugins"
                | "callback_plugins"
                | "inventory_plugins"
                | "connection_plugins"
                | "become_plugins"
                | "strategy_plugins"
        )
}
