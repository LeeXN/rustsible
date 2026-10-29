//! The HTTP client is constructed inside the target module child. Target DNS,
//! proxy variables and native certificate roots therefore remain authoritative.
use crate::filesystem::{Attributes, Destination};
use anyhow::{bail, Context, Result};
use rustsible_protocol::{
    param::{get_optional_param, get_param, validate_params},
    ModuleResult, Request,
};
use serde_yaml::Value;
use std::{
    io::{self, Read, Seek},
    time::Duration,
};

const MAX_URI_BODY: usize = 8 * 1024 * 1024;

fn client(
    timeout_ms: u64,
    args: Option<&Value>,
    method: &str,
) -> Result<reqwest::blocking::Client> {
    let option = |name: &str| args.and_then(|args| args.get(name));
    let redirects = option("follow_redirects")
        .and_then(Value::as_str)
        .unwrap_or("safe");
    let follow = redirects == "all" || redirects == "safe" && matches!(method, "GET" | "HEAD");
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(if follow {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        })
        .connect_timeout(Duration::from_millis(timeout_ms).min(Duration::from_secs(30)))
        .timeout(Duration::from_millis(timeout_ms))
        .danger_accept_invalid_certs(
            option("validate_certs").and_then(Value::as_bool) == Some(false),
        );
    if option("use_proxy").and_then(Value::as_bool) == Some(false) {
        builder = builder.no_proxy();
    }
    if let Some(path) = option("ca_path").and_then(Value::as_str) {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .context("Cannot open target CA file")?;
        if !file.metadata()?.is_file() || file.metadata()?.len() > 4 * 1024 * 1024 {
            bail!("Target CA file must be a regular file no larger than 4 MiB");
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 4 * 1024 * 1024 {
            bail!("Target CA file exceeded its size bound");
        }
        let certificates = reqwest::Certificate::from_pem_bundle(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid target CA PEM bundle"))?;
        if certificates.is_empty() {
            bail!("Target CA PEM bundle is empty");
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder
        .build()
        .map_err(|error| anyhow::anyhow!(error.without_url()))
}

pub fn uri(request: &Request) -> Result<ModuleResult> {
    rustsible_protocol::registry::Builtin::Uri.validate_parameters(&request.args)?;
    if request.check {
        return Ok(ModuleResult {
            skipped: true,
            msg: "Check mode: URI request skipped".into(),
            ..Default::default()
        });
    }
    let args = &request.args;
    let raw_url = args
        .get("url")
        .and_then(Value::as_str)
        .context("URI requires URL")?;
    let mut url = reqwest::Url::parse(raw_url).map_err(|_| anyhow::anyhow!("Invalid URI URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("URI requires HTTP or HTTPS");
    }
    let method_name = args
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let method = reqwest::Method::from_bytes(method_name.as_bytes())
        .map_err(|_| anyhow::anyhow!("Invalid HTTP method"))?;
    let url_credentials = !url.username().is_empty() || url.password().is_some();
    let request_url = url.clone();
    let username = args
        .get("url_username")
        .or_else(|| args.get("user"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let password = args
        .get("url_password")
        .or_else(|| args.get("password"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let timeout_ms = args
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .saturating_mul(1000)
        .min(request.timeout_ms);
    let mut headers = reqwest::header::HeaderMap::new();
    let mut private = password.is_some() || username.is_some() || url_credentials;
    if let Some(values) = args.get("headers").and_then(Value::as_mapping) {
        for (name, value) in values {
            let name = reqwest::header::HeaderName::from_bytes(
                name.as_str()
                    .context("HTTP header name must be a string")?
                    .as_bytes(),
            )
            .map_err(|_| anyhow::anyhow!("Invalid HTTP header name"))?;
            private |= matches!(
                name.as_str(),
                "authorization" | "proxy-authorization" | "cookie"
            );
            let value = reqwest::header::HeaderValue::from_str(
                value
                    .as_str()
                    .context("HTTP header value must be a string")?,
            )
            .map_err(|_| anyhow::anyhow!("Invalid HTTP header value"))?;
            headers.insert(name, value);
        }
    }
    // Validate and encode the complete body before building/sending a request.
    let body = if let Some(body) = args.get("body") {
        let bytes = match args
            .get("body_format")
            .and_then(Value::as_str)
            .unwrap_or("raw")
        {
            "raw" => body
                .as_str()
                .context("Raw URI body must be a string")?
                .as_bytes()
                .to_vec(),
            "json" => {
                headers.entry(reqwest::header::CONTENT_TYPE).or_insert(
                    reqwest::header::HeaderValue::from_static("application/json"),
                );
                if let Some(text) = body.as_str() {
                    let _: serde_json::Value = serde_json::from_str(text)
                        .map_err(|_| anyhow::anyhow!("Invalid JSON URI body"))?;
                    text.as_bytes().to_vec()
                } else {
                    serde_json::to_vec(body)?
                }
            }
            "form-urlencoded" => {
                headers.entry(reqwest::header::CONTENT_TYPE).or_insert(
                    reqwest::header::HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
                let fields = body
                    .as_mapping()
                    .context("URI form body must be a mapping of strings")?;
                let mut encoded = reqwest::Url::parse("http://form.invalid/")?;
                {
                    let mut pairs = encoded.query_pairs_mut();
                    for (key, value) in fields {
                        pairs.append_pair(
                            key.as_str().context("URI form keys must be strings")?,
                            value.as_str().context("URI form values must be strings")?,
                        );
                    }
                }
                encoded.query().unwrap_or("").as_bytes().to_vec()
            }
            _ => bail!("Unsupported URI body format"),
        };
        if bytes.len() > MAX_URI_BODY {
            bail!("URI request body exceeds 8 MiB");
        }
        Some(bytes)
    } else {
        None
    };
    let client = client(timeout_ms, Some(args), &method_name)?;
    let mut outgoing = client
        .request(
            method,
            if url_credentials {
                request_url
            } else {
                url.clone()
            },
        )
        .headers(headers);
    if let Some(body) = body {
        outgoing = outgoing.body(body);
    }
    let preemptive =
        args.get("force_basic_auth").and_then(Value::as_bool) == Some(true) || url_credentials;
    let challenge = if !preemptive && username.is_some() {
        outgoing.try_clone()
    } else {
        None
    };
    if preemptive {
        if let Some(username) = &username {
            outgoing = outgoing.basic_auth(username, password.as_deref());
        }
    }
    let mut response = outgoing
        .send()
        .map_err(|error| anyhow::anyhow!(error.without_url()))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        && response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("basic"))
    {
        if let (Some(challenge), Some(username)) = (challenge, &username) {
            response = challenge
                .basic_auth(username, password.as_deref())
                .send()
                .map_err(|error| anyhow::anyhow!(error.without_url()))?;
        }
    }
    let mut final_url = response.url().clone();
    let _ = final_url.set_username("");
    let _ = final_url.set_password(None);
    let status = response.status().as_u16();
    let media_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if response
        .content_length()
        .is_some_and(|size| size > MAX_URI_BODY as u64)
    {
        bail!("URI response body exceeds 8 MiB");
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_URI_BODY as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(body_error)?;
    if bytes.len() > MAX_URI_BODY {
        bail!("URI response body exceeds 8 MiB");
    }
    let expected = match args.get("status_code") {
        None => status == 200,
        Some(Value::Sequence(values)) => values
            .iter()
            .any(|value| value.as_u64() == Some(status as u64)),
        Some(value) => value.as_u64() == Some(status as u64),
    };
    let mut result = ModuleResult {
        failed: !expected,
        no_log: private,
        msg: if expected {
            format!("HTTP status {status}")
        } else {
            format!("Unexpected HTTP status {status}")
        },
        ..Default::default()
    };
    result
        .values
        .insert("status".into(), Value::Number(status.into()));
    result
        .values
        .insert("url".into(), Value::String(final_url.to_string()));
    if args.get("return_content").and_then(Value::as_bool) == Some(true) || !expected {
        result.values.insert(
            "content".into(),
            Value::String(
                std::str::from_utf8(&bytes)
                    .context("URI return_content requires UTF-8 text")?
                    .to_owned(),
            ),
        );
    }
    if media_type == "application/json" || media_type.ends_with("+json") {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            result
                .values
                .insert("json".into(), serde_yaml::to_value(value)?);
        }
    }
    Ok(result)
}

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
    let client = client(request.timeout_ms, None, "GET")?;
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
