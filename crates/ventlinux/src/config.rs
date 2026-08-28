use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use vent_ptt::Binding;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedServer {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password: String,
}

impl SavedServer {
    pub fn display_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub ptt: Binding,
    #[serde(default = "default_mode")]
    pub transmit_mode: String,
    #[serde(default = "default_vox")]
    pub vox_sensitivity: f32,
    #[serde(default)]
    pub input_device: String,
    #[serde(default)]
    pub output_device: String,
    /// False in configs written before the F13 default was dropped; those get
    /// migrated once on load.
    #[serde(default)]
    pub ptt_migrated: bool,
}

fn default_mode() -> String {
    "ptt".into()
}
fn default_vox() -> f32 {
    -40.0
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ptt: Binding::default(),
            transmit_mode: default_mode(),
            vox_sensitivity: default_vox(),
            input_device: String::new(),
            output_device: String::new(),
            // A fresh config already uses the current default binding.
            ptt_migrated: true,
        }
    }
}

pub fn dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("com", "cryptexlabs", "ventlinux")
        .ok_or_else(|| anyhow::anyhow!("no home directory"))
}

fn ensure_dir() -> Result<PathBuf> {
    let d = dirs()?.config_dir().to_path_buf();
    fs::create_dir_all(&d)?;
    Ok(d)
}

pub fn load_servers() -> Vec<SavedServer> {
    let Ok(dir) = ensure_dir() else {
        return Vec::new();
    };
    let path = dir.join("servers.json");
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_servers(servers: &[SavedServer]) -> Result<()> {
    let dir = ensure_dir()?;
    let path = dir.join("servers.json");
    let tmp = dir.join("servers.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(servers)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn load_config() -> AppConfig {
    let Ok(dir) = ensure_dir() else {
        return AppConfig::default();
    };
    let path = dir.join("config.toml");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(cfg: &AppConfig) -> Result<()> {
    let dir = ensure_dir()?;
    fs::write(dir.join("config.toml"), toml::to_string_pretty(cfg)?)?;
    Ok(())
}
