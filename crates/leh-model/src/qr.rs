//! Wi-Fi QR code payload (`WIFI:` URI scheme understood by Android and iOS).

/// Escapes `\ ; , : "` with a single backslash, as the scheme requires.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Builds the payload. `T:WPA` covers WPA/WPA2/WPA3-transition on both
/// Android and iOS.
pub fn payload(ssid: &str, password: &str, hidden: bool) -> String {
    let mut p = format!("WIFI:T:WPA;S:{};P:{};", escape(ssid), escape(password));
    if hidden {
        p.push_str("H:true;");
    }
    p.push(';');
    p
}

/// Parses a payload back into `(ssid, password, hidden)`. Used by tests to
/// prove escaping round-trips, the way a phone's scanner would read it.
pub fn parse(payload: &str) -> Option<(String, String, bool)> {
    let body = payload.strip_prefix("WIFI:")?.strip_suffix(';')?;
    let (mut ssid, mut pass, mut hidden) = (None, None, false);
    let mut field = String::new();
    let mut chars = body.chars();
    let mut fields = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => field.push(chars.next()?),
            ';' => fields.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    if !field.is_empty() {
        return None;
    }
    for f in fields {
        let (k, v) = f.split_once(':')?;
        match k {
            "S" => ssid = Some(v.to_owned()),
            "P" => pass = Some(v.to_owned()),
            "H" => hidden = v == "true",
            _ => {}
        }
    }
    Some((ssid?, pass.unwrap_or_default(), hidden))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn simple_payload() {
        assert_eq!(
            payload("Home", "secret123", false),
            "WIFI:T:WPA;S:Home;P:secret123;;"
        );
        assert_eq!(
            payload("Home", "secret123", true),
            "WIFI:T:WPA;S:Home;P:secret123;H:true;;"
        );
    }

    #[test]
    fn escapes_special_chars() {
        assert_eq!(
            payload(r#"a;b,c:d"e\f"#, "p;w", false),
            r#"WIFI:T:WPA;S:a\;b\,c\:d\"e\\f;P:p\;w;;"#
        );
    }

    proptest! {
        #[test]
        fn round_trips(ssid in "\\PC{1,32}", pw in "[ -~]{8,63}", hidden: bool) {
            let p = payload(&ssid, &pw, hidden);
            prop_assert_eq!(parse(&p), Some((ssid, pw, hidden)));
        }
    }
}
