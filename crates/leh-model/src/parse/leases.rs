//! dnsmasq lease file parser.
//!
//! Format, one lease per line: `<expiry> <mac> <ip> <hostname|*> <client-id|*>`.
//! NM keeps the file at `/var/lib/NetworkManager/dnsmasq-<iface>.leases`, which
//! is root-only on many distros (including Ubuntu/Mint), so this is an optional
//! source: the main one is the kernel neighbor table plus reverse DNS.

use crate::clients::Client;

/// Parses as many leases as possible; malformed or half-written lines are skipped.
pub fn parse(text: &str) -> Vec<Client> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Client> {
    let mut f = line.split_ascii_whitespace();
    let _expiry: u64 = f.next()?.parse().ok()?;
    let mac = f.next()?;
    if !looks_like_mac(mac) {
        return None;
    }
    let ip = f.next()?.parse().ok()?;
    let hostname = f.next().filter(|h| *h != "*").map(str::to_owned);
    Some(Client {
        mac: mac.to_ascii_lowercase(),
        ip: Some(ip),
        hostname,
    })
}

fn looks_like_mac(s: &str) -> bool {
    s.len() == 17
        && s.split(':')
            .all(|o| o.len() == 2 && o.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    #[test]
    fn empty_file() {
        assert!(parse(&fixture("leases-empty")).is_empty());
    }

    #[test]
    fn normal_file() {
        let l = parse(&fixture("leases-normal"));
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].hostname.as_deref(), Some("Pixel-8"));
        assert_eq!(l[1].hostname, None);
    }

    #[test]
    fn partial_last_line_is_skipped() {
        let l = parse(&fixture("leases-partial-line"));
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn utf8_hostname() {
        let l = parse(&fixture("leases-utf8-hostname"));
        assert_eq!(l[0].hostname.as_deref(), Some("iPhone-de-José"));
    }

    proptest! {
        #[test]
        fn never_panics(s in "\\PC*") {
            let _ = parse(&s);
        }
    }
}
