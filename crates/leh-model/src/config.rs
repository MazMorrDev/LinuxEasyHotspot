//! User configuration, persisted as TOML.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{password, ssid};

/// Current on-disk config format. Bump and add a migration in
/// [`HotspotConfig::from_toml`] when the format changes incompatibly.
pub const CONFIG_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// NAT: devices share this computer's internet connection.
    #[default]
    Router,
    /// Devices join the wired LAN through a bridge.
    Bridge,
    /// Devices can reach each other and this computer, but not the internet.
    LocalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Band {
    #[default]
    Auto,
    #[serde(rename = "2.4ghz")]
    Ghz2_4,
    #[serde(rename = "5ghz")]
    Ghz5,
}

impl Band {
    /// Channels offered for manual selection. Conservative on purpose:
    /// 1–11 are legal everywhere, 36–48 are 5 GHz channels without DFS.
    pub fn allowed_channels(self) -> &'static [u32] {
        match self {
            Band::Auto => &[],
            Band::Ghz2_4 => &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            Band::Ghz5 => &[36, 40, 44, 48],
        }
    }

    /// Value for `802-11-wireless.band`, `None` lets NM decide.
    pub fn nm_value(self) -> Option<&'static str> {
        match self {
            Band::Auto => None,
            Band::Ghz2_4 => Some("bg"),
            Band::Ghz5 => Some("a"),
        }
    }
}

/// Interface language; `System` follows `LANGUAGE`/`LC_*`/`LANG`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "es")]
    Spanish,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::System, Self::English, Self::Spanish];
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct HotspotConfig {
    pub version: u8,
    pub ssid: String,
    pub password: String,
    pub mode: Mode,
    pub band: Band,
    /// 0 means automatic.
    pub channel: u32,
    pub hidden: bool,
    /// Wi-Fi interface that runs the access point; `None` = first AP-capable.
    pub ap_iface: Option<String>,
    /// Upstream interface. Informational in Router mode, required (Ethernet)
    /// in Bridge mode.
    pub upstream_iface: Option<String>,
    /// Launch the app (hidden) at login.
    pub autostart: bool,
    /// Also start the hotspot when launched at login.
    pub start_at_login: bool,
    pub show_tray: bool,
    /// Interface language.
    pub language: Language,
    /// UUID of the NM profile we created, to find it again.
    pub profile_uuid: Option<String>,
}

impl Default for HotspotConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            ssid: String::new(),
            password: String::new(),
            mode: Mode::Router,
            band: Band::Auto,
            channel: 0,
            hidden: false,
            ap_iface: None,
            upstream_iface: None,
            autostart: false,
            start_at_login: false,
            show_tray: true,
            language: Language::System,
            profile_uuid: None,
        }
    }
}

impl fmt::Debug for HotspotConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HotspotConfig")
            .field("version", &self.version)
            .field("ssid", &self.ssid)
            .field("password", &"<redacted>")
            .field("mode", &self.mode)
            .field("band", &self.band)
            .field("channel", &self.channel)
            .field("hidden", &self.hidden)
            .field("ap_iface", &self.ap_iface)
            .field("upstream_iface", &self.upstream_iface)
            .field("autostart", &self.autostart)
            .field("start_at_login", &self.start_at_login)
            .field("show_tray", &self.show_tray)
            .field("language", &self.language)
            .field("profile_uuid", &self.profile_uuid)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Ssid(#[from] ssid::SsidError),
    #[error(transparent)]
    Password(#[from] password::PasswordError),
    #[error("channel {channel} is not available on this band")]
    Channel { channel: u32 },
    #[error("bridge mode needs a wired (Ethernet) internet source")]
    BridgeNeedsUpstream,
    #[error("config file is not valid TOML: {0}")]
    Parse(String),
    #[error("config file version {0} is newer than this app supports")]
    TooNew(u8),
}

impl HotspotConfig {
    /// Config for the very first launch: friendly SSID from the hostname and
    /// a freshly generated password.
    pub fn first_run(hostname: &str) -> Self {
        Self {
            ssid: ssid::from_hostname(hostname),
            password: password::generate(),
            ..Self::default()
        }
    }

    /// Checks everything that can be checked without talking to NM.
    pub fn validate(&self) -> Result<(), Vec<ConfigError>> {
        let mut errors = Vec::new();
        if let Err(e) = ssid::validate(&self.ssid) {
            errors.push(e.into());
        }
        if let Err(e) = password::validate(&self.password) {
            errors.push(e.into());
        }
        if self.channel != 0 && !self.band.allowed_channels().contains(&self.channel) {
            errors.push(ConfigError::Channel {
                channel: self.channel,
            });
        }
        if self.mode == Mode::Bridge && self.upstream_iface.is_none() {
            errors.push(ConfigError::BridgeNeedsUpstream);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Parses a config file, applying migrations from older versions.
    /// Unknown keys are ignored so a downgrade does not lose the whole file.
    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        let mut cfg: Self = toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;
        if cfg.version > CONFIG_VERSION {
            return Err(ConfigError::TooNew(cfg.version));
        }
        // Migrations go here, e.g. `if cfg.version < 2 { ... }`.
        cfg.version = CONFIG_VERSION;
        Ok(cfg)
    }

    /// Whether loading this text required a migration (caller should keep a
    /// `.bak` copy of the original before overwriting).
    pub fn needs_migration(s: &str) -> bool {
        #[derive(Deserialize)]
        struct V {
            #[serde(default)]
            version: u8,
        }
        toml::from_str::<V>(s).is_ok_and(|v| v.version < CONFIG_VERSION)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("HotspotConfig is always serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> HotspotConfig {
        HotspotConfig {
            ssid: "My Hotspot".into(),
            password: "correcthorse".into(),
            ..HotspotConfig::default()
        }
    }

    #[test]
    fn debug_redacts_password() {
        let dbg = format!("{:?}", valid());
        assert!(!dbg.contains("correcthorse"));
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn first_run_is_valid() {
        let cfg = HotspotConfig::first_run("mazmorr-laptop");
        assert_eq!(cfg.ssid, "mazmorr-laptop");
        assert!(cfg.validate().is_ok(), "{:?}", cfg.validate());
    }

    #[test]
    fn rejects_channel_not_in_band() {
        let mut cfg = valid();
        cfg.band = Band::Ghz2_4;
        cfg.channel = 36;
        assert_eq!(
            cfg.validate(),
            Err(vec![ConfigError::Channel { channel: 36 }])
        );
        cfg.channel = 6;
        assert!(cfg.validate().is_ok());
        cfg.band = Band::Auto;
        assert!(cfg.validate().is_err(), "manual channel needs a band");
    }

    #[test]
    fn bridge_requires_upstream() {
        let mut cfg = valid();
        cfg.mode = Mode::Bridge;
        assert_eq!(cfg.validate(), Err(vec![ConfigError::BridgeNeedsUpstream]));
        cfg.upstream_iface = Some("enp0s31f6".into());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn toml_round_trip() {
        let mut cfg = valid();
        cfg.mode = Mode::LocalOnly;
        cfg.band = Band::Ghz5;
        cfg.channel = 40;
        let text = cfg.to_toml();
        assert!(text.contains("band = \"5ghz\""));
        assert!(text.contains("mode = \"local-only\""));
        assert_eq!(HotspotConfig::from_toml(&text).unwrap(), cfg);
    }

    #[test]
    fn missing_and_unknown_fields_are_tolerated() {
        let cfg = HotspotConfig::from_toml("ssid = \"x\"\nfuture-option = 3\n").unwrap();
        assert_eq!(cfg.ssid, "x");
        assert_eq!(cfg.version, CONFIG_VERSION);
        assert!(cfg.show_tray);
    }

    #[test]
    fn version_handling() {
        assert!(HotspotConfig::needs_migration("ssid = \"x\""));
        assert!(!HotspotConfig::needs_migration("version = 1"));
        assert_eq!(
            HotspotConfig::from_toml("version = 99"),
            Err(ConfigError::TooNew(99))
        );
        assert!(matches!(
            HotspotConfig::from_toml("ssid = "),
            Err(ConfigError::Parse(_))
        ));
    }
}
