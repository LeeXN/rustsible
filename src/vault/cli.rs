//! Password providers are data files or a controlling-terminal prompt. An
//! executable provider is rejected, never executed or passed to a shell.
use super::{Secret, Vault};
use anyhow::{bail, Context, Result};
use clap::ArgMatches;
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroizing;
impl Vault {
    pub fn from_cli(matches: &ArgMatches) -> Result<Arc<Self>> {
        // Clap propagates global append arguments by replacement between
        // subcommands. Collect independent scopes instead, in argv order.
        let mut ordered = Vec::new();
        let mut scope = Some(matches);
        while let Some(matches) = scope {
            let mut sources = Vec::new();
            if let Some(paths) = matches.get_many::<PathBuf>("vault-password-file") {
                for (index, path) in matches
                    .indices_of("vault-password-file")
                    .into_iter()
                    .flatten()
                    .zip(paths)
                {
                    sources.push((index, None, Some(path.clone())));
                }
            }
            if let Some(ids) = matches.get_many::<String>("vault-id") {
                for (index, spec) in matches
                    .indices_of("vault-id")
                    .into_iter()
                    .flatten()
                    .zip(ids)
                {
                    let (id, path) = source(spec)?;
                    sources.push((index, id, path));
                }
            }
            if matches.get_flag("ask-vault-pass") {
                sources.push((
                    matches.index_of("ask-vault-pass").unwrap_or(usize::MAX),
                    None,
                    None,
                ));
            }
            sources.sort_by_key(|source| source.0);
            ordered.extend(sources);
            scope = matches.subcommand().map(|(_, sub)| sub);
        }
        if ordered.len() > 32 {
            bail!("At most 32 Vault password sources are supported");
        }
        let secrets = ordered
            .into_iter()
            .map(|(_, id, path)| {
                let password = password(path.as_deref(), id.as_deref())?;
                Ok(Secret { id, password })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Arc::new(Self::from_secrets(secrets)?))
    }
    pub fn run_cli(self: &Arc<Self>, matches: &ArgMatches) -> Result<()> {
        let _scope = self.enter();
        let _changes = rustsible_protocol::change::ChangeScope::enter(false);
        self.protect((|| {
            let (operation, args) = matches
                .subcommand()
                .context("Vault operation is required")?;
            let path = args
                .get_one::<PathBuf>("input")
                .context("Vault input is required")?;
            let input = super::files::Input::read(path)?;
            if operation == "encrypt" && Self::is_encrypted(&input.bytes) {
                bail!("Vault input is already encrypted");
            }
            if operation != "encrypt" && !Self::is_encrypted(&input.bytes) {
                bail!("Vault input is not encrypted");
            }
            let active = if self.has_passwords() {
                self.clone()
            } else {
                Arc::new(Self::from_secrets(vec![Secret {
                    id: None,
                    password: password(None, None)?,
                }])?)
            };
            let _active = active.enter();

            let output = args
                .try_get_one::<PathBuf>("output")
                .ok()
                .flatten()
                .map(PathBuf::as_path);
            let id = args
                .try_get_one::<String>("encrypt-vault-id")
                .ok()
                .flatten()
                .map(String::as_str);
            match operation {
                "encrypt" => {
                    let encrypted = active.encrypt(&input.bytes, id)?;
                    input.write(path, output, &encrypted)?;
                }
                "decrypt" | "view" => {
                    let plaintext = active.decrypt(&input.bytes)?;
                    input.write(
                        path,
                        if operation == "view" {
                            Some(Path::new("-"))
                        } else {
                            output
                        },
                        &plaintext,
                    )?;
                }
                "rekey" => {
                    let plaintext = active.decrypt(&input.bytes)?;
                    let old_id = std::str::from_utf8(&input.bytes)
                        .ok()
                        .and_then(|text| text.lines().next())
                        .and_then(|header| header.trim_end_matches('\r').split(';').nth(3))
                        .map(str::to_owned);
                    let mut secrets = Vec::new();
                    if let Some(paths) = args.get_many::<PathBuf>("new-vault-password-file") {
                        for path in paths {
                            secrets.push(Secret {
                                id: old_id.clone(),
                                password: password(Some(path), old_id.as_deref())?,
                            });
                        }
                    }
                    if let Some(specs) = args.get_many::<String>("new-vault-id") {
                        for spec in specs {
                            let (id, path) = source(spec)?;
                            secrets.push(Secret {
                                id: id.clone(),
                                password: password(path.as_deref(), id.as_deref())?,
                            });
                        }
                    }
                    if secrets.is_empty() {
                        secrets.push(Secret {
                            id: old_id.clone(),
                            password: password(None, old_id.as_deref())?,
                        });
                    }
                    let new = Self::from_secrets(secrets)?;
                    let encrypted = new.encrypt(&plaintext, id)?;
                    input.write(path, output, &encrypted)?;
                }
                _ => bail!("Unsupported Vault operation"),
            }
            Ok(())
        })())
    }
}
fn source(spec: &str) -> Result<(Option<String>, Option<PathBuf>)> {
    let (id, provider) = if let Some((id, provider)) = spec.split_once('@') {
        let id = (!id.is_empty()).then(|| id.to_owned());
        if let Some(id) = &id {
            super::validate_id(id)?;
        }
        (id, provider)
    } else {
        (None, spec)
    };
    if provider.is_empty() {
        bail!("Vault password provider must not be empty");
    }
    Ok((
        id,
        if provider == "prompt" {
            None
        } else {
            Some(PathBuf::from(provider))
        },
    ))
}
fn password(path: Option<&Path>, id: Option<&str>) -> Result<Zeroizing<Vec<u8>>> {
    if let Some(path) = path {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .context("Cannot open Vault password file")?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.mode() & 0o111 != 0 {
            bail!("Vault password providers must be non-executable regular files; scripts are unsupported");
        }
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(65537).read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            bail!("Vault password file exceeds 64 KiB");
        }
        let trimmed = bytes.trim_ascii();
        if trimmed.is_empty() {
            bail!("Vault password must not be empty");
        }
        Ok(Zeroizing::new(trimmed.to_vec()))
    } else {
        Ok(Zeroizing::new(
            crate::ssh::credentials::prompt(&format!(
                "Vault {}password: ",
                id.map(|id| format!("{id} ")).unwrap_or_default()
            ))?
            .into_bytes(),
        ))
    }
}
