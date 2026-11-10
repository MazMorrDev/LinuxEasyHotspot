//! Minimal DNS PTR query/response codec.
//!
//! NM's dnsmasq (Router mode) registers DHCP client hostnames, so asking it
//! for the PTR record of a client's IP returns the hostname. The lease file
//! would be simpler but it is root-only on many distros.

use std::net::Ipv4Addr;

/// Builds a recursion-desired PTR query for `ip` with transaction `id`.
pub fn ptr_query(id: u16, ip: Ipv4Addr) -> Vec<u8> {
    let o = ip.octets();
    let name = format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0]);
    let mut q = Vec::with_capacity(12 + name.len() + 6);
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // flags: RD
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // QD=1, AN=NS=AR=0
    for label in name.split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&[0, 12, 0, 1]); // QTYPE=PTR, QCLASS=IN
    q
}

/// Extracts the first PTR answer's host label (without the domain suffix)
/// from a response to transaction `id`. Returns `None` on any malformation.
pub fn parse_ptr_response(id: u16, msg: &[u8]) -> Option<String> {
    if msg.len() < 12 || msg[0..2] != id.to_be_bytes() || msg[2] & 0x80 == 0 {
        return None;
    }
    if msg[3] & 0x0f != 0 {
        return None; // RCODE != NOERROR
    }
    let qd = u16::from_be_bytes([msg[4], msg[5]]);
    let an = u16::from_be_bytes([msg[6], msg[7]]);
    let mut pos = 12;
    for _ in 0..qd {
        pos = skip_name(msg, pos)? + 4;
    }
    for _ in 0..an {
        pos = skip_name(msg, pos)?;
        let rtype = u16::from_be_bytes([*msg.get(pos)?, *msg.get(pos + 1)?]);
        let rdlen = u16::from_be_bytes([*msg.get(pos + 8)?, *msg.get(pos + 9)?]) as usize;
        let rdata = pos + 10;
        if rtype == 12 {
            let name = read_name(msg, rdata)?;
            let host = name.split('.').next()?.to_owned();
            return (!host.is_empty()).then_some(host);
        }
        pos = rdata + rdlen;
    }
    None
}

fn skip_name(msg: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *msg.get(pos)?;
        match len {
            0 => return Some(pos + 1),
            l if l & 0xc0 == 0xc0 => return Some(pos + 2),
            l => pos += 1 + l as usize,
        }
    }
}

fn read_name(msg: &[u8], mut pos: usize) -> Option<String> {
    let mut labels = Vec::new();
    for _ in 0..32 {
        let len = *msg.get(pos)?;
        match len {
            0 => return Some(labels.join(".")),
            l if l & 0xc0 == 0xc0 => {
                pos = (((l & 0x3f) as usize) << 8) | *msg.get(pos + 1)? as usize;
            }
            l => {
                let bytes = msg.get(pos + 1..pos + 1 + l as usize)?;
                labels.push(String::from_utf8_lossy(bytes).into_owned());
                pos += 1 + l as usize;
            }
        }
    }
    None // pointer loop
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn response(id: u16, ip: Ipv4Addr, host: &str) -> Vec<u8> {
        let mut r = ptr_query(id, ip);
        r[2] = 0x81;
        r[3] = 0x80;
        r[7] = 1; // ANCOUNT=1
        r.extend_from_slice(&[0xc0, 0x0c, 0, 12, 0, 1, 0, 0, 0, 0]);
        let mut rdata = Vec::new();
        for label in [host, "lan"] {
            rdata.push(label.len() as u8);
            rdata.extend_from_slice(label.as_bytes());
        }
        rdata.push(0);
        r.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        r.extend_from_slice(&rdata);
        r
    }

    #[test]
    fn query_encoding() {
        let q = ptr_query(0xabcd, Ipv4Addr::new(10, 42, 0, 23));
        assert_eq!(&q[0..2], &[0xab, 0xcd]);
        assert!(q.windows(8).any(|w| w == b"\x07in-addr"));
        assert_eq!(&q[q.len() - 4..], &[0, 12, 0, 1]);
    }

    #[test]
    fn parses_answer_with_compression() {
        let ip = Ipv4Addr::new(10, 42, 0, 23);
        let r = response(7, ip, "Pixel-8");
        assert_eq!(parse_ptr_response(7, &r).as_deref(), Some("Pixel-8"));
        assert_eq!(parse_ptr_response(8, &r), None, "wrong id");
    }

    #[test]
    fn nxdomain_and_truncation() {
        let ip = Ipv4Addr::new(10, 42, 0, 23);
        let mut r = response(7, ip, "x");
        r[3] = 0x83;
        assert_eq!(parse_ptr_response(7, &r), None);
        let r = response(7, ip, "x");
        assert_eq!(parse_ptr_response(7, &r[..r.len() - 3]), None);
    }

    proptest! {
        #[test]
        fn never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
            let _ = parse_ptr_response(u16::from_be_bytes([bytes.first().copied().unwrap_or(0), bytes.get(1).copied().unwrap_or(0)]), &bytes);
        }
    }
}
