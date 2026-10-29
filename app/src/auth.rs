//! Web passwords are printed once; only SHA-256 digests are persisted. Adapted from YYproject.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Write, path::Path};
use subtle::ConstantTimeEq;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettings {
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub secure_cookies: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebUser {
    pub name: String,
    pub password_sha256: String,
    #[serde(default)]
    pub read_only: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerSettings,
    #[serde(default)]
    pub web_users: Vec<WebUser>,
}
pub fn digest(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn random_id() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "OS random generator unavailable")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if fs::metadata(path)?.len() > 1024 * 1024 {
            return Err("Authentication config exceeds 1 MiB".into());
        }
        let config: Self = toml::from_str(&fs::read_to_string(path)?)?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<()> {
        if self.web_users.is_empty() || self.web_users.len() > 256 {
            return Err("Authentication config requires 1..256 web users".into());
        }
        let mut names = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        for user in &self.web_users {
            if user.name.trim().is_empty()
                || user.name.len() > 128
                || user.name.chars().any(char::is_control)
                || !names.insert(&user.name)
                || !hashes.insert(&user.password_sha256)
                || user.password_sha256.len() != 64
                || !user
                    .password_sha256
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err("Invalid or duplicate Web user/digest".into());
            }
        }
        Ok(())
    }
    pub fn authenticate(&self, password: &str) -> Option<WebUser> {
        if !password.starts_with("yya_") || password.len() != 68 {
            return None;
        }
        let hash = digest(password);
        self.web_users
            .iter()
            .find(|u| bool::from(u.password_sha256.as_bytes().ct_eq(hash.as_bytes())))
            .cloned()
    }
}
/// An exclusive sidecar lock prevents concurrent generation from losing another user.
pub fn generate_password(path: &Path, name: &str, read_only: bool) -> Result<String> {
    let lock_path = path.with_extension("toml.lock");
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)?;
    struct Unlock {
        path: std::path::PathBuf,
        file: Option<fs::File>,
    }
    impl Drop for Unlock {
        fn drop(&mut self) {
            drop(self.file.take());
            let _ = fs::remove_file(&self.path);
        }
    }
    let unlock = Unlock {
        path: lock_path,
        file: Some(lock),
    };
    let mut config = if path.exists() {
        Config::load(path)?
    } else {
        Config::default()
    };
    let token = format!("yya_{}", random_id()?);
    config.web_users.retain(|u| u.name != name);
    config.web_users.push(WebUser {
        name: name.into(),
        password_sha256: digest(&token),
        read_only,
    });
    config.validate()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temp.write_all(toml::to_string_pretty(&config)?.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    drop(unlock);
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passwords_rotate_and_are_never_persisted_in_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.toml");
        let first = generate_password(&path, "alice", false).unwrap();
        let viewer = generate_password(&path, "viewer", true).unwrap();
        let second = generate_password(&path, "alice", false).unwrap();
        let c = Config::load(&path).unwrap();
        assert!(c.authenticate(&first).is_none());
        assert_eq!(c.authenticate(&second).unwrap().name, "alice");
        assert!(c.authenticate(&viewer).unwrap().read_only);
        let before = fs::read(&path).unwrap();
        assert!(generate_password(&path, "", false).is_err());
        assert!(!path.with_extension("toml.lock").exists());
        assert_eq!(fs::read(&path).unwrap(), before);
        let raw = String::from_utf8(before).unwrap();
        for secret in [first, viewer, second] {
            assert!(!raw.contains(&secret));
        }
    }
}
