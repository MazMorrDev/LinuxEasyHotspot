//! Small embedded MAC-prefix → vendor table.
//!
//! Deliberately a curated list of common consumer vendors rather than the full
//! IEEE registry (megabytes, refreshed constantly). Format of `oui.csv`:
//! `aabbcc,Vendor` per line.

static TABLE: &str = include_str!("oui.csv");

/// Looks up the vendor for `aa:bb:cc:dd:ee:ff` (any case).
pub fn lookup(mac: &str) -> Option<&'static str> {
    let prefix: String = mac
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(6)
        .collect::<String>()
        .to_ascii_lowercase();
    if prefix.len() != 6 {
        return None;
    }
    TABLE.lines().find_map(|line| {
        let (p, vendor) = line.split_once(',')?;
        (p == prefix).then_some(vendor.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert_eq!(lookup("3C:28:6D:11:22:33"), Some("Google"));
        assert_eq!(lookup("ff:ff:ff:00:00:00"), None);
        assert_eq!(lookup("3c"), None);
    }

    #[test]
    fn table_is_well_formed() {
        for line in TABLE.lines() {
            let (p, v) = line.split_once(',').expect(line);
            assert_eq!(p.len(), 6, "{line}");
            assert!(
                p.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "{line}"
            );
            assert!(!v.trim().is_empty(), "{line}");
        }
    }
}
