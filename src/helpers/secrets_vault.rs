//! Encrypted on-disk secrets store.
//!
//! Replaces the macOS Keychain as the storage for connection URIs, SSH
//! credentials, proxy passwords, and AI provider API keys. The whole vault is
//! encrypted with a single user-chosen master password using
//! Argon2id (key derivation) + AES-256-GCM.
//!
//! File layout:
//!
//! ```text
//! [ 8 bytes  ] magic "OMVAULT\0"
//! [ 1 byte   ] format version (currently 1)
//! [16 bytes  ] argon2id salt
//! [12 bytes  ] AES-GCM nonce
//! [remaining ] ciphertext (incl. 16-byte GCM auth tag)
//! ```
//!
//! Plaintext payload is a UTF-8 JSON object: `{ "<key>": "<secret>", ... }`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, bail};
use argon2::Argon2;
use rand::RngExt as _;

const MAGIC: &[u8; 8] = b"OMVAULT\0";
const FORMAT_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const HEADER_LEN: usize = MAGIC.len() + 1 + SALT_LEN + NONCE_LEN;

/// State machine for the on-disk vault.
enum VaultState {
    /// No file on disk; no secrets stored yet.
    Empty,
    /// File exists; key not derived yet.
    Locked { salt: [u8; SALT_LEN], nonce: [u8; NONCE_LEN], ciphertext: Vec<u8> },
    /// Key derived and payload decrypted; cached for the session.
    Unlocked { key: [u8; KEY_LEN], salt: [u8; SALT_LEN], data: BTreeMap<String, String> },
}

pub struct SecretsVault {
    file_path: PathBuf,
    state: VaultState,
}

impl SecretsVault {
    /// Open the vault at `file_path`. Reads the file header if present;
    /// does not derive the key.
    pub fn open(file_path: PathBuf) -> Result<Self> {
        let state = if file_path.exists() {
            let bytes = fs::read(&file_path)
                .with_context(|| format!("read vault file {}", file_path.display()))?;
            parse_locked(&bytes)?
        } else {
            VaultState::Empty
        };
        Ok(Self { file_path, state })
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.state, VaultState::Empty)
    }

    pub fn is_locked(&self) -> bool {
        matches!(self.state, VaultState::Locked { .. })
    }

    pub fn is_unlocked(&self) -> bool {
        matches!(self.state, VaultState::Unlocked { .. })
    }

    /// Try to unlock a Locked vault with the supplied password.
    /// Returns an error if the vault is in any other state or the password is wrong.
    pub fn unlock(&mut self, password: &str) -> Result<()> {
        let VaultState::Locked { salt, nonce, ciphertext } = &self.state else {
            bail!("vault is not locked");
        };
        let key = derive_key(password, salt)?;
        let cipher = Aes256Gcm::new_from_slice(&key).context("failed to create cipher")?;
        let plaintext = cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext.as_slice())
            .map_err(|_| anyhow::anyhow!("decryption failed — wrong password?"))?;
        let data: BTreeMap<String, String> =
            serde_json::from_slice(&plaintext).context("vault payload is not valid JSON")?;
        let salt = *salt;
        self.state = VaultState::Unlocked { key, salt, data };
        Ok(())
    }

    /// Create a fresh vault on disk encrypted with the supplied password.
    /// Errors if the vault already exists.
    pub fn create(&mut self, password: &str) -> Result<()> {
        if !matches!(self.state, VaultState::Empty) {
            bail!("vault already exists; refusing to overwrite");
        }
        let mut rng = rand::rng();
        let mut salt = [0u8; SALT_LEN];
        rng.fill(&mut salt);
        let key = derive_key(password, &salt)?;
        self.state = VaultState::Unlocked { key, salt, data: BTreeMap::new() };
        self.persist()
    }

    /// Delete the vault file and return to the Empty state. Used as the
    /// "forgot password" recovery path.
    pub fn reset(&mut self) -> Result<()> {
        if self.file_path.exists() {
            fs::remove_file(&self.file_path)
                .with_context(|| format!("remove vault file {}", self.file_path.display()))?;
        }
        self.state = VaultState::Empty;
        Ok(())
    }

    /// Look up a secret. Returns `None` if the vault is locked, empty, or the
    /// key is absent.
    pub fn get(&self, key: &str) -> Option<String> {
        match &self.state {
            VaultState::Unlocked { data, .. } => data.get(key).cloned(),
            _ => None,
        }
    }

    /// Store a secret and persist immediately. Errors if the vault is not unlocked.
    pub fn set(&mut self, key: String, value: String) -> Result<()> {
        let VaultState::Unlocked { data, .. } = &mut self.state else {
            bail!("vault is not unlocked");
        };
        data.insert(key, value);
        self.persist()
    }

    /// Remove a secret and persist. No-op if the key is absent.
    /// Errors if the vault is not unlocked.
    pub fn remove(&mut self, key: &str) -> Result<()> {
        let VaultState::Unlocked { data, .. } = &mut self.state else {
            bail!("vault is not unlocked");
        };
        if data.remove(key).is_some() {
            self.persist()?;
        }
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        let VaultState::Unlocked { key, salt, data } = &self.state else {
            bail!("cannot persist a vault that is not unlocked");
        };

        let plaintext = serde_json::to_vec(data).context("serialize vault payload")?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill(&mut nonce_bytes);

        let cipher = Aes256Gcm::new_from_slice(key).context("failed to create cipher")?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_slice())
            .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;

        let mut bytes = Vec::with_capacity(HEADER_LEN + ciphertext.len());
        bytes.extend_from_slice(MAGIC);
        bytes.push(FORMAT_VERSION);
        bytes.extend_from_slice(salt);
        bytes.extend_from_slice(&nonce_bytes);
        bytes.extend_from_slice(&ciphertext);

        atomic_write(&self.file_path, &bytes)
    }
}

fn parse_locked(bytes: &[u8]) -> Result<VaultState> {
    if bytes.len() < HEADER_LEN {
        bail!("vault file is shorter than the expected header");
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        bail!("vault file magic does not match");
    }
    let version = bytes[MAGIC.len()];
    if version != FORMAT_VERSION {
        bail!("unsupported vault format version {version}");
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&bytes[MAGIC.len() + 1..MAGIC.len() + 1 + SALT_LEN]);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[MAGIC.len() + 1 + SALT_LEN..HEADER_LEN]);
    let ciphertext = bytes[HEADER_LEN..].to_vec();
    Ok(VaultState::Locked { salt, nonce, ciphertext })
}

fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    let mut key = [0u8; KEY_LEN];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

/// Write `bytes` to `path` atomically: write to a sibling temp file then rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("vault path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create vault parent dir {}", parent.display()))?;
    let tmp_path = path.with_extension("enc.tmp");
    {
        let mut tmp = fs::File::create(&tmp_path)
            .with_context(|| format!("create temp vault file {}", tmp_path.display()))?;
        tmp.write_all(bytes)
            .with_context(|| format!("write temp vault file {}", tmp_path.display()))?;
        tmp.sync_all().ok();
    }
    fs::rename(&tmp_path, path)
        .with_context(|| format!("rename {} to {}", tmp_path.display(), path.display()))?;
    Ok(())
}

/// Default vault location: `<config>/openmango/secrets.enc`.
pub fn default_vault_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("could not determine config directory")?.join("openmango");
    Ok(dir.join("secrets.enc"))
}

// ── Process-wide singleton ───────────────────────────────────────────
//
// The vault lives behind a `Mutex` and is initialised once at app
// startup via `init()`. The KeyStore module reads/writes through `global()`.

use std::sync::{Mutex, OnceLock};

static VAULT: OnceLock<Mutex<SecretsVault>> = OnceLock::new();

/// Initialise the process-wide vault from `path`. Should be called exactly once
/// at app startup. Subsequent calls are ignored.
pub fn init(path: PathBuf) -> Result<()> {
    let vault = SecretsVault::open(path)?;
    VAULT.set(Mutex::new(vault)).map_err(|_| anyhow::anyhow!("vault already initialised")).ok();
    Ok(())
}

/// Access the process-wide vault. Returns `None` before `init()` is called.
pub fn global() -> Option<&'static Mutex<SecretsVault>> {
    VAULT.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault_path() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openmango-vault-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("secrets.enc")
    }

    #[test]
    fn create_set_get_unlock_roundtrip() {
        let path = temp_vault_path();
        let mut vault = SecretsVault::open(path.clone()).unwrap();
        assert!(vault.is_empty());

        vault.create("hunter2").unwrap();
        assert!(vault.is_unlocked());

        vault.set("conn.123.uri".into(), "mongodb://user:pw@host".into()).unwrap();
        assert_eq!(vault.get("conn.123.uri").as_deref(), Some("mongodb://user:pw@host"));

        let mut reopened = SecretsVault::open(path.clone()).unwrap();
        assert!(reopened.is_locked());
        assert_eq!(reopened.get("conn.123.uri"), None, "locked vault should not return secrets");

        reopened.unlock("hunter2").unwrap();
        assert!(reopened.is_unlocked());
        assert_eq!(reopened.get("conn.123.uri").as_deref(), Some("mongodb://user:pw@host"));

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn wrong_password_fails_to_unlock() {
        let path = temp_vault_path();
        let mut vault = SecretsVault::open(path.clone()).unwrap();
        vault.create("right").unwrap();
        vault.set("k".into(), "v".into()).unwrap();

        let mut reopened = SecretsVault::open(path.clone()).unwrap();
        assert!(reopened.unlock("wrong").is_err());
        assert!(reopened.is_locked(), "failed unlock should leave vault locked");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn remove_persists() {
        let path = temp_vault_path();
        let mut vault = SecretsVault::open(path.clone()).unwrap();
        vault.create("pw").unwrap();
        vault.set("a".into(), "1".into()).unwrap();
        vault.set("b".into(), "2".into()).unwrap();
        vault.remove("a").unwrap();

        let mut reopened = SecretsVault::open(path.clone()).unwrap();
        reopened.unlock("pw").unwrap();
        assert_eq!(reopened.get("a"), None);
        assert_eq!(reopened.get("b").as_deref(), Some("2"));

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn reset_wipes_file() {
        let path = temp_vault_path();
        let mut vault = SecretsVault::open(path.clone()).unwrap();
        vault.create("pw").unwrap();
        vault.set("k".into(), "v".into()).unwrap();
        assert!(path.exists());

        vault.reset().unwrap();
        assert!(!path.exists());
        assert!(vault.is_empty());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn reject_garbage_file() {
        let path = temp_vault_path();
        std::fs::write(&path, b"not a real vault").unwrap();
        let result = SecretsVault::open(path.clone());
        assert!(result.is_err());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
