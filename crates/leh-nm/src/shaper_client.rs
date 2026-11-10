//! Client for the `leh-shaper` helper (system bus).

use std::collections::HashMap;

use leh_model::policy::{Access, Priority};
use leh_model::tc::ShapeDevice;
use zbus::{Connection, proxy};

pub const API_LEVEL_NEEDED: u32 = 1;

#[proxy(
    interface = "io.github.OWNER.LinuxEasyHotspot.Shaper1",
    default_service = "io.github.OWNER.LinuxEasyHotspot.Shaper",
    default_path = "/io/github/OWNER/LinuxEasyHotspot/Shaper"
)]
trait Shaper {
    fn apply(
        &self,
        iface: &str,
        totals: (u32, u32),
        devices: Vec<(String, String, u32, u32, String)>,
    ) -> zbus::Result<()>;

    fn clear(&self, iface: &str) -> zbus::Result<()>;

    fn counters(&self, iface: &str) -> zbus::Result<HashMap<String, (u64, u64)>>;

    #[zbus(property)]
    fn api_level(&self) -> zbus::Result<u32>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperProbe {
    Missing,
    TooOld,
    Ready { api_level: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaperError {
    /// polkit said no (user cancelled the password dialog, or not allowed).
    Denied,
    Missing,
    Other(String),
}

impl std::fmt::Display for ShaperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied => f.write_str("not authorized"),
            Self::Missing => f.write_str("helper not installed"),
            Self::Other(e) => f.write_str(e),
        }
    }
}

fn map_err(e: zbus::Error) -> ShaperError {
    match &e {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.AccessDenied" => ShaperError::Denied,
            "org.freedesktop.DBus.Error.ServiceUnknown" => ShaperError::Missing,
            _ => ShaperError::Other(e.to_string()),
        },
        zbus::Error::FDO(f) => match **f {
            zbus::fdo::Error::AccessDenied(_) => ShaperError::Denied,
            zbus::fdo::Error::ServiceUnknown(_) => ShaperError::Missing,
            _ => ShaperError::Other(e.to_string()),
        },
        _ => ShaperError::Other(e.to_string()),
    }
}

/// Is the helper installed (activatable) and new enough?
pub async fn probe(conn: &Connection) -> HelperProbe {
    let Ok(p) = ShaperProxy::new(conn).await else {
        return HelperProbe::Missing;
    };
    match p.api_level().await {
        Ok(level) if level >= API_LEVEL_NEEDED => HelperProbe::Ready { api_level: level },
        Ok(_) => HelperProbe::TooOld,
        Err(_) => HelperProbe::Missing,
    }
}

fn access_str(a: Access) -> &'static str {
    match a {
        Access::Allowed => "allowed",
        Access::Pending => "pending",
        Access::Blocked => "blocked",
    }
}

fn priority_str(p: Priority) -> &'static str {
    match p {
        Priority::High => "high",
        Priority::Normal => "normal",
        Priority::Low => "low",
    }
}

pub async fn apply(
    conn: &Connection,
    iface: &str,
    total_down_kbps: Option<u32>,
    total_up_kbps: Option<u32>,
    devices: &[ShapeDevice],
) -> Result<(), ShaperError> {
    let p = ShaperProxy::new(conn).await.map_err(map_err)?;
    let devs = devices
        .iter()
        .map(|d| {
            (
                d.mac.clone(),
                access_str(d.access).to_owned(),
                d.down_kbps.unwrap_or(0),
                d.up_kbps.unwrap_or(0),
                priority_str(d.priority).to_owned(),
            )
        })
        .collect();
    p.apply(
        iface,
        (total_down_kbps.unwrap_or(0), total_up_kbps.unwrap_or(0)),
        devs,
    )
    .await
    .map_err(map_err)
}

pub async fn clear(conn: &Connection, iface: &str) -> Result<(), ShaperError> {
    let p = ShaperProxy::new(conn).await.map_err(map_err)?;
    p.clear(iface).await.map_err(map_err)
}

pub async fn counters(
    conn: &Connection,
    iface: &str,
) -> Result<HashMap<String, (u64, u64)>, ShaperError> {
    let p = ShaperProxy::new(conn).await.map_err(map_err)?;
    p.counters(iface).await.map_err(map_err)
}
