//! Human-readable messages for NetworkManager failure reasons.

/// What the user should do about a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suggestion {
    Retry,
    CheckPassword,
    ChangeChannel,
    EnableWifi,
    UseRouterMode,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub message: String,
    pub suggestion: Suggestion,
}

fn e(message: &str, suggestion: Suggestion) -> Explanation {
    Explanation {
        message: message.to_owned(),
        suggestion,
    }
}

/// `NMActiveConnectionStateReason`.
pub fn active_connection(reason: u32) -> Explanation {
    use Suggestion::*;
    match reason {
        2 => e("The hotspot was turned off.", None),
        3 => e("The Wi-Fi adapter disconnected.", Retry),
        5 => e("Couldn't set up the hotspot's network addresses.", Retry),
        6 => e("Starting the hotspot took too long.", Retry),
        9 => e("The hotspot password was rejected.", CheckPassword),
        11 => e("The hotspot settings were deleted outside the app.", Retry),
        12 => e(
            "A connection the hotspot depends on failed (for example the bridge).",
            UseRouterMode,
        ),
        13 | 14 => e("The Wi-Fi adapter disappeared.", EnableWifi),
        other => e(
            &format!("NetworkManager stopped the hotspot (reason {other})."),
            Retry,
        ),
    }
}

/// `NMDeviceStateReason`, for the access-point device.
pub fn device(reason: u32) -> Explanation {
    use Suggestion::*;
    match reason {
        4 => e(
            "The Wi-Fi adapter rejected the settings. Try another band or channel.",
            ChangeChannel,
        ),
        7 => e("The hotspot password is missing or invalid.", CheckPassword),
        8..=11 => e(
            "The Wi-Fi driver couldn't start access-point mode. Try another band or channel.",
            ChangeChannel,
        ),
        18 | 19 => e(
            "Internet sharing couldn't start (dnsmasq or firewall failed).",
            Retry,
        ),
        35 => e("The Wi-Fi adapter's firmware is missing.", None),
        36 => e("The Wi-Fi adapter was removed.", EnableWifi),
        37 => e("The computer is going to sleep.", None),
        39 => e("The hotspot was turned off.", None),
        50 => e(
            "A connection the hotspot depends on failed (for example the bridge).",
            UseRouterMode,
        ),
        60 => e("Another connection took over the Wi-Fi adapter.", Retry),
        other => e(
            &format!("The Wi-Fi adapter stopped (reason {other})."),
            Retry,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_reasons() {
        assert_eq!(device(9).suggestion, Suggestion::ChangeChannel);
        assert_eq!(active_connection(9).suggestion, Suggestion::CheckPassword);
        assert!(device(9999).message.contains("9999"));
        assert!(active_connection(9999).message.contains("9999"));
    }
}
