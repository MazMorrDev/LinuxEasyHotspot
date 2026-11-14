//! Parts shared by the Linux and Windows backends.

use std::collections::HashMap;
use std::time::Instant;

use leh_model::clients::Client;
use leh_model::policy::Access;

use crate::state::{ClientVm, DeviceIcon, HelperStatus, UiState, now_secs};

fn guess_icon(c: &Client) -> DeviceIcon {
    let h = c
        .hostname
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| h.contains(w));
    if any(&[
        "iphone", "ipad", "pixel", "galaxy", "android", "phone", "redmi", "moto", "oneplus",
    ]) {
        DeviceIcon::Phone
    } else if any(&[
        "laptop", "book", "pc", "desktop", "thinkpad", "mac", "ubuntu", "mint",
    ]) {
        DeviceIcon::Laptop
    } else if any(&["tv", "roku", "chromecast", "firetv"]) {
        DeviceIcon::Tv
    } else {
        DeviceIcon::Unknown
    }
}

pub fn client_vm(c: &Client) -> ClientVm {
    ClientVm {
        mac: c.mac.clone(),
        hostname: c.hostname.clone(),
        ip: c.ip.map(|ip| ip.to_string()),
        vendor: c.vendor(),
        icon: guess_icon(c),
        down_kbps: 0.0,
        up_kbps: 0.0,
        session_bytes: 0,
        demand_kbps: (0.0, 0.0),
    }
}

/// Merges a fresh client list into the view-model, keeping usage stats and
/// creating policies for devices seen for the first time. Returns the names
/// of new devices that are waiting for approval.
pub fn merge_clients(st: &mut UiState, list: &[Client]) -> Vec<String> {
    let mut waiting = Vec::new();
    let mut old: HashMap<String, ClientVm> =
        st.clients.drain(..).map(|c| (c.mac.clone(), c)).collect();
    let now = now_secs();
    for c in list {
        let mut vm = client_vm(c);
        let joined = match old.remove(&vm.mac) {
            Some(prev) => {
                vm.session_bytes = prev.session_bytes;
                vm.down_kbps = prev.down_kbps;
                vm.up_kbps = prev.up_kbps;
                false
            }
            None => true,
        };
        if !st.policies.contains_key(&vm.mac) {
            let p = leh_model::policy::DevicePolicy::new_device(&st.defaults);
            if p.access == Access::Pending && st.helper == HelperStatus::Ready {
                waiting.push(st.display_name(&vm));
            }
            st.policies.insert(vm.mac.clone(), p);
        }
        // Remember the device's name and when it was here, for the list of
        // known devices (written to devices.toml only on changes).
        if let Some(p) = st.policies.get_mut(&vm.mac) {
            if vm.hostname.is_some() && p.hostname != vm.hostname {
                p.hostname = vm.hostname.clone();
            }
            if joined {
                p.last_seen = Some(now);
            }
        }
        st.clients.push(vm);
    }
    // Devices that just left.
    for mac in old.keys() {
        if let Some(p) = st.policies.get_mut(mac) {
            p.last_seen = Some(now);
        }
    }
    waiting
}

/// When the hotspot stops: remember when the connected devices left.
pub fn forget_session(st: &mut UiState) {
    st.traffic = crate::state::Traffic::default();
    let now = now_secs();
    for c in st.clients.drain(..) {
        if let Some(p) = st.policies.get_mut(&c.mac) {
            p.last_seen = Some(now);
        }
    }
}

/// Turns the hotspot interface's byte counters into session totals and
/// current rates.
#[derive(Default)]
pub struct TrafficMeter {
    /// Previous (rx, tx, when) sample, and the first one.
    last: Option<(u64, u64, Instant)>,
    base: Option<(u64, u64)>,
}

impl TrafficMeter {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `rx` is what devices sent (upload), `tx` what they received (download).
    pub fn fold(&mut self, st: &mut UiState, rx: u64, tx: u64) {
        let now = Instant::now();
        // Counters restart if the interface was re-created.
        if self.last.is_some_and(|(lrx, ltx, _)| rx < lrx || tx < ltx) {
            self.reset();
        }
        let (brx, btx) = *self.base.get_or_insert((rx, tx));
        st.traffic.up_bytes = rx - brx;
        st.traffic.down_bytes = tx - btx;
        if let Some((lrx, ltx, t)) = self.last {
            let secs = now.duration_since(t).as_secs_f64().max(0.5);
            st.traffic.up_kbps = (rx - lrx) as f64 * 8.0 / 1000.0 / secs;
            st.traffic.down_kbps = (tx - ltx) as f64 * 8.0 / 1000.0 / secs;
        }
        self.last = Some((rx, tx, now));
    }
}

/// How to get the helper, for the "Install the helper" dialog.
pub struct HelperInstall {
    /// A one-click install is possible.
    pub can_install: bool,
    /// The terminal command to show.
    pub command: String,
    /// This process can't ask for administrator rights.
    pub no_new_privs: bool,
}
