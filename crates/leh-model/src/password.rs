//! WPA2 passphrase generation and validation.

use rand::RngExt as _;
use thiserror::Error;

/// Alphabet without look-alike characters (0/O, 1/l/I).
const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PasswordError {
    #[error("the password must be at least 8 characters")]
    TooShort,
    #[error("the password must be at most 63 characters")]
    TooLong,
    #[error("the password can only use letters, numbers and common symbols (no accents or emoji)")]
    NotPrintableAscii,
}

/// A WPA passphrase is 8–63 printable ASCII characters (IEEE 802.11i H.4.1).
pub fn validate(pw: &str) -> Result<(), PasswordError> {
    if !pw.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(PasswordError::NotPrintableAscii);
    }
    match pw.len() {
        0..8 => Err(PasswordError::TooShort),
        8..=63 => Ok(()),
        _ => Err(PasswordError::TooLong),
    }
}

/// Generates an easy-to-read password like `k7Rp-x4Mz-q9Tw`:
/// three groups of four, about 70 bits of entropy.
pub fn generate() -> String {
    let mut rng = rand::rng();
    let mut out = String::with_capacity(14);
    for group in 0..3 {
        if group > 0 {
            out.push('-');
        }
        for _ in 0..4 {
            out.push(ALPHABET[rng.random_range(0..ALPHABET.len())] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn length_bounds() {
        assert_eq!(validate("1234567"), Err(PasswordError::TooShort));
        assert!(validate("12345678").is_ok());
        assert!(validate(&"a".repeat(63)).is_ok());
        assert_eq!(validate(&"a".repeat(64)), Err(PasswordError::TooLong));
    }

    #[test]
    fn rejects_non_ascii() {
        assert_eq!(
            validate("contraseña1"),
            Err(PasswordError::NotPrintableAscii)
        );
        assert_eq!(
            validate("password🔒"),
            Err(PasswordError::NotPrintableAscii)
        );
        assert_eq!(
            validate("pass\tword"),
            Err(PasswordError::NotPrintableAscii)
        );
        assert!(validate("p@ss w0rd;\"\\").is_ok());
    }

    #[test]
    fn generated_has_no_ambiguous_chars() {
        for _ in 0..200 {
            let pw = generate();
            assert_eq!(pw.len(), 14);
            assert!(!pw.contains(['0', 'O', '1', 'l', 'I']), "{pw}");
        }
    }

    proptest! {
        #[test]
        fn generated_always_validates(_seed in 0u8..255) {
            prop_assert!(validate(&generate()).is_ok());
        }

        #[test]
        fn validate_never_panics(s in ".*") {
            let _ = validate(&s);
        }
    }
}
