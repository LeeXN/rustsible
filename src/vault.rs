//! Ansible Vault 1.1/1.2 AES256 envelopes use RustCrypto primitives: PBKDF2
//! derives the AES/HMAC keys and CTR IV; HMAC is verified before decrypting or
//! checking padding. Passwords and intermediate key/plaintext buffers zeroize
//! on drop. Ciphertext framing and errors never echo decrypted content.
mod cli;
mod files;
use aes::cipher::{KeyIvInit, StreamCipher};
use anyhow::{bail, Context, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use zeroize::Zeroizing;
pub const MAX_PLAINTEXT: usize = 16 * 1024 * 1024;
pub const MAX_ENVELOPE: usize = 80 * 1024 * 1024;
type Cipher = ctr::Ctr128BE<aes::Aes256>;
#[derive(PartialEq, Eq)]
struct Secret {
    id: Option<String>,
    password: Zeroizing<Vec<u8>>,
}
#[derive(Clone, Default)]
pub struct Vault {
    secrets: Arc<Vec<Secret>>,
    used: Arc<AtomicBool>,
}
impl fmt::Debug for Vault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Vault")
            .field("password_sources", &self.secrets.len())
            .field("protected_input", &self.used())
            .finish()
    }
}
impl PartialEq for Vault {
    fn eq(&self, other: &Self) -> bool {
        self.secrets == other.secrets
    }
}
impl Eq for Vault {}
impl Vault {
    pub fn new(secrets: Vec<(Option<String>, Vec<u8>)>) -> Result<Self> {
        let values = secrets
            .into_iter()
            .map(|(id, password)| Secret {
                id,
                password: Zeroizing::new(password),
            })
            .collect::<Vec<_>>();
        Self::from_secrets(values)
    }
    fn from_secrets(values: Vec<Secret>) -> Result<Self> {
        if values.len() > 32 {
            bail!("At most 32 Vault password sources are supported");
        }
        for secret in &values {
            if secret.password.is_empty() || secret.password.len() > 65536 {
                bail!("Vault passwords must contain 1 to 65536 bytes");
            }
            if let Some(id) = &secret.id {
                validate_id(id)?;
            }
        }
        Ok(Self {
            secrets: Arc::new(values),
            used: Arc::default(),
        })
    }
    pub(crate) fn protect_result(&self, result: &mut rustsible_protocol::ModuleResult) {
        if self.used() && (result.failed || result.unreachable || result.uncertain) {
            result.no_log = true;
        }
    }
    pub fn used(&self) -> bool {
        self.used.load(Ordering::Relaxed)
    }
    pub fn has_passwords(&self) -> bool {
        !self.secrets.is_empty()
    }
    pub fn is_encrypted(bytes: &[u8]) -> bool {
        bytes.starts_with(b"$ANSIBLE_VAULT;")
    }
    pub fn decrypt(&self, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        self.used.store(true, Ordering::Relaxed);
        let envelope = Envelope::parse(bytes)?;
        let mut secrets = self.secrets.iter().collect::<Vec<_>>();
        if let Some(id) = &envelope.id {
            secrets.sort_by_key(|secret| secret.id.as_ref() != Some(id));
        }
        for secret in secrets {
            let key = derive(&secret.password, &envelope.salt);
            let mut mac =
                Hmac::<Sha256>::new_from_slice(&key[32..64]).expect("Fixed HMAC key length");
            mac.update(&envelope.ciphertext);
            if mac.verify_slice(&envelope.mac).is_err() {
                continue;
            }
            let mut plaintext = Zeroizing::new(envelope.ciphertext.clone());
            Cipher::new_from_slices(&key[..32], &key[64..])
                .expect("Fixed AES key and IV length")
                .apply_keystream(&mut plaintext);
            let padding = *plaintext
                .last()
                .context("Invalid authenticated Vault padding")? as usize;
            if padding == 0
                || padding > 16
                || padding > plaintext.len()
                || !plaintext[plaintext.len() - padding..]
                    .iter()
                    .all(|byte| *byte as usize == padding)
            {
                bail!("Invalid authenticated Vault padding");
            }
            let length = plaintext.len() - padding;
            if length > MAX_PLAINTEXT {
                bail!("Vault plaintext exceeds 16 MiB");
            }
            plaintext.truncate(length);
            return Ok(plaintext);
        }
        bail!("Cannot decrypt Vault content with the supplied password sources")
    }
    pub fn encrypt(&self, plaintext: &[u8], id: Option<&str>) -> Result<Vec<u8>> {
        if plaintext.len() > MAX_PLAINTEXT {
            bail!("Vault plaintext exceeds 16 MiB");
        }
        if let Some(id) = id {
            validate_id(id)?;
        }
        let secret = if let Some(id) = id {
            self.secrets
                .iter()
                .find(|secret| secret.id.as_deref() == Some(id))
                .context("No Vault password source matches encrypt-vault-id")?
        } else {
            if self.secrets.len() != 1 {
                bail!("Encryption needs one password source or an explicit encrypt-vault-id");
            }
            &self.secrets[0]
        };
        let mut salt = [0u8; 32];
        getrandom::fill(&mut salt)
            .map_err(|_| anyhow::anyhow!("Cannot obtain cryptographic Vault salt"))?;
        self.encrypt_with_salt(plaintext, secret, id.or(secret.id.as_deref()), &salt)
    }
    fn encrypt_with_salt(
        &self,
        plaintext: &[u8],
        secret: &Secret,
        id: Option<&str>,
        salt: &[u8],
    ) -> Result<Vec<u8>> {
        let key = derive(&secret.password, salt);
        let padding = 16 - plaintext.len() % 16;
        let mut ciphertext = Zeroizing::new(plaintext.to_vec());
        ciphertext.resize(plaintext.len() + padding, padding as u8);
        Cipher::new_from_slices(&key[..32], &key[64..])
            .expect("Fixed AES key and IV length")
            .apply_keystream(&mut ciphertext);
        let mut mac = Hmac::<Sha256>::new_from_slice(&key[32..64]).expect("Fixed HMAC key length");
        mac.update(&ciphertext);
        let inner = format!(
            "{}\n{}\n{}",
            hex(salt),
            hex(&mac.finalize().into_bytes()),
            hex(&ciphertext)
        );
        let body = hex(inner.as_bytes());
        let mut output = if let Some(id) = id {
            format!("$ANSIBLE_VAULT;1.2;AES256;{id}\n").into_bytes()
        } else {
            b"$ANSIBLE_VAULT;1.1;AES256\n".to_vec()
        };
        for line in body.as_bytes().chunks(80) {
            output.extend_from_slice(line);
            output.push(b'\n');
        }
        Ok(output)
    }
}
fn derive(password: &[u8], salt: &[u8]) -> Zeroizing<[u8; 80]> {
    let mut output = Zeroizing::new([0u8; 80]);
    pbkdf2::pbkdf2_hmac::<Sha256>(password, salt, 10000, &mut *output);
    output
}
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 1024
        || id
            .chars()
            .any(|character| character.is_control() || character == ';' || character == '@')
    {
        bail!("Invalid Vault ID");
    }
    Ok(())
}
struct Envelope {
    id: Option<String>,
    salt: Vec<u8>,
    mac: Vec<u8>,
    ciphertext: Vec<u8>,
}
impl Envelope {
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_ENVELOPE {
            bail!("Vault envelope exceeds 80 MiB");
        }
        let content = std::str::from_utf8(bytes)
            .map_err(|_| anyhow::anyhow!("Vault envelope must be ASCII/UTF-8"))?;
        let (header, body) = content
            .split_once('\n')
            .context("Truncated Vault envelope")?;
        if header.len() > 2048 {
            bail!("Oversized Vault header");
        }
        let fields = header
            .trim_end_matches('\r')
            .splitn(5, ';')
            .collect::<Vec<_>>();
        let id = match fields.as_slice() {
            ["$ANSIBLE_VAULT", "1.1", "AES256"] => None,
            ["$ANSIBLE_VAULT", "1.2", "AES256", id] => {
                validate_id(id)?;
                Some((*id).into())
            }
            _ => bail!("Unsupported Vault header; expected 1.1/1.2 AES256"),
        };
        let body = body
            .bytes()
            .filter(|byte| !matches!(*byte, b'\n' | b'\r'))
            .collect::<Vec<_>>();
        let inner = unhex(&body)?;
        let fields = inner.splitn(4, |byte| *byte == b'\n').collect::<Vec<_>>();
        let [salt, mac, ciphertext] = fields.as_slice() else {
            bail!("Invalid Vault payload framing");
        };
        if salt.len() > 2048 || mac.len() != 64 || ciphertext.len() > 2 * (MAX_PLAINTEXT + 16) {
            bail!("Invalid Vault payload field lengths");
        }
        let salt = unhex(salt)?;
        let mac = unhex(mac)?;
        let ciphertext = unhex(ciphertext)?;
        if salt.is_empty()
            || salt.len() > 1024
            || mac.len() != 32
            || ciphertext.is_empty()
            || !ciphertext.len().is_multiple_of(16)
            || ciphertext.len() > MAX_PLAINTEXT + 16
        {
            bail!("Invalid Vault salt, authentication tag or ciphertext length");
        }
        Ok(Self {
            id,
            salt,
            mac,
            ciphertext,
        })
    }
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}
fn unhex(bytes: &[u8]) -> Result<Vec<u8>> {
    if !bytes.len().is_multiple_of(2) {
        bail!("Invalid Vault hex encoding");
    }
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16);
            let low = (pair[1] as char).to_digit(16);
            match (high, low) {
                (Some(high), Some(low)) => Ok(((high << 4) | low) as u8),
                _ => bail!("Invalid Vault hex encoding"),
            }
        })
        .collect()
}
thread_local! { static CURRENT: std::cell::RefCell<Arc<Vault>> = std::cell::RefCell::new(Arc::default()); }
/// A run context follows existing scoped execution guards. Every task thread
/// explicitly enters its runtime's context; nested/library runs restore the
/// caller's context and cannot exchange password sources across threads.
pub struct VaultGuard {
    previous: Arc<Vault>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for VaultGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.previous.clone());
        });
    }
}
impl Vault {
    pub fn current() -> Arc<Self> {
        CURRENT.with(|current| current.borrow().clone())
    }
    pub fn enter(self: &Arc<Self>) -> VaultGuard {
        VaultGuard {
            previous: CURRENT.with(|current| current.replace(self.clone())),
            _thread: std::marker::PhantomData,
        }
    }
    pub fn protect<T>(&self, result: Result<T>) -> Result<T> {
        result.map_err(|error| {
            if self.used() {
                anyhow::anyhow!(
                    "Vault-protected operation failed; sensitive error details were censored"
                )
            } else {
                error
            }
        })
    }
    pub(crate) fn read_text(&self, path: &std::path::Path) -> Result<Zeroizing<String>> {
        use std::{io::Read, os::unix::fs::OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .with_context(|| format!("Cannot open input {}", path.display()))?;
        if !file.metadata()?.is_file() {
            bail!("Input must be a regular file");
        }
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_ENVELOPE as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_ENVELOPE {
            bail!("Input exceeds the 80 MiB Vault envelope bound");
        }
        if Self::is_encrypted(&bytes) {
            bytes = self.decrypt(&bytes)?;
        }
        Ok(Zeroizing::new(String::from_utf8(bytes.to_vec()).map_err(
            |_| anyhow::anyhow!("Input requires UTF-8 text"),
        )?))
    }
}

pub(crate) fn is_value(value: &serde_yaml::Value) -> bool {
    matches!(value,serde_yaml::Value::Tagged(tagged) if tagged.tag == "vault")
}
impl Vault {
    pub(crate) fn materialize(&self, value: &serde_yaml::Value) -> Result<serde_yaml::Value> {
        use serde_yaml::Value;
        match value {
            Value::Tagged(tagged) if tagged.tag == "vault" => {
                let ciphertext = tagged
                    .value
                    .as_str()
                    .context("Vault tag requires an encrypted string")?;
                let plaintext = self.decrypt(ciphertext.as_bytes())?;
                Ok(Value::String(
                    String::from_utf8(plaintext.to_vec())
                        .map_err(|_| anyhow::anyhow!("Vault variables require UTF-8 text"))?,
                ))
            }
            Value::Mapping(values) => values
                .iter()
                .map(|(key, value)| Ok((key.clone(), self.materialize(value)?)))
                .collect::<Result<serde_yaml::Mapping>>()
                .map(Value::Mapping),
            Value::Sequence(values) => values
                .iter()
                .map(|value| self.materialize(value))
                .collect::<Result<Vec<_>>>()
                .map(Value::Sequence),
            _ => Ok(value.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_ansible_221_envelopes_and_thread_scopes_are_interoperable() {
        let vault = Arc::new(
            Vault::new(vec![(Some("prod".into()), b"fixture-password".to_vec())]).unwrap(),
        );
        let expected = b"literal {{ must_not_execute }}\nsecret-canary\n";
        for input in [
            include_bytes!("../tests/fixtures/vault/reference-1.1.vault").as_slice(),
            include_bytes!("../tests/fixtures/vault/reference-1.2.vault").as_slice(),
        ] {
            assert_eq!(&*vault.decrypt(input).unwrap(), expected);
        }
        let salt = (0u8..32).collect::<Vec<_>>();
        assert_eq!(
            vault
                .encrypt_with_salt(expected, &vault.secrets[0], None, &salt)
                .unwrap(),
            include_bytes!("../tests/fixtures/vault/reference-1.1.vault")
        );
        assert_eq!(
            vault
                .encrypt_with_salt(expected, &vault.secrets[0], Some("prod"), &salt)
                .unwrap(),
            include_bytes!("../tests/fixtures/vault/reference-1.2.vault")
        );
        let previous = Vault::current();
        {
            let _scope = vault.enter();
            assert!(Arc::ptr_eq(&Vault::current(), &vault));
            let other = std::thread::spawn(|| !Vault::current().has_passwords())
                .join()
                .unwrap();
            assert!(other);
        }
        assert!(Arc::ptr_eq(&Vault::current(), &previous));
    }
    #[test]
    fn authenticated_roundtrip_ids_padding_binary_and_random_salts() {
        let vault = Vault::new(vec![(Some("prod".into()), b"fixture-password".to_vec())]).unwrap();
        for plaintext in [
            &b""[..],
            &b"a\0b\xff"[..],
            &b"literal {{ must_not_execute }}"[..],
            &[b'x'; 16][..],
        ] {
            let encrypted = vault.encrypt(plaintext, None).unwrap();
            assert!(encrypted.starts_with(b"$ANSIBLE_VAULT;1.2;AES256;prod\n"));
            assert_eq!(&*vault.decrypt(&encrypted).unwrap(), plaintext);
            assert_ne!(vault.encrypt(plaintext, None).unwrap(), encrypted);
            let mut tampered = encrypted.clone();
            let position = tampered.len() - 2;
            tampered[position] = if tampered[position] == b'0' {
                b'1'
            } else {
                b'0'
            };
            assert!(vault.decrypt(&tampered).is_err());
            assert!(vault.decrypt(&encrypted[..encrypted.len() / 2]).is_err());
            assert!(Vault::new(vec![(None, b"wrong".to_vec())])
                .unwrap()
                .decrypt(&encrypted)
                .is_err());
        }
        assert!(!format!("{vault:?}").contains("fixture-password"));
        let other_label =
            Vault::new(vec![(Some("dev".into()), b"fixture-password".to_vec())]).unwrap();
        assert_eq!(
            &*other_label
                .decrypt(&vault.encrypt(b"hint", None).unwrap())
                .unwrap(),
            b"hint"
        );
    }
}
