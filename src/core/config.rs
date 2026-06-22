use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;
use directories::BaseDirs;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum AccountType {
    Offline,
    Microsoft,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MicrosoftAuth {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Account {
    pub uuid: String,
    pub username: String,
    pub account_type: AccountType,
    pub microsoft_auth: Option<MicrosoftAuth>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub game_dir: PathBuf,
    pub java_path: PathBuf,
    pub jvm_args: Vec<String>,
    pub accounts: Vec<Account>,
    pub active_account_uuid: Option<String>,
    pub selected_version: Option<String>,
    pub active_instance: Option<String>,
    /// Number of files downloaded concurrently. Defaults to 16; older configs
    /// without the field fall back via `default_download_concurrency`.
    #[serde(default = "default_download_concurrency")]
    pub download_concurrency: usize,
}

/// Default parallel download count, used both for `Config::default` and as the
/// serde fallback for configs written before this field existed.
pub fn default_download_concurrency() -> usize {
    16
}

/// Total physical RAM in megabytes, or `None` if it can't be determined.
pub fn system_ram_mb() -> Option<u64> {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total = sys.total_memory(); // bytes
    if total == 0 {
        None
    } else {
        Some(total / 1024 / 1024)
    }
}

impl Default for Config {
    fn default() -> Self {
        let game_dir = Self::default_game_dir();
        Self {
            game_dir,
            java_path: PathBuf::from("java"),
            jvm_args: vec!["-Xmx2G".to_string()],
            accounts: Vec::new(),
            active_account_uuid: None,
            selected_version: None,
            active_instance: None,
            download_concurrency: default_download_concurrency(),
        }
    }
}

impl Config {
    pub fn default_game_dir() -> PathBuf {
        if let Some(base_dirs) = BaseDirs::new() {
            base_dirs.home_dir().join(".minecraft")
        } else {
            PathBuf::from(".minecraft")
        }
    }

    pub fn config_path() -> Option<PathBuf> {
        BaseDirs::new().map(|base_dirs| {
            base_dirs.config_dir().join("pyrite-launcher").join("config.json")
        })
    }

    pub fn load() -> Self {
        let mut config = if let Some(path) = Self::config_path() {
            if path.exists() {
                if let Ok(content) = fs::read_to_string(&path) {
                    serde_json::from_str::<Config>(&content).unwrap_or_default()
                } else {
                    Config::default()
                }
            } else {
                Config::default()
            }
        } else {
            Config::default()
        };
        // Pull Microsoft tokens from the keyring into memory (and migrate any
        // left over in the plaintext config). Re-save afterwards so a migrated
        // config drops its inline tokens from disk.
        let migrated = config.hydrate_secrets();
        config.migrate_and_initialize();
        if migrated {
            let _ = config.save();
        }
        config
    }

    /// Reconcile in-memory Microsoft tokens with the OS keyring. For each
    /// Microsoft account: if its tokens are missing (the normal case once
    /// secrets live in the keyring) load them from there; if they are still
    /// present inline (an older plaintext config) move them into the keyring and
    /// signal that the config should be re-saved to strip them from disk.
    ///
    /// No-op when no keyring backend is available — tokens then stay in the
    /// config file as before. Returns whether a migration occurred.
    fn hydrate_secrets(&mut self) -> bool {
        if !crate::core::secrets::available() {
            return false;
        }
        let mut migrated = false;
        for acc in &mut self.accounts {
            if acc.account_type != AccountType::Microsoft {
                continue;
            }
            match &acc.microsoft_auth {
                Some(auth) => {
                    if crate::core::secrets::store(&acc.uuid, auth) {
                        migrated = true;
                    }
                }
                None => acc.microsoft_auth = crate::core::secrets::load(&acc.uuid),
            }
        }
        migrated
    }

    pub fn migrate_and_initialize(&mut self) {
        let instances_dir = self.game_dir.join("instances");
        let _ = fs::create_dir_all(&instances_dir);

        if self.active_instance.is_none() {
            let list = crate::core::instance::Instance::load_all(&self.game_dir);
            if let Some(first) = list.first() {
                self.active_instance = Some(first.id.clone());
            } else {
                let version = self.selected_version.clone().unwrap_or_else(|| "1.21.1".to_string());
                if let Ok(inst) = crate::core::instance::Instance::create(&self.game_dir, "default", "Default Profile", &version) {
                    self.active_instance = Some(inst.id);
                }
            }
            let _ = self.save();
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().ok_or("Could not locate config directory")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // When a keyring backend exists, move Microsoft tokens into it and write
        // a copy of the config with those secret fields stripped from disk. Any
        // account whose tokens fail to store keeps them inline so the session
        // isn't lost. Without a keyring, serialize as-is (config-file fallback).
        let content = if crate::core::secrets::available() {
            let mut redacted = self.clone();
            for acc in &mut redacted.accounts {
                if acc.account_type != AccountType::Microsoft {
                    continue;
                }
                if let Some(auth) = &acc.microsoft_auth
                    && crate::core::secrets::store(&acc.uuid, auth)
                {
                    acc.microsoft_auth = None;
                }
            }
            serde_json::to_string_pretty(&redacted).map_err(|e| e.to_string())?
        } else {
            serde_json::to_string_pretty(self).map_err(|e| e.to_string())?
        };
        fs::write(path, content).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_active_account(&self) -> Option<&Account> {
        let uuid = self.active_account_uuid.as_ref()?;
        self.accounts.iter().find(|acc| &acc.uuid == uuid)
    }

    pub fn add_account(&mut self, account: Account) {
        // Remove existing account with the same UUID or username
        self.accounts.retain(|acc| acc.uuid != account.uuid);
        self.active_account_uuid = Some(account.uuid.clone());
        self.accounts.push(account);
        let _ = self.save();
    }

    pub fn remove_account(&mut self, uuid: &str) {
        self.accounts.retain(|acc| acc.uuid != uuid);
        if self.active_account_uuid.as_deref() == Some(uuid) {
            self.active_account_uuid = self.accounts.first().map(|acc| acc.uuid.clone());
        }
        // Drop any keyring-held tokens for the removed account.
        crate::core::secrets::delete(uuid);
        let _ = self.save();
    }
}
