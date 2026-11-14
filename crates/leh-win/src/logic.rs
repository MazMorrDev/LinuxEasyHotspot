//! Platform-independent pieces of the Windows backend: text clean-up, id
//! matching, WinRT enum → message mapping and the state-following rules.
//! Kept free of WinRT types so it builds and is tested on every system;
//! WinRT enums are passed as their `i32` values.
// Only the Windows backend (and the tests) use this.
#![cfg_attr(not(windows), allow(dead_code))]

use std::cmp::Ordering;
use std::net::Ipv4Addr;
use std::time::Duration;

use leh_model::reasons::{Explanation, Suggestion};
use leh_model::{Band, ErrorKind, HotspotConfig, HotspotState};

use crate::{Uplink, UplinkKind};

/// Internet Connection Sharing's default address on the hotspot side
/// (`SharedAccess\Parameters\ScopeAddress` when that's not set).
pub(crate) const ICS_GATEWAY: Ipv4Addr = Ipv4Addr::new(192, 168, 137, 1);

/// After a successful start, how long `Off` is still taken as "not up yet".
pub(crate) const OFF_GRACE: Duration = Duration::from_secs(10);
/// After a successful start, how long Windows may stay in transition.
pub(crate) const START_DEADLINE: Duration = Duration::from_secs(45);

pub(crate) const NO_WIFI: &str =
    "This computer has no Wi-Fi adapter, so it can't create a hotspot.";

/// `HRESULT`s handled specially.
pub(crate) mod hresult {
    pub const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
    pub const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154_u32 as i32;
}

/// `Windows.Networking.NetworkOperators.TetheringOperationStatus`.
pub(crate) mod status {
    pub const SUCCESS: i32 = 0;
    pub const UNKNOWN: i32 = 1;
    pub const MOBILE_BROADBAND_DEVICE_OFF: i32 = 2;
    pub const WIFI_DEVICE_OFF: i32 = 3;
    pub const ENTITLEMENT_CHECK_TIMEOUT: i32 = 4;
    pub const ENTITLEMENT_CHECK_FAILURE: i32 = 5;
    pub const OPERATION_IN_PROGRESS: i32 = 6;
    pub const BLUETOOTH_DEVICE_OFF: i32 = 7;
    pub const NETWORK_LIMITED_CONNECTIVITY: i32 = 8;
    pub const ALREADY_ON: i32 = 9;
    pub const RADIO_RESTRICTION: i32 = 10;
    pub const BAND_INTERFERENCE: i32 = 11;
}

/// `Windows.Networking.NetworkOperators.TetheringCapability`.
pub(crate) mod capability {
    pub const ENABLED: i32 = 0;
    pub const DISABLED_BY_GROUP_POLICY: i32 = 1;
    pub const DISABLED_BY_HARDWARE_LIMITATION: i32 = 2;
    pub const DISABLED_BY_OPERATOR: i32 = 3;
    pub const DISABLED_BY_SKU: i32 = 4;
    pub const DISABLED_BY_REQUIRED_APP_NOT_INSTALLED: i32 = 5;
    pub const DISABLED_DUE_TO_UNKNOWN_CAUSE: i32 = 6;
    pub const DISABLED_BY_SYSTEM_CAPABILITY: i32 = 7;
}

/// `TetheringOperationalState`, as far as the state machine cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    On,
    Off,
    InTransition,
    /// `TetheringOperationalState::Unknown`, or nothing could be read.
    Unknown,
}

pub(crate) fn op_from_winrt(value: i32) -> Op {
    match value {
        1 => Op::On,
        2 => Op::Off,
        3 => Op::InTransition,
        _ => Op::Unknown,
    }
}

/// `TetheringWiFiBand` value for the configured band.
pub(crate) fn band_to_winrt(band: Band) -> i32 {
    match band {
        Band::Auto => 0,
        Band::Ghz2_4 => 1,
        Band::Ghz5 => 2,
    }
}

/// 6 GHz (3) has no [`Band`] equivalent and reads as `Auto`.
pub(crate) fn band_from_winrt(value: i32) -> Band {
    match value {
        1 => Band::Ghz2_4,
        2 => Band::Ghz5,
        _ => Band::Auto,
    }
}

/// GUID as `NetworkAdapterId` is stored: lower case, no braces.
pub(crate) fn format_guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> String {
    let [a, b, c, d, e, f, g, h] = data4;
    format!(
        "{data1:08x}-{data2:04x}-{data3:04x}-{a:02x}{b:02x}-{c:02x}{d:02x}{e:02x}{f:02x}{g:02x}{h:02x}"
    )
}

/// Adapter id as written by people or older configs: braces and case ignored.
pub(crate) fn normalize_id(id: &str) -> String {
    id.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

/// `aa:bb:cc:dd:ee:ff` from `AA-BB-CC-DD-EE-FF`, `aabbccddeeff`…
/// Anything that isn't 6 bytes is kept (lower case) rather than dropped.
pub(crate) fn normalize_mac(raw: &str) -> String {
    let hex: Vec<u8> = raw
        .bytes()
        .filter(|b| !matches!(b, b':' | b'-' | b'.' | b' '))
        .collect();
    if hex.len() == 12 && hex.iter().all(u8::is_ascii_hexdigit) {
        hex.chunks(2)
            .map(|pair| String::from_utf8_lossy(pair).to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(":")
    } else {
        raw.trim().to_ascii_lowercase()
    }
}

/// Suffixes that DHCP/ICS appends to device names and that only add noise.
const HOSTNAME_SUFFIXES: &[&str] = &[
    ".mshome.net",
    ".home.arpa",
    ".localdomain",
    ".local",
    ".lan",
    ".home",
];

/// Device name from a client's `DomainName` host name, without the
/// `.mshome.net` (ICS) or similar suffix. `None` when nothing is left.
pub(crate) fn clean_hostname(raw: &str) -> Option<String> {
    let mut name = raw.trim().trim_end_matches('.');
    let has_suffix = |name: &str, suffix: &str| {
        name.len()
            .checked_sub(suffix.len())
            .filter(|&cut| name.is_char_boundary(cut))
            .filter(|&cut| name[cut..].eq_ignore_ascii_case(suffix))
    };
    if let Some(cut) = HOSTNAME_SUFFIXES
        .iter()
        .find_map(|suffix| has_suffix(name, suffix))
    {
        name = &name[..cut];
    }
    (!name.is_empty() && name != "*").then(|| name.to_owned())
}

/// Device-interface id of a USB device (`\\?\USB#VID_148F&PID_7601#…`).
pub(crate) fn is_usb_device(device_id: &str) -> bool {
    let id = device_id.to_ascii_uppercase();
    id.contains("USB#") || id.starts_with("USB\\")
}

/// The hotspot's own virtual adapter (or the old hosted-network one), which
/// must not be offered as an adapter to run the hotspot on.
pub(crate) fn is_virtual_wifi(name: &str, device_id: &str) -> bool {
    let name = name.to_ascii_lowercase();
    device_id.to_ascii_lowercase().contains("vwifimp")
        || name.contains("wi-fi direct virtual")
        || name.contains("hosted network virtual")
}

/// From `IsWwanConnectionProfile`, `IsWlanConnectionProfile` and the
/// adapter's `IanaInterfaceType`.
pub(crate) fn uplink_kind(wwan: bool, wlan: bool, iana_type: u32) -> UplinkKind {
    match (wwan, wlan, iana_type) {
        (true, _, _) | (_, _, 243 | 244) => UplinkKind::Cellular,
        (_, true, _) | (_, _, 71) => UplinkKind::Wifi,
        (_, _, 6) => UplinkKind::Ethernet,
        _ => UplinkKind::Other,
    }
}

/// Stable order for the uplink list: the internet connection first, then
/// connections with internet, then by name.
pub(crate) fn uplink_order(a: &Uplink, b: &Uplink) -> Ordering {
    b.is_default
        .cmp(&a.is_default)
        .then(b.internet.cmp(&a.internet))
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.id.cmp(&b.id))
}

/// Which connection to share: the one asked for (by adapter id) if it is
/// connected, otherwise Windows' internet connection. Candidates are
/// `(adapter id, is the internet connection)`.
pub(crate) fn pick_uplink(candidates: &[(&str, bool)], wanted: Option<&str>) -> Option<usize> {
    wanted
        .map(normalize_id)
        .and_then(|w| candidates.iter().position(|(id, _)| normalize_id(id) == w))
        .or_else(|| candidates.iter().position(|(_, default)| *default))
}

/// Which Wi-Fi adapter to run the hotspot on; `None` lets Windows choose.
pub(crate) fn pick_adapter(ids: &[&str], wanted: Option<&str>) -> Option<usize> {
    let wanted = normalize_id(wanted?);
    ids.iter().position(|id| normalize_id(id) == wanted)
}

/// A running hotspot started by the app with these settings doesn't need a
/// restart for a new start request.
pub(crate) fn same_settings(a: &HotspotConfig, b: &HotspotConfig) -> bool {
    let id = |v: &Option<String>| v.as_deref().map(normalize_id);
    a.ssid == b.ssid
        && a.password == b.password
        && a.band == b.band
        && id(&a.ap_iface) == id(&b.ap_iface)
        && id(&a.upstream_iface) == id(&b.upstream_iface)
}

/// `SharedAccess\Parameters\ScopeAddress` (ICS's hotspot-side address).
pub(crate) fn parse_scope_address(value: &str) -> Option<Ipv4Addr> {
    value
        .trim()
        .trim_end_matches('\0')
        .parse()
        .ok()
        .filter(|ip: &Ipv4Addr| !ip.is_unspecified())
}

fn explain(message: impl Into<String>, suggestion: Suggestion) -> Explanation {
    Explanation {
        message: message.into(),
        suggestion,
    }
}

pub(crate) fn no_internet() -> ErrorKind {
    ErrorKind::Activation(explain(
        "Connect this computer to the internet first: Windows' hotspot shares an existing connection.",
        Suggestion::None,
    ))
}

pub(crate) fn band_unsupported(band: Band) -> ErrorKind {
    let band = match band {
        Band::Ghz5 => "5 GHz",
        Band::Ghz2_4 => "2.4 GHz",
        Band::Auto => "this band",
    };
    ErrorKind::Activation(explain(
        format!("This Wi-Fi adapter can't run the hotspot on {band}. Choose another band."),
        Suggestion::ChangeChannel,
    ))
}

pub(crate) fn settings_rejected() -> ErrorKind {
    ErrorKind::Activation(explain(
        "Windows rejected the hotspot name or password. Use up to 32 characters for the name and 8 to 63 for the password.",
        Suggestion::CheckPassword,
    ))
}

/// What a WinRT call that took too long was doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Doing {
    Query,
    Configure,
    Start,
    Stop,
}

pub(crate) fn timed_out(doing: Doing) -> ErrorKind {
    let what = match doing {
        Doing::Query => "reading the network settings",
        Doing::Configure => "saving the hotspot settings",
        Doing::Start => "starting the hotspot",
        Doing::Stop => "turning the hotspot off",
    };
    ErrorKind::Activation(explain(
        format!("Windows didn't answer in time while {what}. Try again."),
        Suggestion::Retry,
    ))
}

/// Text for [`ErrorKind::Call`]: Windows' message plus the `HRESULT`.
pub(crate) fn call_text(message: &str, code: i32) -> String {
    let message = message.trim();
    let code = code as u32;
    match (message.is_empty(), code) {
        (true, 0) => "Windows returned no result.".to_owned(),
        (true, _) => format!("Windows error 0x{code:08X}."),
        (false, _) => format!("{message} (0x{code:08X})"),
    }
}

pub(crate) fn start_succeeded(status: i32) -> bool {
    matches!(status, status::SUCCESS | status::ALREADY_ON)
}

/// A failed `TetheringOperationStatus`, with Windows' own detail
/// (`AdditionalErrorMessage`) appended when there is one.
pub(crate) fn explain_status(code: i32, additional: &str, stopping: bool) -> Explanation {
    use Suggestion::*;
    let generic = if stopping {
        "Windows couldn't turn the hotspot off."
    } else {
        "Windows couldn't start the hotspot."
    };
    let (message, suggestion) = match code {
        status::MOBILE_BROADBAND_DEVICE_OFF => (
            "The mobile broadband (cellular) connection is turned off. Turn it on and try again."
                .to_owned(),
            Retry,
        ),
        status::WIFI_DEVICE_OFF => (
            "Wi-Fi is turned off. Turn it on and try again.".to_owned(),
            EnableWifi,
        ),
        status::ENTITLEMENT_CHECK_TIMEOUT => (
            "Your mobile operator didn't confirm in time that sharing is allowed. Try again."
                .to_owned(),
            Retry,
        ),
        status::ENTITLEMENT_CHECK_FAILURE => (
            "Your mobile operator doesn't allow sharing this connection.".to_owned(),
            None,
        ),
        status::OPERATION_IN_PROGRESS => (
            "Windows is already turning the hotspot on or off. Wait a moment and try again."
                .to_owned(),
            Retry,
        ),
        status::BLUETOOTH_DEVICE_OFF => (
            "Bluetooth is turned off. Turn it on and try again.".to_owned(),
            Retry,
        ),
        status::NETWORK_LIMITED_CONNECTIVITY => (
            "The connection being shared has no internet access right now.".to_owned(),
            Retry,
        ),
        status::RADIO_RESTRICTION => (
            "Wi-Fi can't be used right now (airplane mode or a radio restriction). Turn off airplane mode and try again."
                .to_owned(),
            EnableWifi,
        ),
        status::BAND_INTERFERENCE => (
            "The hotspot can't use this band while Wi-Fi is connected on another one. Choose another band."
                .to_owned(),
            ChangeChannel,
        ),
        status::SUCCESS | status::UNKNOWN | status::ALREADY_ON => (generic.to_owned(), Retry),
        other => (
            format!("{} (status {other}).", generic.trim_end_matches('.')),
            Retry,
        ),
    };
    let additional = additional.trim();
    let message = if additional.is_empty() {
        message
    } else {
        format!("{message} Windows says: {additional}")
    };
    explain(message, suggestion)
}

/// Why Windows won't share the connection at all (`TetheringCapability`).
/// `None` when sharing is possible or the cause is unknown.
pub(crate) fn capability_message(value: i32) -> Option<&'static str> {
    match value {
        capability::DISABLED_BY_GROUP_POLICY => {
            Some("Sharing this computer's connection is turned off by your organization's policy.")
        }
        capability::DISABLED_BY_HARDWARE_LIMITATION => {
            Some("This computer's Wi-Fi hardware can't run Windows' Mobile Hotspot.")
        }
        capability::DISABLED_BY_OPERATOR => {
            Some("Your mobile operator doesn't allow sharing this connection.")
        }
        capability::DISABLED_BY_SKU => {
            Some("This connection's service plan doesn't include sharing.")
        }
        capability::DISABLED_BY_REQUIRED_APP_NOT_INSTALLED => {
            Some("Sharing this connection needs your mobile operator's app, which isn't installed.")
        }
        capability::DISABLED_BY_SYSTEM_CAPABILITY => {
            Some("Windows doesn't let apps control the Mobile Hotspot on this computer.")
        }
        capability::ENABLED | capability::DISABLED_DUE_TO_UNKNOWN_CAUSE => None,
        _ => None,
    }
}

/// A failure that means Windows can't tether at all.
pub(crate) fn unavailable_from_hresult(code: i32) -> Option<&'static str> {
    (code == hresult::REGDB_E_CLASSNOTREG)
        .then_some("This edition of Windows doesn't include Mobile Hotspot.")
}

/// Next state from what Windows reports. `starting_for` is the time since
/// the app's own start request succeeded, while that start is being
/// confirmed; `None` otherwise.
pub(crate) fn follow(
    current: &HotspotState,
    op: Op,
    starting_for: Option<Duration>,
) -> HotspotState {
    use HotspotState::*;
    let timed_out = starting_for.is_some_and(|t| t >= START_DEADLINE);
    match op {
        Op::On => Active,
        Op::Off => match (current, starting_for) {
            // Keep the reason a start failed until something else happens.
            (Error(_), _) => current.clone(),
            (Activating, Some(t)) if t < OFF_GRACE => Activating,
            (Activating, Some(_)) => Error(ErrorKind::Activation(explain(
                "Windows turned the hotspot off right after starting it. Try again.",
                Suggestion::Retry,
            ))),
            // Stopped by the app, from Windows' quick settings, or by Windows
            // itself (no devices for a while, shared connection lost).
            _ => Idle,
        },
        Op::InTransition => match current {
            Active | Deactivating => Deactivating,
            // Often Windows still settling after the failure; `On` clears it.
            Error(_) => current.clone(),
            _ if timed_out => Error(ErrorKind::Activation(explain(
                "Starting the hotspot took too long.",
                Suggestion::Retry,
            ))),
            _ => Activating,
        },
        Op::Unknown if timed_out && *current == Activating => Error(ErrorKind::Activation(
            explain("Starting the hotspot took too long.", Suggestion::Retry),
        )),
        Op::Unknown => current.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotspotState::*;

    #[test]
    fn guid_format_matches_network_adapter_id() {
        let id = format_guid(
            0x4D36E972,
            0xE325,
            0x11CE,
            [0xBF, 0xC1, 0x08, 0x00, 0x2B, 0xE1, 0x03, 0x18],
        );
        assert_eq!(id, "4d36e972-e325-11ce-bfc1-08002be10318");
        assert_eq!(normalize_id("{4D36E972-E325-11CE-BFC1-08002BE10318}"), id);
        assert_eq!(normalize_id(" 4d36e972-e325-11ce-bfc1-08002be10318 "), id);
    }

    #[test]
    fn mac_normalization() {
        assert_eq!(normalize_mac("AA-BB-CC-0D-0E-0F"), "aa:bb:cc:0d:0e:0f");
        assert_eq!(normalize_mac("aabbcc0d0e0f"), "aa:bb:cc:0d:0e:0f");
        assert_eq!(normalize_mac("aa:bb:cc:0d:0e:0f"), "aa:bb:cc:0d:0e:0f");
        assert_eq!(normalize_mac("AABB.CC0D.0E0F"), "aa:bb:cc:0d:0e:0f");
        assert_eq!(normalize_mac(" Not-A-MAC "), "not-a-mac");
        assert_eq!(normalize_mac("gg-bb-cc-0d-0e-0f"), "gg-bb-cc-0d-0e-0f");
    }

    #[test]
    fn hostname_cleanup() {
        assert_eq!(
            clean_hostname("Pixel-8.mshome.net").as_deref(),
            Some("Pixel-8")
        );
        assert_eq!(
            clean_hostname("Pixel-8.MSHOME.NET.").as_deref(),
            Some("Pixel-8")
        );
        assert_eq!(clean_hostname("iPhone.local").as_deref(), Some("iPhone"));
        assert_eq!(
            clean_hostname("laptop.home.arpa").as_deref(),
            Some("laptop")
        );
        assert_eq!(
            clean_hostname("build.example.com").as_deref(),
            Some("build.example.com")
        );
        assert_eq!(clean_hostname("").as_deref(), None);
        assert_eq!(clean_hostname(".mshome.net").as_deref(), None);
        assert_eq!(clean_hostname("*").as_deref(), None);
        assert_eq!(clean_hostname("ñandú.lan").as_deref(), Some("ñandú"));
        assert_eq!(clean_hostname("ñ").as_deref(), Some("ñ"));
    }

    #[test]
    fn usb_and_virtual_adapters() {
        assert!(is_usb_device(
            r"\\?\USB#VID_148F&PID_7601#1.0#{ad498944-762f-11d0-8dcb-00c04fc3358c}"
        ));
        assert!(is_usb_device(r"\\?\usb#vid_0bda&pid_8179&mi_00#6&1#{x}"));
        assert!(!is_usb_device(
            r"\\?\PCI#VEN_8086&DEV_A0F0&SUBSYS_00748086#3&11583659&0&A3#{x}"
        ));
        assert!(is_virtual_wifi(
            "Microsoft Wi-Fi Direct Virtual Adapter #2",
            r"\\?\PCI#x"
        ));
        assert!(is_virtual_wifi(
            "Adaptador virtual",
            r"\\?\{5d624f94-8850-40c3-a3fa-a4fd2080baf3}#vwifimp_wfd#5&1#{x}"
        ));
        assert!(!is_virtual_wifi(
            "Intel(R) Wi-Fi 6 AX201 160MHz",
            r"\\?\PCI#x"
        ));
    }

    #[test]
    fn uplink_kinds() {
        assert_eq!(uplink_kind(false, false, 6), UplinkKind::Ethernet);
        assert_eq!(uplink_kind(false, true, 71), UplinkKind::Wifi);
        assert_eq!(uplink_kind(false, false, 71), UplinkKind::Wifi);
        assert_eq!(uplink_kind(true, false, 243), UplinkKind::Cellular);
        assert_eq!(uplink_kind(false, false, 244), UplinkKind::Cellular);
        assert_eq!(uplink_kind(false, false, 131), UplinkKind::Other);
    }

    #[test]
    fn uplink_sorting() {
        let up = |id: &str, name: &str, internet, is_default| Uplink {
            id: id.into(),
            name: name.into(),
            kind: UplinkKind::Other,
            internet,
            is_default,
        };
        let mut list = [
            up("c", "zeta", false, false),
            up("b", "Alpha", false, false),
            up("a", "Wi-Fi", true, false),
            up("d", "Ethernet", true, true),
        ];
        list.sort_by(uplink_order);
        let ids: Vec<_> = list.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, ["d", "a", "b", "c"]);
    }

    #[test]
    fn picking_uplink_and_adapter() {
        let c = [("aaaa-1", false), ("bbbb-2", true), ("cccc-3", false)];
        assert_eq!(pick_uplink(&c, Some("{CCCC-3}")), Some(2));
        assert_eq!(pick_uplink(&c, Some("gone")), Some(1));
        assert_eq!(pick_uplink(&c, None), Some(1));
        assert_eq!(pick_uplink(&[("aaaa-1", false)], None), None);
        assert_eq!(pick_uplink(&[("aaaa-1", false)], Some("AAAA-1")), Some(0));
        assert_eq!(pick_uplink(&[], Some("x")), None);

        let ids = ["aaaa-1", "bbbb-2"];
        assert_eq!(pick_adapter(&ids, Some("BBBB-2")), Some(1));
        assert_eq!(pick_adapter(&ids, Some("zzzz")), None);
        assert_eq!(pick_adapter(&ids, None), None);
    }

    #[test]
    fn settings_comparison() {
        let a = HotspotConfig {
            ssid: "Net".into(),
            password: "password1".into(),
            ap_iface: Some("{AAAA-1}".into()),
            ..HotspotConfig::default()
        };
        let mut b = a.clone();
        b.ap_iface = Some("aaaa-1".into());
        b.autostart = true;
        assert!(same_settings(&a, &b));
        b.band = Band::Ghz5;
        assert!(!same_settings(&a, &b));
    }

    #[test]
    fn bands_round_trip() {
        for band in [Band::Auto, Band::Ghz2_4, Band::Ghz5] {
            assert_eq!(band_from_winrt(band_to_winrt(band)), band);
        }
        assert_eq!(band_from_winrt(3), Band::Auto, "6 GHz");
        assert_eq!(op_from_winrt(1), Op::On);
        assert_eq!(op_from_winrt(2), Op::Off);
        assert_eq!(op_from_winrt(3), Op::InTransition);
        assert_eq!(op_from_winrt(0), Op::Unknown);
    }

    #[test]
    fn scope_address() {
        assert_eq!(
            parse_scope_address("192.168.50.1\0"),
            Some(Ipv4Addr::new(192, 168, 50, 1))
        );
        assert_eq!(parse_scope_address("0.0.0.0"), None);
        assert_eq!(parse_scope_address("nonsense"), None);
    }

    #[test]
    fn status_messages() {
        let off = explain_status(status::WIFI_DEVICE_OFF, "", false);
        assert!(off.message.starts_with("Wi-Fi is turned off"));
        assert_eq!(off.suggestion, Suggestion::EnableWifi);
        assert_eq!(
            explain_status(status::BAND_INTERFERENCE, "", false).suggestion,
            Suggestion::ChangeChannel
        );
        let extra = explain_status(status::UNKNOWN, " Driver said no. ", false);
        assert_eq!(
            extra.message,
            "Windows couldn't start the hotspot. Windows says: Driver said no."
        );
        assert_eq!(
            explain_status(42, "", true).message,
            "Windows couldn't turn the hotspot off (status 42)."
        );
        assert!(start_succeeded(status::SUCCESS));
        assert!(start_succeeded(status::ALREADY_ON));
        assert!(!start_succeeded(status::OPERATION_IN_PROGRESS));
    }

    #[test]
    fn capability_and_hresult_messages() {
        assert_eq!(capability_message(capability::ENABLED), None);
        assert_eq!(
            capability_message(capability::DISABLED_DUE_TO_UNKNOWN_CAUSE),
            None
        );
        assert!(capability_message(capability::DISABLED_BY_GROUP_POLICY).is_some());
        assert_eq!(capability_message(99), None);
        assert!(unavailable_from_hresult(hresult::REGDB_E_CLASSNOTREG).is_some());
        assert_eq!(unavailable_from_hresult(hresult::E_INVALIDARG), None);
    }

    #[test]
    fn call_texts() {
        assert_eq!(
            call_text("Access is denied.\r\n", 0x8007_0005_u32 as i32),
            "Access is denied. (0x80070005)"
        );
        assert_eq!(
            call_text("", 0x8007_0005_u32 as i32),
            "Windows error 0x80070005."
        );
        assert_eq!(call_text("", 0), "Windows returned no result.");
    }

    #[test]
    fn following_windows() {
        let err = Error(ErrorKind::NoApAdapter);
        // Started or stopped outside the app.
        assert_eq!(follow(&Idle, Op::On, None), Active);
        assert_eq!(follow(&Active, Op::Off, None), Idle);
        assert_eq!(follow(&Idle, Op::InTransition, None), Activating);
        assert_eq!(follow(&Active, Op::InTransition, None), Deactivating);
        assert_eq!(follow(&Activating, Op::Off, None), Idle);
        // Errors stay until Windows reports the hotspot on.
        assert_eq!(follow(&err, Op::Off, None), err);
        assert_eq!(follow(&err, Op::On, None), Active);
        assert_eq!(follow(&err, Op::Unknown, None), err);
        assert_eq!(follow(&err, Op::InTransition, None), err);
        // Stop requested by the app.
        assert_eq!(follow(&Deactivating, Op::Off, None), Idle);
        assert_eq!(follow(&Deactivating, Op::InTransition, None), Deactivating);
    }

    #[test]
    fn confirming_the_apps_start() {
        let soon = Some(Duration::from_secs(2));
        let later = Some(OFF_GRACE + Duration::from_secs(1));
        let too_late = Some(START_DEADLINE);
        assert_eq!(follow(&Activating, Op::On, soon), Active);
        assert_eq!(follow(&Activating, Op::Off, soon), Activating);
        assert!(matches!(
            follow(&Activating, Op::Off, later),
            Error(ErrorKind::Activation(_))
        ));
        assert_eq!(follow(&Activating, Op::InTransition, later), Activating);
        assert!(matches!(
            follow(&Activating, Op::InTransition, too_late),
            Error(ErrorKind::Activation(_))
        ));
        assert_eq!(follow(&Activating, Op::Unknown, soon), Activating);
        assert!(matches!(
            follow(&Activating, Op::Unknown, too_late),
            Error(ErrorKind::Activation(_))
        ));
    }
}
