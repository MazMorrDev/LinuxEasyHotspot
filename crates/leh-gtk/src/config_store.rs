//! Loading and saving `~/.config/linuxeasyhotspot/config.toml`.

use std::path::PathBuf;

use leh_model::HotspotConfig;

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("linuxeasyhotspot")
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_owned())
        .unwrap_or_default()
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

/// Writes atomically (temp file + rename), readable only by the user since
/// it contains the hotspot password.
pub fn save(cfg: &HotspotConfig) {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let dir = config_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("config: can't create {}: {e}", dir.display());
        return;
    }
    let tmp = dir.join("config.toml.tmp");
    let res = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(cfg.to_toml().as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, config_path()));
    if let Err(e) = res {
        eprintln!("config: can't save: {e}");
    }
}

/// Per-device settings and the defaults for new devices (`devices.toml`).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DeviceStore {
    pub new_devices: leh_model::policy::DefaultPolicy,
    pub total_down_mbps: Option<u32>,
    pub total_up_mbps: Option<u32>,
    pub devices: std::collections::BTreeMap<String, leh_model::policy::DevicePolicy>,
}

pub fn load_devices() -> DeviceStore {
    std::fs::read_to_string(config_dir().join("devices.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_devices(store: &DeviceStore) {
    match toml::to_string_pretty(store) {
        Ok(text) => write_private("devices.toml", &text),
        Err(e) => eprintln!("devices: can't serialize: {e}"),
    }
}

fn write_private(name: &str, text: &str) {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let dir = config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let tmp = dir.join(format!("{name}.tmp"));
    let res = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, dir.join(name)));
    if let Err(e) = res {
        eprintln!("{name}: can't save: {e}");
    }
}
