//! Desired traffic-control state → validated `tc` commands.
//!
//! Pure on purpose: the privileged helper builds its commands *only* through
//! this module from a structured request, so every input is validated here and
//! the exact output is snapshot-tested.
//!
//! Layout:
//! - Download (to devices): HTB on the AP interface's egress, one class per
//!   device matched by `flower dst_mac`, `fq_codel` leaves.
//! - Upload (from devices): ingress redirected to an IFB device, same HTB tree
//!   matched by `flower src_mac`.
//! - Pending/blocked devices: on ingress, DHCP passes, everything else drops.

use std::fmt::Write as _;

use thiserror::Error;

use crate::policy::{Access, Priority};

pub const IFB_IFACE: &str = "lehifb0";
/// "Auto" link speed when the user didn't set a total.
pub const AUTO_TOTAL_KBPS: u32 = 1_000_000;
pub const MIN_KBPS: u32 = 8;
pub const MAX_KBPS: u32 = 10_000_000;
pub const MAX_DEVICES: usize = 250;
/// HTB class minor for traffic not matching any device (hex in tc syntax).
const DEFAULT_MINOR: u32 = 0x9999;
const FIRST_MINOR: u32 = 0x10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeDevice {
    /// `aa:bb:cc:dd:ee:ff`
    pub mac: String,
    pub access: Access,
    pub down_kbps: Option<u32>,
    pub up_kbps: Option<u32>,
    pub priority: Priority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeState {
    pub iface: String,
    pub total_down_kbps: Option<u32>,
    pub total_up_kbps: Option<u32>,
    pub devices: Vec<ShapeDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TcError {
    #[error("invalid interface name {0:?}")]
    Iface(String),
    #[error("invalid MAC address {0:?}")]
    Mac(String),
    #[error("rate {0} kbit/s is out of range ({MIN_KBPS}–{MAX_KBPS})")]
    Rate(u32),
    #[error("too many devices ({0}, max {MAX_DEVICES})")]
    TooMany(usize),
    #[error("device {0} is listed twice")]
    Duplicate(String),
}

pub fn valid_iface(s: &str) -> bool {
    (1..=15).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub fn valid_mac(s: &str) -> bool {
    s.len() == 17
        && s.split(':').count() == 6
        && s.split(':').all(|o| {
            o.len() == 2
                && o.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

fn check_rate(r: Option<u32>) -> Result<(), TcError> {
    match r {
        Some(k) if !(MIN_KBPS..=MAX_KBPS).contains(&k) => Err(TcError::Rate(k)),
        _ => Ok(()),
    }
}

pub fn validate(s: &ShapeState) -> Result<(), TcError> {
    if !valid_iface(&s.iface) || s.iface == IFB_IFACE {
        return Err(TcError::Iface(s.iface.clone()));
    }
    if s.devices.len() > MAX_DEVICES {
        return Err(TcError::TooMany(s.devices.len()));
    }
    check_rate(s.total_down_kbps)?;
    check_rate(s.total_up_kbps)?;
    let mut seen = std::collections::HashSet::new();
    for d in &s.devices {
        if !valid_mac(&d.mac) {
            return Err(TcError::Mac(d.mac.clone()));
        }
        if !seen.insert(&d.mac) {
            return Err(TcError::Duplicate(d.mac.clone()));
        }
        check_rate(d.down_kbps)?;
        check_rate(d.up_kbps)?;
    }
    Ok(())
}

/// Whether any shaping is needed at all (otherwise the helper just clears).
pub fn is_noop(s: &ShapeState) -> bool {
    s.total_down_kbps.is_none()
        && s.total_up_kbps.is_none()
        && s.devices.iter().all(|d| {
            d.access == Access::Allowed
                && d.down_kbps.is_none()
                && d.up_kbps.is_none()
                && d.priority == Priority::Normal
        })
}

/// The class minor (hex in tc syntax) used for the device at `index`.
pub fn class_minor(index: usize) -> u32 {
    FIRST_MINOR + index as u32
}

fn weight(p: Priority) -> u64 {
    match p {
        Priority::High => 3,
        Priority::Normal => 2,
        Priority::Low => 1,
    }
}

fn htb_prio(p: Priority) -> u8 {
    match p {
        Priority::High => 0,
        Priority::Normal => 1,
        Priority::Low => 2,
    }
}

/// Commands to run, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// `ip` argument vectors to create the IFB device (errors if it already exists are fine).
    pub ip_setup: Vec<Vec<String>>,
    /// `tc` argument vectors that remove previous state; errors are ignored.
    pub teardown: Vec<Vec<String>>,
    /// Script for `tc -batch -` (one command per line, without the `tc` prefix).
    pub batch: String,
}

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

/// Commands that remove everything this module installs.
pub fn teardown(iface: &str) -> Vec<Vec<String>> {
    vec![
        argv(&format!("qdisc del dev {iface} root")),
        argv(&format!("qdisc del dev {iface} ingress")),
        argv(&format!("qdisc del dev {IFB_IFACE} root")),
    ]
}

fn tree(out: &mut String, dev: &str, total: u32, s: &ShapeState, up: bool) {
    let _ = writeln!(
        out,
        "qdisc add dev {dev} root handle 1: htb default {DEFAULT_MINOR:x} r2q 10"
    );
    let _ = writeln!(
        out,
        "class add dev {dev} parent 1: classid 1:1 htb rate {total}kbit ceil {total}kbit quantum 1514"
    );
    let _ = writeln!(
        out,
        "class add dev {dev} parent 1:1 classid 1:{DEFAULT_MINOR:x} htb rate {MIN_KBPS}kbit ceil {total}kbit prio 1 quantum 1514"
    );
    let _ = writeln!(
        out,
        "qdisc add dev {dev} parent 1:{DEFAULT_MINOR:x} fq_codel"
    );
    let allowed: Vec<_> = s
        .devices
        .iter()
        .filter(|d| d.access == Access::Allowed)
        .collect();
    let weights: u64 = allowed
        .iter()
        .map(|d| weight(d.priority))
        .sum::<u64>()
        .max(1);
    for (i, d) in s.devices.iter().enumerate() {
        if d.access != Access::Allowed {
            continue;
        }
        let minor = class_minor(i);
        let limit = if up { d.up_kbps } else { d.down_kbps };
        let ceil = limit.map_or(total, |l| l.min(total));
        let share = (u64::from(total) * weight(d.priority) / weights) as u32;
        let rate = share.clamp(MIN_KBPS, ceil.max(MIN_KBPS));
        let _ = writeln!(
            out,
            "class add dev {dev} parent 1:1 classid 1:{minor:x} htb rate {rate}kbit ceil {ceil}kbit prio {} quantum 1514",
            htb_prio(d.priority)
        );
        let _ = writeln!(out, "qdisc add dev {dev} parent 1:{minor:x} fq_codel");
        let dir = if up { "src_mac" } else { "dst_mac" };
        let _ = writeln!(
            out,
            "filter add dev {dev} parent 1: protocol all prio 10 flower {dir} {} classid 1:{minor:x}",
            d.mac
        );
    }
}

/// Builds the full plan. Call [`validate`] first (done here too).
pub fn plan(s: &ShapeState) -> Result<Plan, TcError> {
    validate(s)?;
    let iface = &s.iface;
    let down_total = s.total_down_kbps.unwrap_or(AUTO_TOTAL_KBPS);
    let up_total = s.total_up_kbps.unwrap_or(AUTO_TOTAL_KBPS);

    let mut b = String::new();
    // Download.
    tree(&mut b, iface, down_total, s, false);
    // Upload: ingress filters on the AP interface.
    let _ = writeln!(b, "qdisc add dev {iface} handle ffff: ingress");
    for d in s.devices.iter().filter(|d| d.access != Access::Allowed) {
        let _ = writeln!(
            b,
            "filter add dev {iface} parent ffff: protocol ip prio 1 flower src_mac {} ip_proto udp dst_port 67 action pass",
            d.mac
        );
        let _ = writeln!(
            b,
            "filter add dev {iface} parent ffff: protocol all prio 2 flower src_mac {} action drop",
            d.mac
        );
    }
    let _ = writeln!(
        b,
        "filter add dev {iface} parent ffff: protocol all prio 10 matchall action mirred egress redirect dev {IFB_IFACE}"
    );
    tree(&mut b, IFB_IFACE, up_total, s, true);

    Ok(Plan {
        ip_setup: vec![
            argv(&format!("link add {IFB_IFACE} type ifb")),
            argv(&format!("link set {IFB_IFACE} up")),
        ],
        teardown: teardown(iface),
        batch: b,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn dev(
        mac: &str,
        access: Access,
        down: Option<u32>,
        up: Option<u32>,
        p: Priority,
    ) -> ShapeDevice {
        ShapeDevice {
            mac: mac.into(),
            access,
            down_kbps: down,
            up_kbps: up,
            priority: p,
        }
    }

    fn state(devices: Vec<ShapeDevice>) -> ShapeState {
        ShapeState {
            iface: "wlp2s0".into(),
            total_down_kbps: Some(20_000),
            total_up_kbps: None,
            devices,
        }
    }

    #[test]
    fn snapshot_mixed() {
        let s = state(vec![
            dev(
                "3c:28:6d:11:22:33",
                Access::Allowed,
                Some(5_000),
                Some(1_000),
                Priority::High,
            ),
            dev(
                "ac:bc:32:00:00:41",
                Access::Allowed,
                None,
                None,
                Priority::Normal,
            ),
            dev(
                "a4:cf:12:9e:00:57",
                Access::Allowed,
                Some(256),
                Some(256),
                Priority::Low,
            ),
            dev(
                "da:a1:19:42:00:88",
                Access::Pending,
                None,
                None,
                Priority::Normal,
            ),
            dev(
                "5c:0a:5b:00:00:99",
                Access::Blocked,
                None,
                None,
                Priority::Normal,
            ),
        ]);
        insta::assert_snapshot!(plan(&s).unwrap().batch);
    }

    #[test]
    fn snapshot_empty() {
        insta::assert_snapshot!(plan(&state(vec![])).unwrap().batch);
    }

    #[test]
    fn limit_never_exceeds_total_and_rate_never_exceeds_ceil() {
        let s = state(vec![dev(
            "aa:00:00:00:00:01",
            Access::Allowed,
            Some(50_000),
            None,
            Priority::High,
        )]);
        let b = plan(&s).unwrap().batch;
        assert!(
            b.contains("classid 1:10 htb rate 20000kbit ceil 20000kbit prio 0 quantum 1514"),
            "{b}"
        );
    }

    #[test]
    fn validation() {
        let mut s = state(vec![]);
        s.iface = "eth0; rm -rf /".into();
        assert!(matches!(validate(&s), Err(TcError::Iface(_))));
        s.iface = IFB_IFACE.into();
        assert!(matches!(validate(&s), Err(TcError::Iface(_))));
        let s = state(vec![dev(
            "AA:00:00:00:00:01",
            Access::Allowed,
            None,
            None,
            Priority::Normal,
        )]);
        assert!(
            matches!(validate(&s), Err(TcError::Mac(_))),
            "uppercase is normalized by the caller"
        );
        let s = state(vec![dev(
            "aa:00:00:00:00:01",
            Access::Allowed,
            Some(1),
            None,
            Priority::Normal,
        )]);
        assert_eq!(validate(&s), Err(TcError::Rate(1)));
        let d = dev(
            "aa:00:00:00:00:01",
            Access::Allowed,
            None,
            None,
            Priority::Normal,
        );
        assert!(matches!(
            validate(&state(vec![d.clone(), d])),
            Err(TcError::Duplicate(_))
        ));
        let many = (0..=MAX_DEVICES)
            .map(|i| {
                dev(
                    &format!("aa:00:00:00:{:02x}:{:02x}", i / 256, i % 256),
                    Access::Allowed,
                    None,
                    None,
                    Priority::Normal,
                )
            })
            .collect();
        assert_eq!(
            validate(&state(many)),
            Err(TcError::TooMany(MAX_DEVICES + 1))
        );
    }

    #[test]
    fn noop_detection() {
        let mut s = state(vec![dev(
            "aa:00:00:00:00:01",
            Access::Allowed,
            None,
            None,
            Priority::Normal,
        )]);
        s.total_down_kbps = None;
        assert!(is_noop(&s));
        s.devices[0].access = Access::Pending;
        assert!(!is_noop(&s));
    }

    proptest! {
        #[test]
        fn only_safe_tokens(n in 0usize..20, total in MIN_KBPS..MAX_KBPS, lim in proptest::option::of(MIN_KBPS..MAX_KBPS)) {
            let devices = (0..n)
                .map(|i| dev(&format!("aa:00:00:00:00:{i:02x}"), Access::Allowed, lim, lim, Priority::Normal))
                .collect();
            let mut s = state(devices);
            s.total_down_kbps = Some(total);
            let b = plan(&s).unwrap().batch;
            prop_assert!(b.chars().all(|c| c.is_ascii_alphanumeric() || " :_-\n".contains(c)));
        }
    }
}
