//! Explicit installation/configuration grants native execution. Project-local
//! collections can supply declarative content, but never silently authorize a
//! repository executable. Every selected ELF is pinned and verified again.
pub mod compiled;
mod module;
mod package;
mod trust;
pub use package::{install, pack, verify};
mod template;
use anyhow::{bail, Context, Result};
pub use module::Module;
use rustsible_protocol::extension::{Artifact, Call, Collection, Definition, Kind};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub(crate) use template::register as register_template;
pub const MANIFEST: &str = "rustsible-collection.json";
pub const RECEIPT: &str = ".rustsible-install.json";
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchRoot {
    path: PathBuf,
    native: bool,
    receipt: bool,
}
#[derive(Debug, Default)]
pub struct Registry {
    roots: Vec<SearchRoot>,
    compiled: compiled::Components,
    filters: Vec<Arc<Native>>,
    collections: Mutex<HashMap<String, Arc<Loaded>>>,
}
#[derive(Debug)]
struct Loaded {
    root: PathBuf,
    manifest: Option<Collection>,
    native: bool,
}
#[derive(Debug, Clone)]
pub struct Native {
    pub definition: Definition,
    root: PathBuf,
    artifacts: BTreeMap<String, Artifact>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub format: u32,
    pub collection: String,
    pub manifest_sha256: String,
    pub files: BTreeMap<String, String>,
}
impl PartialEq for Registry {
    fn eq(&self, other: &Self) -> bool {
        self.roots == other.roots
    }
}
impl Eq for Registry {}
thread_local! {static CURRENT:std::cell::RefCell<Arc<Registry>>=std::cell::RefCell::new(Arc::default());}
pub struct Guard {
    prior: Arc<Registry>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.prior.clone());
        });
    }
}
impl Registry {
    pub fn current() -> Arc<Self> {
        CURRENT.with(|current| current.borrow().clone())
    }
    pub fn enter(self: &Arc<Self>) -> Guard {
        Guard {
            prior: CURRENT.with(|current| current.replace(self.clone())),
            _thread: std::marker::PhantomData,
        }
    }
    pub fn new(paths: &[PathBuf], project: &Path) -> Result<Arc<Self>> {
        let mut roots = paths
            .iter()
            .map(|path| SearchRoot {
                path: path.clone(),
                native: true,
                receipt: false,
            })
            .collect::<Vec<_>>();
        roots.push(SearchRoot {
            path: project.join("collections"),
            native: false,
            receipt: false,
        });
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(SearchRoot {
                path: PathBuf::from(home).join(".rustsible/collections"),
                native: true,
                receipt: true,
            });
        }
        let mut registry = Self {
            roots,
            compiled: Default::default(),
            filters: Vec::new(),
            collections: Mutex::default(),
        };
        // Native manifests in authorized roots are checked before task work;
        // arbitrary Galaxy/Python collections are inspected only if selected.
        for id in registry.native_collections()? {
            let loaded = registry.collection(&id)?;
            if let Some(manifest) = &loaded.manifest {
                for entry in &manifest.extensions {
                    if entry.definition.kind == Kind::Filter {
                        registry
                            .filters
                            .push(registry.resolve(&entry.definition.name, Kind::Filter)?);
                    }
                }
            }
        }
        Ok(Arc::new(registry))
    }
    fn native_collections(&self) -> Result<Vec<String>> {
        let mut ids = std::collections::BTreeSet::new();
        for root in self.roots.iter().filter(|root| root.native) {
            let path = root.path.join("ansible_collections");
            if !path.exists() {
                continue;
            }
            for namespace in fs::read_dir(path)? {
                let namespace = namespace?;
                if !namespace.file_type()?.is_dir() {
                    continue;
                }
                for collection in fs::read_dir(namespace.path())? {
                    let collection = collection?;
                    if !collection.file_type()?.is_dir()
                        || !collection.path().join(MANIFEST).is_file()
                    {
                        continue;
                    }
                    let id = format!(
                        "{}.{}",
                        namespace.file_name().to_string_lossy(),
                        collection.file_name().to_string_lossy()
                    );
                    if root.receipt && trust::Trust::read()?.contains(&root.path, &id).is_none() {
                        continue;
                    }
                    ids.insert(id);
                }
            }
        }
        Ok(ids.into_iter().collect())
    }
    fn collection(&self, id: &str) -> Result<Arc<Loaded>> {
        rustsible_protocol::extension::validate_name(&format!("{id}.collection"))?;
        if let Some(loaded) = self
            .collections
            .lock()
            .map_err(|_| anyhow::anyhow!("Collection cache poisoned"))?
            .get(id)
        {
            return Ok(loaded.clone());
        }
        let (namespace, name) = id
            .split_once('.')
            .context("Collection name needs namespace.collection")?;
        for source in &self.roots {
            let path = source
                .path
                .join("ansible_collections")
                .join(namespace)
                .join(name);
            if !path.exists() {
                continue;
            }
            if !fs::symlink_metadata(&path)?.is_dir() {
                bail!("Collection root must be a real directory");
            }
            reject_python(&path)?;
            let manifest = if path.join(MANIFEST).is_file() {
                let manifest: Collection = read_json(&path.join(MANIFEST))?;
                manifest.validate()?;
                if manifest.id() != id {
                    bail!("Collection identity differs from its directory");
                }
                Some(manifest)
            } else {
                None
            };
            let authorization = if source.receipt {
                Some(trust::Trust::read()?)
            } else {
                None
            };
            let expected = authorization
                .as_ref()
                .and_then(|trust| trust.contains(&source.path, id));
            let native = source.native && (!source.receipt || expected.is_some());
            if source.receipt && native {
                let receipt: Receipt = read_json_limit(&path.join(RECEIPT), 64 * 1024 * 1024)
                    .context("Native collection must be explicitly installed")?;
                if receipt.format != 1
                    || expected != Some(receipt.manifest_sha256.as_str())
                    || receipt.collection != id
                    || rustsible_protocol::sha256(&mut File::open(path.join(MANIFEST))?)?
                        != receipt.manifest_sha256
                {
                    bail!("Installed collection receipt/manifest mismatch");
                }
                for (relative, digest) in &receipt.files {
                    let relative = rustsible_protocol::extension::relative_path(relative)?;
                    if rustsible_protocol::sha256(&mut File::open(path.join(relative))?)? != *digest
                    {
                        bail!("Installed collection file checksum mismatch");
                    }
                }
            }
            let loaded = Arc::new(Loaded {
                root: path.canonicalize()?,
                manifest,
                native,
            });
            if native {
                if let Some(manifest) = &loaded.manifest {
                    for entry in &manifest.extensions {
                        let native = Native {
                            definition: entry.definition.clone(),
                            root: loaded.root.clone(),
                            artifacts: entry.artifacts.clone(),
                        };
                        for arch in native.artifacts.keys() {
                            native.open(arch)?;
                        }
                        if native.definition.kind != Kind::Module
                            && !native.artifacts.contains_key(std::env::consts::ARCH)
                        {
                            bail!("Native provider lacks a controller-architecture artifact");
                        }
                    }
                }
            }
            self.collections
                .lock()
                .map_err(|_| anyhow::anyhow!("Collection cache poisoned"))?
                .insert(id.into(), loaded.clone());
            return Ok(loaded);
        }
        bail!("Collection {id} was not found in configured search paths")
    }
    pub fn resolve(&self, name: &str, kind: Kind) -> Result<Arc<Native>> {
        rustsible_protocol::extension::validate_name(name)?;
        let (id, component) = name.rsplit_once('.').context("Invalid extension name")?;
        let loaded = self.collection(id)?;
        let Some(manifest) = &loaded.manifest else {
            bail!("Collection has no native extension manifest; Python modules/plugins are unsupported");
        };
        let entry = manifest
            .extensions
            .iter()
            .find(|entry| entry.definition.name == name && entry.definition.kind == kind)
            .with_context(|| format!("Collection has no native {kind:?} component {component}"))?;
        if !loaded.native {
            bail!("Repository native programs require explicit installation or --collections-path configuration");
        }
        Ok(Arc::new(Native {
            definition: entry.definition.clone(),
            root: loaded.root.clone(),
            artifacts: entry.artifacts.clone(),
        }))
    }
    pub fn module(&self, name: &str, collections: &[String]) -> Result<Module> {
        if let Some(action) = self.compiled.actions.get(name) {
            return Ok(Module::Action(action.clone()));
        }
        if name.contains('.') {
            if let Ok(builtin) = rustsible_protocol::registry::Builtin::lookup(name) {
                return Ok(Module::Builtin(builtin));
            }
            return self.resolve(name, Kind::Module).map(Module::Native);
        }
        for id in collections {
            if matches!(id.as_str(), "ansible.builtin" | "ansible.legacy") {
                if let Ok(builtin) = rustsible_protocol::registry::Builtin::lookup(name) {
                    return Ok(Module::Builtin(builtin));
                }
            } else {
                let loaded = self.collection(id)?;
                if loaded.manifest.as_ref().is_some_and(|manifest| {
                    manifest.extensions.iter().any(|entry| {
                        entry.definition.kind == Kind::Module
                            && entry.definition.name == format!("{id}.{name}")
                    })
                }) {
                    return self
                        .resolve(&format!("{id}.{name}"), Kind::Module)
                        .map(Module::Native);
                }
            }
        }
        rustsible_protocol::registry::Builtin::lookup(name).map(Module::Builtin)
    }
    pub fn role(&self, name: &str) -> Result<PathBuf> {
        let mut parts = name.rsplitn(2, '.');
        let role = parts.next().context("Missing collection role")?;
        let id = parts
            .next()
            .context("Role needs namespace.collection.role")?;
        rustsible_protocol::extension::validate_name(name)?;
        let loaded = self.collection(id)?;
        let path = loaded.root.join("roles").join(role);
        if !path.is_dir() {
            bail!("Collection role was not found");
        }
        Ok(path)
    }
    pub fn playbook(&self, name: &str) -> Result<PathBuf> {
        let mut parts = name.rsplitn(2, '.');
        let playbook = parts.next().context("Missing collection playbook")?;
        let id = parts
            .next()
            .context("Playbook needs namespace.collection.playbook")?;
        rustsible_protocol::extension::validate_name(name)?;
        let loaded = self.collection(id)?;
        for extension in ["yml", "yaml"] {
            let path = loaded
                .root
                .join("playbooks")
                .join(format!("{playbook}.{extension}"));
            if path.is_file() {
                return Ok(path);
            }
        }
        bail!("Collection playbook was not found")
    }
    pub(crate) fn filters(&self) -> &[Arc<Native>] {
        &self.filters
    }
    pub fn invoke(
        &self,
        name: &str,
        kind: Kind,
        args: serde_yaml::Value,
    ) -> Result<rustsible_protocol::ModuleResult> {
        let native = self.resolve(name, kind)?;
        let (file, digest) = native.open(std::env::consts::ARCH)?;
        let path = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            std::os::fd::AsRawFd::as_raw_fd(&file)
        ));
        let call = Call::new(&native.definition, args, true, false, 30000);
        native.definition.validate_call(&call)?;
        let result =
            rustsible_native::rpc::invoke_controller(&rustsible_native::rpc::Invocation {
                program: path,
                sha256: digest,
                definition: native.definition.clone(),
                call,
            })?;
        if result.failed || result.uncertain || result.unreachable {
            bail!("Native provider {name} failed");
        }
        Ok(result)
    }
}
impl Native {
    pub fn open(&self, arch: &str) -> Result<(File, String)> {
        let artifact = self
            .artifacts
            .get(arch)
            .context("Native component does not support the target architecture")?;
        let relative = rustsible_protocol::extension::relative_path(&artifact.path)?;
        let root = self.root.canonicalize()?;
        let path = root.join(relative);
        if !path.canonicalize()?.starts_with(&root) {
            bail!("Native executable path escapes its collection");
        }
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| directory.starts_with(&root)) {
            let metadata = fs::symlink_metadata(directory)?;
            if !metadata.is_dir()
                || (metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() })
                || metadata.mode() & 0o022 != 0
            {
                bail!("Unsafe native executable directory");
            }
            parent = directory.parent();
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        if metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
        {
            bail!("Native executable has unsafe ownership/write permissions");
        }
        rustsible_protocol::extension::verify_binary(
            &mut file,
            &artifact.sha256,
            arch,
            "Native extension",
        )?;
        Ok((file, artifact.sha256.clone()))
    }
}
pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    read_json_limit(path, 1024 * 1024)
}
pub(crate) fn read_json_limit<T: serde::de::DeserializeOwned>(
    path: &Path,
    limit: usize,
) -> Result<T> {
    let mut bytes = Vec::new();
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        bail!("Collection metadata must be a regular file");
    }
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("Collection metadata exceeds 1 MiB");
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid native collection metadata"))
}
fn reject_python(root: &Path) -> Result<()> {
    fn scan(path: &Path, depth: usize) -> Result<()> {
        if depth > 64 {
            bail!("Collection plugin directory exceeds depth limit");
        }
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            bail!("Collection plugin links are unsupported");
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                bail!("Collection plugin links are unsupported");
            }
            if kind.is_dir() {
                scan(&entry.path(), depth + 1)?;
            } else if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "py")
            {
                bail!("Python collection modules/plugins are unsupported; use native executable extensions");
            }
        }
        Ok(())
    }
    scan(&root.join("plugins"), 0)?;
    scan(&root.join("library"), 0)?;
    Ok(())
}

thread_local! {static SEARCH:std::cell::RefCell<Vec<String>>=const{std::cell::RefCell::new(Vec::new())};}
pub(crate) struct SearchGuard {
    prior: Vec<String>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for SearchGuard {
    fn drop(&mut self) {
        SEARCH.with(|search| {
            search.replace(self.prior.clone());
        });
    }
}
pub(crate) fn search() -> Vec<String> {
    SEARCH.with(|search| search.borrow().clone())
}
pub(crate) fn enter_search(names: &[String]) -> SearchGuard {
    SearchGuard {
        prior: SEARCH.with(|search| search.replace(names.to_vec())),
        _thread: std::marker::PhantomData,
    }
}
pub(crate) fn parse_search(value: Option<&serde_yaml::Value>) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_sequence()
        .context("collections must be a list of literal namespace.collection names")?;
    values
        .iter()
        .map(|value| {
            let name = value.as_str().context("Collection names must be strings")?;
            if !matches!(name, "ansible.builtin" | "ansible.legacy") {
                Registry::current().collection(name)?;
            }
            Ok(name.to_owned())
        })
        .collect()
}
impl Registry {
    pub(crate) fn owning_collection(&self, path: &Path) -> Option<String> {
        let path = path.canonicalize().ok()?;
        self.collections
            .lock()
            .ok()?
            .iter()
            .filter(|(_, loaded)| path.starts_with(loaded.root.join("roles")))
            .max_by_key(|(_, loaded)| loaded.root.as_os_str().len())
            .map(|(id, _)| id.clone())
    }
}

pub fn run_cli(matches: &clap::ArgMatches) -> Result<()> {
    let (operation, args) = matches
        .subcommand()
        .context("Collection operation required")?;
    let input = args
        .get_one::<PathBuf>("input")
        .context("Collection input required")?;
    match operation {
        "pack" => pack(
            input,
            args.get_one::<PathBuf>("output")
                .context("Collection output required")?,
        ),
        "verify" => {
            let metadata = verify(input)?;
            println!(
                "{}",
                serde_json::json!({"collection":metadata.id(),"version":metadata.version,"verified":true})
            );
            Ok(())
        }
        "install" => {
            let root = args
                .get_one::<PathBuf>("path")
                .cloned()
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|home| PathBuf::from(home).join(".rustsible/collections"))
                })
                .context("HOME or an explicit --path is required")?;
            install(input, &root)
        }
        _ => bail!("Unsupported collection operation"),
    }
}
