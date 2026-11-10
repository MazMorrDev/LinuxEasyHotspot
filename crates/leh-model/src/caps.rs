//! NetworkManager version detection and the feature set derived from it.

use std::fmt;

use crate::config::Mode;

/// A parsed NetworkManager version (`major.minor.micro`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NmVersion {
    pub major: u16,
    pub minor: u16,
    pub micro: u16,
}

impl NmVersion {
    pub const fn new(major: u16, minor: u16, micro: u16) -> Self {
        Self {
            major,
            minor,
            micro,
        }
    }

    /// Parses the `Version` property of `org.freedesktop.NetworkManager`.
    ///
    /// Distributions append suffixes (`1.46.0-1ubuntu2`, `1.47.3-dev`), so only
    /// the leading numeric components are considered. Missing components are 0.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().split('.').map(|p| {
            let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u16>().ok()
        });
        let major = parts.next()??;
        let minor = parts.next().flatten().unwrap_or(0);
        let micro = parts.next().flatten().unwrap_or(0);
        Some(Self::new(major, minor, micro))
    }
}

impl fmt::Display for NmVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.micro)
    }
}

/// Oldest NetworkManager the app supports (Debian 11 / Ubuntu 22.04 era).
pub const MIN_SUPPORTED: NmVersion = NmVersion::new(1, 30, 0);
/// First release with the `ipv4.forwarding` property used for Local-only mode.
pub const IPV4_FORWARDING_SINCE: NmVersion = NmVersion::new(1, 52, 0);

/// What the running NetworkManager can do. Profile builders consult this
/// instead of assuming a particular version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NmCapabilities {
    pub version: NmVersion,
    pub has_ipv4_forwarding: bool,
}

impl NmCapabilities {
    pub fn from_version(version: NmVersion) -> Self {
        Self {
            version,
            has_ipv4_forwarding: version >= IPV4_FORWARDING_SINCE,
        }
    }

    pub fn is_supported(&self) -> bool {
        self.version >= MIN_SUPPORTED
    }

    /// Whether `mode` can be offered. Bridge additionally needs an Ethernet
    /// device, which is a runtime check done elsewhere.
    pub fn supports_mode(&self, mode: Mode) -> bool {
        match mode {
            Mode::Router | Mode::Bridge => true,
            Mode::LocalOnly => self.has_ipv4_forwarding,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_suffixed_versions() {
        assert_eq!(NmVersion::parse("1.46.0"), Some(NmVersion::new(1, 46, 0)));
        assert_eq!(
            NmVersion::parse("1.46.0-1ubuntu2"),
            Some(NmVersion::new(1, 46, 0))
        );
        assert_eq!(
            NmVersion::parse("1.47.3-dev"),
            Some(NmVersion::new(1, 47, 3))
        );
        assert_eq!(NmVersion::parse("1.52"), Some(NmVersion::new(1, 52, 0)));
        assert_eq!(NmVersion::parse(" 2 "), Some(NmVersion::new(2, 0, 0)));
        assert_eq!(NmVersion::parse(""), None);
        assert_eq!(NmVersion::parse("abc"), None);
    }

    #[test]
    fn capability_gates() {
        let old = NmCapabilities::from_version(NmVersion::new(1, 28, 0));
        assert!(!old.is_supported());

        let mint = NmCapabilities::from_version(NmVersion::new(1, 46, 0));
        assert!(mint.is_supported());
        assert!(!mint.supports_mode(Mode::LocalOnly));
        assert!(mint.supports_mode(Mode::Bridge));

        let new = NmCapabilities::from_version(NmVersion::new(1, 52, 0));
        assert!(new.supports_mode(Mode::LocalOnly));
    }
}
