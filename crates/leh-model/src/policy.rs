//! Per-device policies (speed limits, priority, approval, data caps) and the
//! defaults applied to devices the first time they connect.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    #[default]
    Allowed,
    /// Waiting for the user to approve. Only DHCP is let through.
    Pending,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Priority {
    High,
    #[default]
    Normal,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapAction {
    #[default]
    Block,
    /// Slow the device down to this rate (both directions).
    Throttle { kbps: u32 },
}

/// The tunable limits, shared by defaults and per-device policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Limits {
    /// `None` = unlimited.
    pub down_kbps: Option<u32>,
    pub up_kbps: Option<u32>,
    pub priority: Priority,
    /// Data allowed per hotspot session, `None` = no cap.
    pub cap_mb: Option<u32>,
    pub cap_action: CapAction,
}

/// Settings applied to devices seen for the first time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DefaultPolicy {
    pub require_approval: bool,
    #[serde(flatten)]
    pub limits: Limits,
}

/// What we remember about one device (keyed by MAC elsewhere).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DevicePolicy {
    /// User-chosen name, overrides the hostname.
    pub alias: Option<String>,
    /// Name the device reported last time it was connected, so it can be
    /// recognized in the list while it is away.
    pub hostname: Option<String>,
    /// When the device was last connected (Unix time, seconds).
    pub last_seen: Option<u64>,
    pub access: Access,
    /// While true, `limits` is ignored and the current defaults apply.
    pub follow_defaults: bool,
    pub limits: Limits,
}

impl Default for DevicePolicy {
    fn default() -> Self {
        Self {
            alias: None,
            hostname: None,
            last_seen: None,
            access: Access::Allowed,
            follow_defaults: true,
            limits: Limits::default(),
        }
    }
}

impl DevicePolicy {
    /// The record created when a device connects for the first time.
    pub fn new_device(defaults: &DefaultPolicy) -> Self {
        Self {
            access: if defaults.require_approval {
                Access::Pending
            } else {
                Access::Allowed
            },
            limits: defaults.limits,
            ..Self::default()
        }
    }

    pub fn limits<'a>(&'a self, defaults: &'a DefaultPolicy) -> &'a Limits {
        if self.follow_defaults {
            &defaults.limits
        } else {
            &self.limits
        }
    }
}

/// The policy actually enforced right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effective {
    pub access: Access,
    pub down_kbps: Option<u32>,
    pub up_kbps: Option<u32>,
    pub priority: Priority,
    /// The session data cap has been reached (and its action applied).
    pub cap_reached: bool,
}

const MB: u64 = 1_000_000;

/// Combines a device's policy, the defaults and this session's usage.
pub fn resolve(defaults: &DefaultPolicy, dev: &DevicePolicy, session_bytes: u64) -> Effective {
    let l = dev.limits(defaults);
    let mut eff = Effective {
        access: dev.access,
        down_kbps: l.down_kbps,
        up_kbps: l.up_kbps,
        priority: l.priority,
        cap_reached: false,
    };
    if let Some(cap) = l.cap_mb
        && session_bytes >= u64::from(cap) * MB
        && eff.access == Access::Allowed
    {
        eff.cap_reached = true;
        match l.cap_action {
            CapAction::Block => eff.access = Access::Blocked,
            CapAction::Throttle { kbps } => {
                let clamp = |v: Option<u32>| Some(v.map_or(kbps, |v| v.min(kbps)));
                eff.down_kbps = clamp(eff.down_kbps);
                eff.up_kbps = clamp(eff.up_kbps);
            }
        }
    }
    eff
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn defaults() -> DefaultPolicy {
        DefaultPolicy {
            require_approval: false,
            limits: Limits {
                down_kbps: Some(10_000),
                up_kbps: Some(2_000),
                priority: Priority::Normal,
                cap_mb: Some(500),
                cap_action: CapAction::Block,
            },
        }
    }

    #[test]
    fn new_device_respects_approval() {
        let mut d = defaults();
        assert_eq!(DevicePolicy::new_device(&d).access, Access::Allowed);
        d.require_approval = true;
        assert_eq!(DevicePolicy::new_device(&d).access, Access::Pending);
    }

    #[test]
    fn follow_defaults_tracks_changes() {
        let mut d = defaults();
        let dev = DevicePolicy::new_device(&d);
        d.limits.down_kbps = Some(1_000);
        assert_eq!(resolve(&d, &dev, 0).down_kbps, Some(1_000));
    }

    #[test]
    fn override_ignores_defaults() {
        let d = defaults();
        let dev = DevicePolicy {
            follow_defaults: false,
            limits: Limits {
                priority: Priority::High,
                ..Limits::default()
            },
            ..DevicePolicy::default()
        };
        let e = resolve(&d, &dev, 10_000 * MB);
        assert_eq!(e.down_kbps, None);
        assert_eq!(e.priority, Priority::High);
        assert!(!e.cap_reached);
    }

    #[test]
    fn cap_block_and_throttle() {
        let mut d = defaults();
        let dev = DevicePolicy::new_device(&d);
        assert!(!resolve(&d, &dev, 499 * MB).cap_reached);
        let e = resolve(&d, &dev, 500 * MB);
        assert!(e.cap_reached);
        assert_eq!(e.access, Access::Blocked);

        d.limits.cap_action = CapAction::Throttle { kbps: 256 };
        let e = resolve(&d, &dev, 600 * MB);
        assert_eq!(e.access, Access::Allowed);
        assert_eq!((e.down_kbps, e.up_kbps), (Some(256), Some(256)));
    }

    #[test]
    fn pending_device_stays_pending_past_cap() {
        let mut d = defaults();
        d.require_approval = true;
        let dev = DevicePolicy::new_device(&d);
        let e = resolve(&d, &dev, 900 * MB);
        assert_eq!(e.access, Access::Pending);
        assert!(!e.cap_reached);
    }

    #[test]
    fn toml_round_trip() {
        let d = DefaultPolicy {
            require_approval: true,
            limits: Limits {
                cap_action: CapAction::Throttle { kbps: 256 },
                ..defaults().limits
            },
        };
        let s = toml::to_string(&d).unwrap();
        assert_eq!(toml::from_str::<DefaultPolicy>(&s).unwrap(), d);
    }

    proptest! {
        #[test]
        fn cap_is_monotonic(cap in 1u32..5000, a in 0u64..10_000_000_000, b in 0u64..10_000_000_000) {
            let mut d = defaults();
            d.limits.cap_mb = Some(cap);
            let dev = DevicePolicy::new_device(&d);
            let (lo, hi) = (a.min(b), a.max(b));
            // Once reached, a larger usage never "un-reaches" the cap.
            prop_assert!(!resolve(&d, &dev, lo).cap_reached || resolve(&d, &dev, hi).cap_reached);
        }
    }
}
