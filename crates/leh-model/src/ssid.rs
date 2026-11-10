//! SSID generation and validation.

use thiserror::Error;

/// 802.11 limits an SSID to 32 bytes.
pub const MAX_SSID_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SsidError {
    #[error("the network name can't be empty")]
    Empty,
    #[error("the network name is too long ({0} bytes, max 32)")]
    TooLong(usize),
    #[error("the network name can't contain control characters")]
    ControlChars,
}

pub fn validate(ssid: &str) -> Result<(), SsidError> {
    if ssid.trim().is_empty() {
        return Err(SsidError::Empty);
    }
    if ssid.len() > MAX_SSID_BYTES {
        return Err(SsidError::TooLong(ssid.len()));
    }
    if ssid.chars().any(char::is_control) {
        return Err(SsidError::ControlChars);
    }
    Ok(())
}

/// Turns a hostname into a friendly, always-valid SSID: keeps
/// `[A-Za-z0-9_-]`, maps spaces and dots to `-`, drops everything else, and
/// truncates to 32 bytes. Falls back to `LinuxEasyHotspot`.
pub fn from_hostname(hostname: &str) -> String {
    let host = hostname.split('.').next().unwrap_or_default();
    let mut out = String::with_capacity(host.len());
    for c in host.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' => out.push(c),
            ' ' => out.push('-'),
            _ => {}
        }
    }
    let trimmed = out.trim_matches('-');
    let mut s: String = trimmed.chars().take(MAX_SSID_BYTES).collect();
    if s.is_empty() || s.eq_ignore_ascii_case("localhost") {
        s = "LinuxEasyHotspot".into();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_sanitization() {
        assert_eq!(from_hostname("mazmorr-laptop"), "mazmorr-laptop");
        assert_eq!(from_hostname("box.local"), "box");
        assert_eq!(from_hostname("My PC!"), "My-PC");
        assert_eq!(from_hostname("señor"), "seor");
        assert_eq!(from_hostname(""), "LinuxEasyHotspot");
        assert_eq!(from_hostname("localhost"), "LinuxEasyHotspot");
        assert_eq!(from_hostname(&"a".repeat(50)).len(), 32);
        for h in ["???", "--", "日本"] {
            assert!(validate(&from_hostname(h)).is_ok());
        }
    }

    #[test]
    fn validation() {
        assert_eq!(validate(""), Err(SsidError::Empty));
        assert_eq!(validate("   "), Err(SsidError::Empty));
        assert_eq!(validate(&"x".repeat(33)), Err(SsidError::TooLong(33)));
        assert_eq!(validate("a\nb"), Err(SsidError::ControlChars));
        assert!(validate("Café ☕").is_ok());
    }
}
