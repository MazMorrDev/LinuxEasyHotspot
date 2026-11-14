//! Loading and saving the settings: `config.toml` (the hotspot) and
//! `devices.toml` (per-device settings and defaults).
//!
//! - Linux: `~/.config/linuxeasyhotspot/` (the folder of earlier versions,
//!   so existing settings carry over), files readable only by the user.
//! - Windows: `%APPDATA%\Easy Hotspot\`, which only the user can read.

use std::path::PathBuf;

use leh_model::HotspotConfig;

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(leh_model::APP_NAME)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("linuxeasyhotspot")
    }
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// This computer's name, the base of the default network name.
pub fn hostname() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|h| h.trim().to_owned())
            .unwrap_or_default()
    }
}

/// Loads the config, or creates first-run defaults (not saved until changed).
/// A file from an older version is backed up to `config.toml.bak` first.
pub fn load() -> HotspotConfig {
    let path = config_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return HotspotConfig::first_run(&hostname());
    };
    if HotspotConfig::needs_migration(&text) {
        let _ = std::fs::copy(&path, path.with_extension("toml.bak"));
    }
    match HotspotConfig::from_toml(&text) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("config: {e}; keeping a copy and starting fresh");
            let _ = std::fs::copy(&path, path.with_extension("toml.broken"));
            HotspotConfig::first_run(&hostname())
        }
    }
}

/// Saves the config (it contains the hotspot password).
pub fn save(cfg: &HotspotConfig) {
    write_private("config.toml", &cfg.to_toml());
}

/// Per-device settings and the defaults for new devices (`devices.toml`).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DeviceStore {
    pub new_devices: leh_model::policy::DefaultPolicy,
    pub total_down_kbps: Option<u32>,
    pub total_up_kbps: Option<u32>,
    /// Older files stored whole Mbps; read once and converted.
    #[serde(skip_serializing)]
    pub(crate) total_down_mbps: Option<u32>,
    #[serde(skip_serializing)]
    pub(crate) total_up_mbps: Option<u32>,
    pub devices: std::collections::BTreeMap<String, leh_model::policy::DevicePolicy>,
}

pub fn load_devices() -> DeviceStore {
    let mut s: DeviceStore = std::fs::read_to_string(config_dir().join("devices.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    if let Some(m) = s.total_down_mbps.take() {
        s.total_down_kbps.get_or_insert(m * 1000);
    }
    if let Some(m) = s.total_up_mbps.take() {
        s.total_up_kbps.get_or_insert(m * 1000);
    }
    s
}

pub fn save_devices(store: &DeviceStore) {
    match toml::to_string_pretty(store) {
        Ok(text) => write_private("devices.toml", &text),
        Err(e) => eprintln!("devices: can't serialize: {e}"),
    }
}

/// Writes atomically (temp file + rename). On Unix the file is readable only
/// by the user; on Windows the folder's permissions already ensure that.
fn write_private(name: &str, text: &str) {
    use std::io::Write as _;
    let dir = config_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{name}: can't create {}: {e}", dir.display());
        return;
    }
    let tmp = dir.join(format!("{name}.tmp"));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let res = opts
        .open(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, dir.join(name)));
    if let Err(e) = res {
        eprintln!("{name}: can't save: {e}");
    }
}
