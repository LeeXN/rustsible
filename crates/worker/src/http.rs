//! The HTTP client is constructed inside the target module child. Target DNS,
//! proxy variables and native certificate roots therefore remain authoritative.
use crate::filesystem::{Attributes, Destination};
use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    param::{get_optional_param, get_param, validate_params},
    ModuleResult, Request,
};
use std::{
    io::{self, Seek},
    time::Duration,
};

pub fn get_url(request: &Request) -> Result<ModuleResult> {
    let args = &request.args;
    validate_params(
        args,
        &["url", "dest", "owner", "group", "mode", "force", "checksum"],
    )?;
    let attributes = Attributes::parse(args)?;
    let raw_url = get_param::<String>(args, "url")?;
    let url = reqwest::Url::parse(&raw_url).map_err(|_| anyhow::anyhow!("Invalid HTTP URL"))?;
    if !["http", "https"].contains(&url.scheme()) || url.host_str().is_none() {
        bail!("Expected an HTTP or HTTPS URL");
    }
    let dest = get_param::<String>(args, "dest")?;
    let force = get_optional_param::<bool>(args, "force")?.unwrap_or(false);
    let checksum = get_optional_param::<String>(args, "checksum")?;
    let expected = checksum
        .as_deref()
        .map(|checksum| -> Result<(&str, &str)> {
            let (algorithm, value) = checksum
                .split_once(':')
                .context("checksum must use algorithm:hex format")?;
            if value.len() != rustsible_protocol::digest_hex_length(algorithm)?
                || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                bail!("Invalid inline download checksum");
            }
            Ok((algorithm, value))
        })
        .transpose()?;
    let mut destination = Destination::open(&dest)?;
    let mut skip = destination.file.is_some() && !force;
    if skip {
        if let Some((algorithm, value)) = expected {
            let file = destination
                .file
                .as_mut()
                .context("Missing existing download")?;
            file.rewind()?;
            skip = crate::modules::hash(file, algorithm)?.eq_ignore_ascii_case(value);
        }
    }
    let metadata_changed = destination
        .original
        .as_ref()
        .is_some_and(|metadata| attributes.differs(metadata));
    if skip {
        let file = destination
            .file
            .as_mut()
            .context("Missing retained download")?;
        file.rewind()?;
        let checksum_src = crate::modules::hash(file, "sha1")?;
        destination.unchanged()?;
        if metadata_changed {
            destination.location.ensure_safe_parent()?;
            if !request.check {
                destination.set_metadata(attributes)?;
            }
        }
        return result(metadata_changed, &dest, Some(checksum_src));
    }
    destination.location.ensure_safe_parent()?;
    // Validate and allocate a private destination-side inode before network IO.
    // Check mode uses HEAD and never creates destination staging entries.
    let prepared = if request.check {
        None
    } else {
        Some(destination.prepare()?)
    };
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_millis(request.timeout_ms).min(Duration::from_secs(30)))
        .timeout(Duration::from_millis(request.timeout_ms))
        .build()
        .map_err(|error| anyhow::anyhow!(error.without_url()))?;
    let mut response = if request.check {
        client.head(url)
    } else {
        client.get(url)
    }
    .send()
    .map_err(|error| anyhow::anyhow!(error.without_url()))?;
    if !response.status().is_success() {
        bail!(
            "HTTP request failed with status {}",
            response.status().as_u16()
        );
    }
    if request.check {
        return result(true, &dest, None);
    }
    let mut prepared = prepared.context("Missing download staging file")?;
    let length = response.content_length();
    let received = io::copy(&mut response, &mut prepared.file).map_err(body_error)?;
    if length.is_some_and(|length| length != received) {
        bail!("Incomplete HTTP response body");
    }
    if let Some((algorithm, value)) = expected {
        prepared.file.rewind()?;
        if !crate::modules::hash(&mut prepared.file, algorithm)?.eq_ignore_ascii_case(value) {
            bail!("Downloaded file checksum does not match the expected checksum");
        }
    }
    prepared.file.rewind()?;
    let checksum_src = crate::modules::hash(&mut prepared.file, "sha1")?;
    let content_changed = !destination.equals(&mut prepared.file)?;
    let changed = metadata_changed || content_changed;
    destination.unchanged()?;
    if content_changed {
        destination.commit(prepared, attributes)?;
    } else if metadata_changed {
        destination.set_metadata(attributes)?;
    }
    result(changed, &dest, Some(checksum_src))
}
fn result(changed: bool, dest: &str, checksum: Option<String>) -> Result<ModuleResult> {
    let mut result = ModuleResult {
        changed,
        msg: "Download is current or installed atomically".into(),
        ..ModuleResult::default()
    };
    result.values.insert("dest".into(), dest.into());
    if let Some(checksum) = checksum {
        result.values.insert("checksum_src".into(), checksum.into());
    }
    Ok(result)
}

fn body_error(error: io::Error) -> anyhow::Error {
    // Preserve useful IO/TLS causes without reflecting credential-bearing URLs.
    if let Some(code) = error.raw_os_error() {
        return anyhow::Error::from(io::Error::from_raw_os_error(code))
            .context("Cannot write complete HTTP body");
    }
    let kind = error.kind();
    if let Some(inner) = error.into_inner() {
        if let Ok(error) = inner.downcast::<reqwest::Error>() {
            return anyhow::Error::from((*error).without_url())
                .context("Cannot read complete HTTP body");
        }
    }
    anyhow::anyhow!("Cannot transfer complete HTTP body ({kind})")
}
