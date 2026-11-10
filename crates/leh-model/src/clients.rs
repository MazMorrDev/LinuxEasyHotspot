//! Connected-device model, merged from several sources.

use std::collections::BTreeMap;
use std::net::IpAddr;

use crate::parse::oui;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Client {
    /// Lower-case `aa:bb:cc:dd:ee:ff`.
    pub mac: String,
    pub ip: Option<IpAddr>,
    pub hostname: Option<String>,
}

impl Client {
    /// Vendor name from the MAC prefix, or "Private address" for randomized
    /// (locally administered) MACs, which most modern phones use.
    pub fn vendor(&self) -> Option<&'static str> {
        if is_randomized_mac(&self.mac) {
            return Some("Private address");
        }
        oui::lookup(&self.mac)
    }

    pub fn display_name(&self) -> &str {
        self.hostname.as_deref().unwrap_or("Unknown device")
    }
}

/// Bit 1 of the first octet marks a locally administered address.
pub fn is_randomized_mac(mac: &str) -> bool {
    mac.get(0..2)
        .and_then(|b| u8::from_str_radix(b, 16).ok())
        .is_some_and(|b| b & 0x02 != 0)
}

/// Merges observations keyed by MAC; later sources fill in missing fields.
pub fn merge(sources: impl IntoIterator<Item = Client>) -> Vec<Client> {
    let mut by_mac: BTreeMap<String, Client> = BTreeMap::new();
    for c in sources {
        let entry = by_mac
            .entry(c.mac.to_ascii_lowercase())
            .or_insert_with(|| Client {
                mac: c.mac.to_ascii_lowercase(),
                ..Client::default()
            });
        entry.ip = entry.ip.or(c.ip);
        if entry.hostname.is_none() {
            entry.hostname = c.hostname.filter(|h| !h.is_empty() && h != "*");
        }
    }
    let mut list: Vec<_> = by_mac.into_values().collect();
    list.sort_by(|a, b| a.ip.cmp(&b.ip).then_with(|| a.mac.cmp(&b.mac)));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn randomized_mac_detection() {
        assert!(is_randomized_mac("da:a1:19:00:00:01"));
        assert!(!is_randomized_mac("3c:28:6d:00:00:01"));
        assert!(!is_randomized_mac("zz"));
    }

    #[test]
    fn merge_fills_gaps() {
        let merged = merge([
            Client {
                mac: "AA:00:00:00:00:01".into(),
                ip: Some("10.42.0.23".parse().unwrap()),
                hostname: None,
            },
            Client {
                mac: "aa:00:00:00:00:01".into(),
                ip: None,
                hostname: Some("Pixel-8".into()),
            },
            Client {
                mac: "aa:00:00:00:00:02".into(),
                ip: Some("10.42.0.10".parse().unwrap()),
                hostname: Some("*".into()),
            },
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].mac, "aa:00:00:00:00:02");
        assert_eq!(merged[0].display_name(), "Unknown device");
        assert_eq!(merged[1].display_name(), "Pixel-8");
        assert_eq!(merged[1].ip, Some("10.42.0.23".parse().unwrap()));
    }
}
