//! Typed NetworkManager connection settings (`a{sa{sv}}`) and the builders
//! that produce them from a [`HotspotConfig`].
//!
//! `leh-nm` converts [`NmSettings`] to `zvariant` values; keeping the DTO here
//! lets us snapshot-test exactly what we send to NM for every mode.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::{
    AP_PROFILE_ID, BRIDGE_IFACE, BRIDGE_PORT_PROFILE_ID, BRIDGE_PROFILE_ID,
    caps::NmCapabilities,
    config::{HotspotConfig, Mode},
};

/// A single NM setting value, restricted to the D-Bus types we use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SettingValue {
    /// `s`
    Str(String),
    /// `b`
    Bool(bool),
    /// `u`
    U32(u32),
    /// `i`
    I32(i32),
    /// `ay`
    Bytes(Vec<u8>),
    /// `as`
    StrList(Vec<String>),
}

impl SettingValue {
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }
    pub fn str_list<I: IntoIterator<Item = S>, S: Into<String>>(items: I) -> Self {
        Self::StrList(items.into_iter().map(Into::into).collect())
    }
}

/// `setting name → (property → value)`, ordered for stable snapshots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct NmSettings(pub BTreeMap<String, BTreeMap<String, SettingValue>>);

impl NmSettings {
    pub fn set(&mut self, setting: &str, key: &str, value: SettingValue) -> &mut Self {
        self.0
            .entry(setting.to_owned())
            .or_default()
            .insert(key.to_owned(), value);
        self
    }

    pub fn get(&self, setting: &str, key: &str) -> Option<&SettingValue> {
        self.0.get(setting)?.get(key)
    }

    /// A copy with secrets replaced, safe for logs and diagnostics.
    pub fn redacted(&self) -> Self {
        let mut copy = self.clone();
        if let Some(sec) = copy.0.get_mut("802-11-wireless-security")
            && let Some(psk) = sec.get_mut("psk")
        {
            *psk = SettingValue::str("<redacted>");
        }
        copy
    }
}

/// Identity of the profiles the app owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileIds {
    pub ap_uuid: String,
    pub bridge_uuid: String,
    pub port_uuid: String,
    /// Local user name, for `connection.permissions = ["user:<name>"]`, which
    /// lets NM accept the profile without a polkit admin prompt.
    pub user: String,
}

/// NM `NMSettingIPConfigForwarding` "no" (NM ≥ 1.52).
const IPV4_FORWARDING_NO: i32 = 0;

fn connection(s: &mut NmSettings, id: &str, uuid: &str, ty: &str, ids: &ProfileIds) {
    s.set("connection", "id", SettingValue::str(id))
        .set("connection", "uuid", SettingValue::str(uuid))
        .set("connection", "type", SettingValue::str(ty))
        .set("connection", "autoconnect", SettingValue::Bool(false))
        .set(
            "connection",
            "permissions",
            SettingValue::str_list([format!("user:{}", ids.user)]),
        );
}

/// The access-point profile. In Bridge mode it is a bridge port without IP
/// settings; otherwise it carries `ipv4.method = shared`.
pub fn ap_profile(cfg: &HotspotConfig, caps: &NmCapabilities, ids: &ProfileIds) -> NmSettings {
    let mut s = NmSettings::default();
    connection(&mut s, AP_PROFILE_ID, &ids.ap_uuid, "802-11-wireless", ids);
    if let Some(iface) = &cfg.ap_iface {
        s.set("connection", "interface-name", SettingValue::str(iface));
    }

    let w = "802-11-wireless";
    s.set(w, "ssid", SettingValue::Bytes(cfg.ssid.as_bytes().to_vec()))
        .set(w, "mode", SettingValue::str("ap"))
        .set(w, "hidden", SettingValue::Bool(cfg.hidden));
    if let Some(band) = cfg.band.nm_value() {
        s.set(w, "band", SettingValue::str(band));
        if cfg.channel != 0 {
            s.set(w, "channel", SettingValue::U32(cfg.channel));
        }
    }

    let sec = "802-11-wireless-security";
    s.set(sec, "key-mgmt", SettingValue::str("wpa-psk"))
        .set(sec, "proto", SettingValue::str_list(["rsn"]))
        .set(sec, "pairwise", SettingValue::str_list(["ccmp"]))
        .set(sec, "group", SettingValue::str_list(["ccmp"]))
        .set(sec, "psk", SettingValue::str(&cfg.password));

    match cfg.mode {
        Mode::Bridge => {
            s.set("connection", "master", SettingValue::str(&ids.bridge_uuid))
                .set("connection", "slave-type", SettingValue::str("bridge"));
        }
        Mode::Router | Mode::LocalOnly => {
            s.set("ipv4", "method", SettingValue::str("shared")).set(
                "ipv6",
                "method",
                SettingValue::str("ignore"),
            );
            if cfg.mode == Mode::LocalOnly && caps.has_ipv4_forwarding {
                s.set("ipv4", "forwarding", SettingValue::I32(IPV4_FORWARDING_NO));
            }
        }
    }
    s
}

/// Bridge master profile (Bridge mode). Gets its address from the LAN.
pub fn bridge_profile(ids: &ProfileIds) -> NmSettings {
    let mut s = NmSettings::default();
    connection(&mut s, BRIDGE_PROFILE_ID, &ids.bridge_uuid, "bridge", ids);
    s.set(
        "connection",
        "interface-name",
        SettingValue::str(BRIDGE_IFACE),
    )
    .set("bridge", "stp", SettingValue::Bool(true))
    .set("bridge", "forward-delay", SettingValue::U32(4))
    .set("ipv4", "method", SettingValue::str("auto"))
    .set("ipv6", "method", SettingValue::str("auto"));
    s
}

/// Ethernet port profile enslaving the upstream NIC to the bridge.
pub fn bridge_port_profile(upstream_iface: &str, ids: &ProfileIds) -> NmSettings {
    let mut s = NmSettings::default();
    connection(
        &mut s,
        BRIDGE_PORT_PROFILE_ID,
        &ids.port_uuid,
        "802-3-ethernet",
        ids,
    );
    s.set(
        "connection",
        "interface-name",
        SettingValue::str(upstream_iface),
    )
    .set("connection", "master", SettingValue::str(&ids.bridge_uuid))
    .set("connection", "slave-type", SettingValue::str("bridge"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        caps::NmVersion,
        config::{Band, HotspotConfig},
    };

    fn ids() -> ProfileIds {
        ProfileIds {
            ap_uuid: "00000000-0000-0000-0000-0000000000a1".into(),
            bridge_uuid: "00000000-0000-0000-0000-0000000000b1".into(),
            port_uuid: "00000000-0000-0000-0000-0000000000c1".into(),
            user: "alice".into(),
        }
    }

    fn cfg(mode: Mode) -> HotspotConfig {
        HotspotConfig {
            ssid: "Test Net".into(),
            password: "k7Rp-x4Mz-q9Tw".into(),
            mode,
            ap_iface: Some("wlp2s0".into()),
            upstream_iface: Some("enp0s31f6".into()),
            ..HotspotConfig::default()
        }
    }

    fn caps(minor: u16) -> NmCapabilities {
        NmCapabilities::from_version(NmVersion::new(1, minor, 0))
    }

    #[test]
    fn snapshot_router_auto_band() {
        insta::assert_json_snapshot!(ap_profile(&cfg(Mode::Router), &caps(46), &ids()));
    }

    #[test]
    fn snapshot_router_5ghz_channel_hidden() {
        let mut c = cfg(Mode::Router);
        c.band = Band::Ghz5;
        c.channel = 40;
        c.hidden = true;
        insta::assert_json_snapshot!(ap_profile(&c, &caps(46), &ids()));
    }

    #[test]
    fn snapshot_local_only_nm_1_52() {
        insta::assert_json_snapshot!(ap_profile(&cfg(Mode::LocalOnly), &caps(52), &ids()));
    }

    #[test]
    fn snapshot_bridge_set() {
        let ids = ids();
        insta::assert_json_snapshot!(
            "bridge_set",
            (
                ap_profile(&cfg(Mode::Bridge), &caps(46), &ids),
                bridge_profile(&ids),
                bridge_port_profile("enp0s31f6", &ids),
            )
        );
    }

    #[test]
    fn local_only_without_forwarding_support_omits_property() {
        let s = ap_profile(&cfg(Mode::LocalOnly), &caps(46), &ids());
        assert_eq!(s.get("ipv4", "forwarding"), None);
    }

    #[test]
    fn auto_band_ignores_channel() {
        let mut c = cfg(Mode::Router);
        c.channel = 6;
        let s = ap_profile(&c, &caps(46), &ids());
        assert_eq!(s.get("802-11-wireless", "band"), None);
        assert_eq!(s.get("802-11-wireless", "channel"), None);
    }

    #[test]
    fn redaction() {
        let s = ap_profile(&cfg(Mode::Router), &caps(46), &ids()).redacted();
        assert_eq!(
            s.get("802-11-wireless-security", "psk"),
            Some(&SettingValue::str("<redacted>"))
        );
        assert!(!format!("{s:?}").contains("k7Rp"));
    }
}
