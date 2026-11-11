//! DHCP client request parser (for name snooping in Bridge mode).
//!
//! In Bridge mode the upstream router hands out addresses, so this computer
//! never sees a lease. Clients still broadcast their DHCPDISCOVER/REQUEST
//! through the bridge, and those carry the device's own name (option 12),
//! the name routers show in their client lists.

use std::net::Ipv4Addr;

const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
const OPT_PAD: u8 = 0;
const OPT_HOSTNAME: u8 = 12;
const OPT_REQUESTED_IP: u8 = 50;
const OPT_CLIENT_FQDN: u8 = 81;
const OPT_END: u8 = 255;
const MAX_NAME: usize = 63;

/// What a client said about itself in one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    /// Lower-case `aa:bb:cc:dd:ee:ff`.
    pub mac: String,
    pub hostname: Option<String>,
    /// `ciaddr`, or the requested address (option 50).
    pub ip: Option<Ipv4Addr>,
}

/// Parses a BOOTREQUEST (client → server). Returns `None` for replies,
/// non-Ethernet hardware, or anything malformed.
pub fn parse_request(msg: &[u8]) -> Option<Sighting> {
    if msg.len() < 240 || msg[0] != 1 || msg[1] != 1 || msg[2] != 6 {
        return None;
    }
    if msg[236..240] != MAGIC_COOKIE {
        return None;
    }
    let mac = msg[28..34]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":");
    let ciaddr = Ipv4Addr::new(msg[12], msg[13], msg[14], msg[15]);
    let mut ip = (!ciaddr.is_unspecified()).then_some(ciaddr);
    let mut hostname = None;
    let mut fqdn = None;

    let mut pos = 240;
    while let Some(&code) = msg.get(pos) {
        match code {
            OPT_PAD => {
                pos += 1;
                continue;
            }
            OPT_END => break,
            _ => {}
        }
        let len = *msg.get(pos + 1)? as usize;
        let data = msg.get(pos + 2..pos + 2 + len)?;
        match code {
            OPT_HOSTNAME => hostname = clean(data),
            OPT_REQUESTED_IP if len == 4 && ip.is_none() => {
                ip = Some(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
            }
            OPT_CLIENT_FQDN if len > 3 => fqdn = fqdn_host(data[0], &data[3..]),
            _ => {}
        }
        pos += 2 + len;
    }
    Some(Sighting {
        mac,
        hostname: hostname.or(fqdn),
        ip,
    })
}

/// Option 81: flags, two obsolete rcodes, then the name, wire-encoded when
/// the E flag (0x04) is set and ASCII otherwise. Only the host label is kept.
fn fqdn_host(flags: u8, name: &[u8]) -> Option<String> {
    if flags & 0x04 != 0 {
        let len = *name.first()? as usize;
        clean(name.get(1..1 + len)?)
    } else {
        clean(name.split(|&b| b == b'.').next()?)
    }
}

/// Lossy UTF-8, trailing NULs dropped, control characters removed, capped.
fn clean(raw: &[u8]) -> Option<String> {
    let s: String = String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn request(options: &[u8]) -> Vec<u8> {
        let mut m = vec![0u8; 240];
        m[0] = 1;
        m[1] = 1;
        m[2] = 6;
        m[28..34].copy_from_slice(&[0xF0, 0x6C, 0x5D, 0x43, 0x79, 0xA0]);
        m[236..240].copy_from_slice(&MAGIC_COOKIE);
        m.extend_from_slice(options);
        m
    }

    #[test]
    fn hostname_and_requested_ip() {
        let mut opts = vec![53, 1, 3, 0, 50, 4, 192, 168, 30, 4, 12, 8];
        opts.extend_from_slice(b"Redmi-12");
        opts.push(255);
        let s = parse_request(&request(&opts)).unwrap();
        assert_eq!(s.mac, "f0:6c:5d:43:79:a0");
        assert_eq!(s.hostname.as_deref(), Some("Redmi-12"));
        assert_eq!(s.ip, Some(Ipv4Addr::new(192, 168, 30, 4)));
    }

    #[test]
    fn fqdn_fallback_ascii_and_wire() {
        let mut ascii = vec![81, 13, 0, 0, 0];
        ascii.extend_from_slice(b"laptop.lan");
        ascii.push(255);
        ascii[1] = 3 + 10;
        let s = parse_request(&request(&ascii)).unwrap();
        assert_eq!(s.hostname.as_deref(), Some("laptop"));

        let mut wire = vec![81, 0, 0x04, 0, 0, 6];
        wire.extend_from_slice(b"iPhone");
        wire.extend_from_slice(&[3, b'l', b'a', b'n', 0]);
        wire[1] = (wire.len() - 2) as u8;
        wire.push(255);
        let s = parse_request(&request(&wire)).unwrap();
        assert_eq!(s.hostname.as_deref(), Some("iPhone"));
    }

    #[test]
    fn rejects_replies_and_garbage() {
        let mut r = request(&[255]);
        r[0] = 2;
        assert_eq!(parse_request(&r), None);
        assert_eq!(parse_request(&[1, 1, 6]), None);
        let mut bad_cookie = request(&[255]);
        bad_cookie[236] = 0;
        assert_eq!(parse_request(&bad_cookie), None);
    }

    #[test]
    fn truncated_option_is_rejected_and_names_cleaned() {
        assert_eq!(parse_request(&request(&[12, 20, b'a'])), None);
        let s = parse_request(&request(&[12, 4, b'a', 0, b'\n', b' ', 255])).unwrap();
        assert_eq!(s.hostname.as_deref(), Some("a"));
        let s = parse_request(&request(&[12, 2, 0, 0, 255])).unwrap();
        assert_eq!(s.hostname, None);
    }

    proptest! {
        #[test]
        fn never_panics(tail in proptest::collection::vec(any::<u8>(), 0..300)) {
            let _ = parse_request(&request(&tail));
        }
    }
}
