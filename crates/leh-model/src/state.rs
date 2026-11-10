//! The single hotspot state machine shared by the controller and the UI.

use crate::reasons::{self, Explanation};

/// NM `NMActiveConnectionState`.
pub mod ac_state {
    pub const UNKNOWN: u32 = 0;
    pub const ACTIVATING: u32 = 1;
    pub const ACTIVATED: u32 = 2;
    pub const DEACTIVATING: u32 = 3;
    pub const DEACTIVATED: u32 = 4;
}

/// Why the hotspot is in the error state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// NetworkManager is not on the system bus.
    NmUnavailable,
    /// NM is older than the supported minimum.
    NmTooOld(String),
    /// No Wi-Fi device supports access-point mode.
    NoApAdapter,
    /// Bridge mode was chosen but no Ethernet device is available.
    BridgeNeedsEthernet,
    /// The AP device became unavailable (Wi-Fi off, unplugged).
    DeviceUnavailable,
    /// NM failed or stopped the activation.
    Activation(Explanation),
    /// A D-Bus call failed; the text is NM's error, shown verbatim.
    Call(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HotspotState {
    #[default]
    Idle,
    Activating,
    Active,
    Deactivating,
    Error(ErrorKind),
}

impl HotspotState {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Activating | Self::Active)
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Activating | Self::Deactivating)
    }
}

/// Inputs to the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    StartRequested,
    StopRequested,
    /// `ActiveConnection.StateChanged(state, reason)` for our AP profile.
    ActiveConnection {
        state: u32,
        reason: u32,
    },
    /// The AP device left a usable state; `reason` is a `NMDeviceStateReason`.
    DeviceUnavailable {
        reason: u32,
    },
    /// Found an already-active profile at startup (hotspot survived an app restart).
    AlreadyActive,
    NmVanished,
    NmAppeared,
    Failed(ErrorKind),
}

/// Pure transition function. Unknown combinations keep the current state.
pub fn transition(state: &HotspotState, event: Event) -> HotspotState {
    use HotspotState::*;
    match (state, event) {
        (_, Event::NmVanished) => Error(ErrorKind::NmUnavailable),
        (Error(ErrorKind::NmUnavailable), Event::NmAppeared) => Idle,
        (_, Event::NmAppeared) => state.clone(),
        (_, Event::Failed(kind)) => Error(kind),
        (_, Event::AlreadyActive) => Active,

        (Idle | Error(_), Event::StartRequested) => Activating,
        (Activating | Active, Event::StopRequested) => Deactivating,

        (_, Event::ActiveConnection { state: ac, reason }) => match ac {
            ac_state::ACTIVATING => Activating,
            ac_state::ACTIVATED => Active,
            ac_state::DEACTIVATING => Deactivating,
            ac_state::DEACTIVATED => match state {
                // We asked for it, or it was a plain user disconnect: back to idle.
                Deactivating | Idle => Idle,
                // Keep the more specific error (e.g. device lost) we already have.
                Error(_) => state.clone(),
                _ if reason == 2 => Idle,
                _ => Error(ErrorKind::Activation(reasons::active_connection(reason))),
            },
            _ => state.clone(),
        },

        (Activating | Active, Event::DeviceUnavailable { reason }) => {
            if reason == 36 || reason == 0 {
                Error(ErrorKind::DeviceUnavailable)
            } else {
                Error(ErrorKind::Activation(reasons::device(reason)))
            }
        }
        (_, Event::DeviceUnavailable { .. }) => state.clone(),

        (_, Event::StartRequested | Event::StopRequested) => state.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotspotState::*;

    fn ac(state: u32, reason: u32) -> Event {
        Event::ActiveConnection { state, reason }
    }

    #[test]
    fn happy_path() {
        let s = transition(&Idle, Event::StartRequested);
        assert_eq!(s, Activating);
        let s = transition(&s, ac(ac_state::ACTIVATED, 1));
        assert_eq!(s, Active);
        let s = transition(&s, Event::StopRequested);
        assert_eq!(s, Deactivating);
        let s = transition(&s, ac(ac_state::DEACTIVATED, 2));
        assert_eq!(s, Idle);
    }

    #[test]
    fn unexpected_deactivation_is_an_error() {
        let s = transition(&Activating, ac(ac_state::DEACTIVATED, 6));
        assert!(matches!(s, Error(ErrorKind::Activation(_))));
    }

    #[test]
    fn external_user_stop_returns_to_idle() {
        // Stopped from the desktop network menu.
        assert_eq!(transition(&Active, ac(ac_state::DEACTIVATED, 2)), Idle);
    }

    #[test]
    fn device_loss_and_nm_loss() {
        assert_eq!(
            transition(&Active, Event::DeviceUnavailable { reason: 36 }),
            Error(ErrorKind::DeviceUnavailable)
        );
        assert_eq!(
            transition(&Idle, Event::DeviceUnavailable { reason: 36 }),
            Idle
        );
        let lost = Error(ErrorKind::DeviceUnavailable);
        assert_eq!(transition(&lost, ac(ac_state::DEACTIVATED, 3)), lost);
        let s = transition(&Active, Event::NmVanished);
        assert_eq!(s, Error(ErrorKind::NmUnavailable));
        assert_eq!(transition(&s, Event::NmAppeared), Idle);
        assert_eq!(transition(&Active, Event::NmAppeared), Active);
    }

    #[test]
    fn double_start_is_ignored_and_retry_from_error_works() {
        assert_eq!(transition(&Activating, Event::StartRequested), Activating);
        assert_eq!(
            transition(&Error(ErrorKind::DeviceUnavailable), Event::StartRequested),
            Activating
        );
        assert_eq!(transition(&Idle, Event::StopRequested), Idle);
    }
}
