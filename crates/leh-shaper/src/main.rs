//! `leh-shaper`: privileged traffic-control helper for LinuxEasyHotspot.
//!
//! A system D-Bus service started on demand (as root). It only ever:
//! - accepts a *structured* desired state, validated and turned into `tc`
//!   commands by `leh_model::tc` (no command strings, no shell);
//! - acts on an interface that currently belongs to the active
//!   `LinuxEasyHotspot` NetworkManager connection, plus its own IFB device;
//! - after polkit authorizes the caller for `…shape` (admin, cached per session).
//!
//! It also tells the app its clients' names (see `names`): read-only, same
//! interface check, polkit `…monitor`.
//!
//! When the hotspot goes down it removes its rules by itself, and it exits
//! after a minute of idleness with nothing shaped.

mod names;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use leh_model::policy::{Access, Priority};
use leh_model::tc::{self, ShapeDevice, ShapeState};
use leh_nm::proxies::{ActiveConnectionProxy, DeviceProxy, NetworkManagerProxy};
use tokio::sync::Mutex;
use zbus::message::Header;
use zbus::zvariant::Value;
use zbus::{Connection, fdo, interface};

const BUS_NAME: &str = "io.github.OWNER.LinuxEasyHotspot.Shaper";
const OBJECT_PATH: &str = "/io/github/OWNER/LinuxEasyHotspot/Shaper";
/// Changing limits on the hotspot interface: active local user (like NM's own network-control).
const ACTION_SHAPE: &str = "io.github.OWNER.LinuxEasyHotspot.shape";
/// Reading counters / removing our own rules: active local user.
const ACTION_MONITOR: &str = "io.github.OWNER.LinuxEasyHotspot.monitor";
const API_LEVEL: u32 = 2;
const IDLE_EXIT: Duration = Duration::from_secs(60);
const WATCH_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Inner {
    /// Interface we currently shape and the MAC order used for class ids.
    shaped: Option<(String, Vec<String>)>,
    last_activity: Option<Instant>,
    /// DHCP name snooping on the hotspot's bridge (Bridge mode).
    snooper: Option<names::Snooper>,
}

struct Shaper {
    inner: Arc<Mutex<Inner>>,
}

fn find_tool(name: &str) -> Option<String> {
    ["/usr/sbin", "/sbin", "/usr/bin", "/bin"]
        .iter()
        .map(|d| format!("{d}/{name}"))
        .find(|p| std::path::Path::new(p).exists())
}

async fn run(tool: &str, args: &[String], stdin: Option<String>) -> Result<String, String> {
    use tokio::io::AsyncWriteExt as _;
    let path = find_tool(tool).ok_or_else(|| format!("{tool} not found"))?;
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

async fn authorize(
    conn: &Connection,
    hdr: &Header<'_>,
    action: &str,
    interactive: bool,
) -> fdo::Result<()> {
    let sender = hdr
        .sender()
        .ok_or_else(|| fdo::Error::AccessDenied("no sender".into()))?
        .to_string();
    let polkit = zbus::Proxy::new(
        conn,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await?;
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Value::from(sender.as_str()))]),
    );
    let details: HashMap<&str, &str> = HashMap::new();
    let flags: u32 = u32::from(interactive); // 1 = AllowUserInteraction
    let (authorized, _challenge, _details): (bool, bool, HashMap<String, String>) = polkit
        .call("CheckAuthorization", &(subject, action, details, flags, ""))
        .await
        .map_err(|e| fdo::Error::AccessDenied(format!("polkit: {e}")))?;
    if authorized {
        Ok(())
    } else {
        Err(fdo::Error::AccessDenied(
            "not authorized to manage hotspot speed limits".into(),
        ))
    }
}

/// True if `iface` belongs to the active LinuxEasyHotspot connection.
async fn is_hotspot_iface(conn: &Connection, iface: &str) -> bool {
    let check = async {
        let nm = NetworkManagerProxy::new(conn).await?;
        for path in nm.active_connections().await? {
            let ac = ActiveConnectionProxy::builder(conn)
                .path(path)?
                .build()
                .await?;
            if ac.id().await.ok().as_deref() != Some(leh_model::AP_PROFILE_ID) {
                continue;
            }
            for dev in ac.devices().await? {
                let d = DeviceProxy::builder(conn).path(dev)?.build().await?;
                if d.interface().await? == iface {
                    return Ok::<bool, zbus::Error>(true);
                }
            }
        }
        Ok(false)
    };
    check.await.unwrap_or(false)
}

async fn guard(conn: &Connection, iface: &str) -> fdo::Result<()> {
    if !tc::valid_iface(iface) {
        return Err(fdo::Error::InvalidArgs(format!(
            "invalid interface {iface:?}"
        )));
    }
    if !is_hotspot_iface(conn, iface).await {
        return Err(fdo::Error::AccessDenied(format!(
            "{iface} is not running the LinuxEasyHotspot hotspot"
        )));
    }
    Ok(())
}

async fn clear_iface(iface: &str) {
    for args in tc::teardown(iface) {
        let _ = run("tc", &args, None).await;
    }
    let _ = run(
        "ip",
        &["link".into(), "del".into(), tc::IFB_IFACE.into()],
        None,
    )
    .await;
}

fn parse_access(s: &str) -> fdo::Result<Access> {
    match s {
        "allowed" => Ok(Access::Allowed),
        "pending" => Ok(Access::Pending),
        "blocked" => Ok(Access::Blocked),
        _ => Err(fdo::Error::InvalidArgs(format!("invalid access {s:?}"))),
    }
}

fn parse_priority(s: &str) -> fdo::Result<Priority> {
    match s {
        "high" => Ok(Priority::High),
        "normal" => Ok(Priority::Normal),
        "low" => Ok(Priority::Low),
        _ => Err(fdo::Error::InvalidArgs(format!("invalid priority {s:?}"))),
    }
}

/// `tc -s class show` output → class minor → bytes sent. Accepts JSON
/// (newer iproute2) or the plain-text format (`class htb 1:10 …` followed by
/// ` Sent 123 bytes …`), since `-j` is ignored for classes on some versions.
fn class_bytes(output: &str) -> HashMap<u32, u64> {
    let mut out = HashMap::new();
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(output) else {
        let mut current: Option<u32> = None;
        for line in output.lines() {
            let mut f = line.split_whitespace();
            match (f.next(), f.next(), f.next()) {
                (Some("class"), Some(_kind), Some(handle)) => {
                    current = handle
                        .strip_prefix("1:")
                        .and_then(|m| u32::from_str_radix(m, 16).ok());
                }
                (Some("Sent"), Some(bytes), Some("bytes")) => {
                    if let (Some(minor), Ok(b)) = (current.take(), bytes.parse()) {
                        out.insert(minor, b);
                    }
                }
                _ => {}
            }
        }
        return out;
    };
    for it in items {
        let handle = it
            .get("handle")
            .and_then(|h| h.as_str())
            .unwrap_or_default();
        let Some(minor) = handle
            .strip_prefix("1:")
            .and_then(|m| u32::from_str_radix(m, 16).ok())
        else {
            continue;
        };
        if let Some(b) = it
            .pointer("/stats/bytes")
            .and_then(serde_json::Value::as_u64)
        {
            out.insert(minor, b);
        }
    }
    out
}

#[interface(name = "io.github.OWNER.LinuxEasyHotspot.Shaper1")]
impl Shaper {
    #[zbus(property)]
    fn api_level(&self) -> u32 {
        API_LEVEL
    }

    #[zbus(property)]
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    #[zbus(property)]
    fn features(&self) -> Vec<String> {
        ["htb", "ifb", "counters", "approval", "names"]
            .map(String::from)
            .to_vec()
    }

    /// Replaces all shaping on `iface`.
    /// `totals` = (download, upload) kbit/s, 0 = automatic.
    /// `devices` = (mac, access, down kbit/s, up kbit/s, priority), 0 = unlimited.
    async fn apply(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        iface: String,
        totals: (u32, u32),
        devices: Vec<(String, String, u32, u32, String)>,
    ) -> fdo::Result<()> {
        authorize(conn, &hdr, ACTION_SHAPE, true).await?;
        guard(conn, &iface).await?;
        let opt = |v: u32| (v != 0).then_some(v);
        let devices = devices
            .into_iter()
            .map(|(mac, access, down, up, prio)| {
                Ok(ShapeDevice {
                    mac: mac.to_ascii_lowercase(),
                    access: parse_access(&access)?,
                    down_kbps: opt(down),
                    up_kbps: opt(up),
                    priority: parse_priority(&prio)?,
                })
            })
            .collect::<fdo::Result<Vec<_>>>()?;
        let state = ShapeState {
            iface: iface.clone(),
            total_down_kbps: opt(totals.0),
            total_up_kbps: opt(totals.1),
            devices,
        };
        let plan = tc::plan(&state).map_err(|e| fdo::Error::InvalidArgs(e.to_string()))?;

        let mut inner = self.inner.lock().await;
        inner.last_activity = Some(Instant::now());
        if let Some((old, _)) = inner.shaped.take()
            && old != iface
        {
            clear_iface(&old).await;
        }
        clear_iface(&iface).await;
        for args in &plan.ip_setup {
            if let Err(e) = run("ip", args, None).await
                && !e.contains("File exists")
            {
                return Err(fdo::Error::Failed(format!("ip {}: {e}", args.join(" "))));
            }
        }
        if let Err(e) = run("tc", &["-batch".into(), "-".into()], Some(plan.batch)).await {
            clear_iface(&iface).await;
            return Err(fdo::Error::Failed(format!("tc: {e}")));
        }
        let macs = state.devices.iter().map(|d| d.mac.clone()).collect();
        tracing::info!(iface, devices = state.devices.len(), "shaping applied");
        inner.shaped = Some((iface, macs));
        Ok(())
    }

    async fn clear(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        iface: String,
    ) -> fdo::Result<()> {
        authorize(conn, &hdr, ACTION_MONITOR, false).await?;
        if !tc::valid_iface(&iface) {
            return Err(fdo::Error::InvalidArgs(format!(
                "invalid interface {iface:?}"
            )));
        }
        let mut inner = self.inner.lock().await;
        inner.last_activity = Some(Instant::now());
        // Clearing is always safe, but only for the interface we shaped.
        if inner.shaped.as_ref().is_some_and(|(i, _)| *i == iface) {
            clear_iface(&iface).await;
            inner.shaped = None;
        }
        Ok(())
    }

    /// Bytes (down, up) per MAC since the last `Apply`.
    async fn counters(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        iface: String,
    ) -> fdo::Result<HashMap<String, (u64, u64)>> {
        authorize(conn, &hdr, ACTION_MONITOR, false).await?;
        let mut inner = self.inner.lock().await;
        inner.last_activity = Some(Instant::now());
        let Some((shaped, macs)) = inner.shaped.clone() else {
            return Ok(HashMap::new());
        };
        if shaped != iface {
            return Ok(HashMap::new());
        }
        drop(inner);
        let show = |dev: &str| {
            ["-s", "-j", "class", "show", "dev", dev]
                .map(String::from)
                .to_vec()
        };
        let down = class_bytes(&run("tc", &show(&iface), None).await.unwrap_or_default());
        let up = class_bytes(
            &run("tc", &show(tc::IFB_IFACE), None)
                .await
                .unwrap_or_default(),
        );
        Ok(macs
            .into_iter()
            .enumerate()
            .map(|(i, mac)| {
                let minor = tc::class_minor(i);
                let d = down.get(&minor).copied().unwrap_or(0);
                let u = up.get(&minor).copied().unwrap_or(0);
                (mac, (d, u))
            })
            .collect())
    }

    /// Client names for the hotspot on `iface` (the AP interface):
    /// MAC → (hostname, IPv4), empty strings when unknown. API level 2.
    async fn client_names(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(connection)] conn: &Connection,
        iface: String,
    ) -> fdo::Result<names::Names> {
        authorize(conn, &hdr, ACTION_MONITOR, false).await?;
        guard(conn, &iface).await?;
        let mut inner = self.inner.lock().await;
        inner.last_activity = Some(Instant::now());
        let Some(bridge) = names::bridge_of(&iface) else {
            inner.snooper = None;
            drop(inner);
            return Ok(names::lease_names(&iface).await);
        };
        if inner
            .snooper
            .as_ref()
            .is_none_or(|s| s.ap_iface != iface || s.bridge != bridge)
        {
            inner.snooper = None;
            let s = names::Snooper::start(iface, bridge)
                .map_err(|e| fdo::Error::Failed(format!("listening for DHCP requests: {e}")))?;
            inner.snooper = Some(s);
        }
        Ok(inner
            .snooper
            .as_ref()
            .map(names::Snooper::names)
            .unwrap_or_default())
    }
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(|e| e == "0"))
        })
        .unwrap_or(false)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    if !is_root() {
        eprintln!("leh-shaper must run as root (it is normally started by D-Bus)");
        std::process::exit(1);
    }
    let inner = Arc::new(Mutex::new(Inner {
        last_activity: Some(Instant::now()),
        ..Inner::default()
    }));
    let conn = zbus::connection::Builder::system()?
        .name(BUS_NAME)?
        .serve_at(
            OBJECT_PATH,
            Shaper {
                inner: inner.clone(),
            },
        )?
        .build()
        .await?;
    tracing::info!("leh-shaper {} ready", env!("CARGO_PKG_VERSION"));

    // On SIGTERM (package removal, shutdown) take our rules with us.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    // Watchdog: clean up when the hotspot goes away; exit when idle.
    loop {
        tokio::select! {
            () = tokio::time::sleep(WATCH_EVERY) => {}
            _ = term.recv() => {
                let mut g = inner.lock().await;
                if let Some((iface, _)) = g.shaped.take() {
                    tracing::info!(iface, "terminating; removing rules");
                    clear_iface(&iface).await;
                }
                return Ok(());
            }
        }
        let mut g = inner.lock().await;
        if let Some((iface, _)) = g.shaped.clone()
            && !is_hotspot_iface(&conn, &iface).await
        {
            tracing::info!(iface, "hotspot gone; removing rules");
            clear_iface(&iface).await;
            g.shaped = None;
            g.last_activity = Some(Instant::now());
        }
        if let Some((ap, bridge)) = g
            .snooper
            .as_ref()
            .map(|s| (s.ap_iface.clone(), s.bridge.clone()))
            && (names::bridge_of(&ap).as_deref() != Some(bridge.as_str())
                || !is_hotspot_iface(&conn, &ap).await)
        {
            tracing::info!(ap, "hotspot gone; stopping name listener");
            g.snooper = None;
        }
        let idle = g.last_activity.is_none_or(|t| t.elapsed() > IDLE_EXIT);
        if g.shaped.is_none() && idle {
            tracing::info!("idle; exiting");
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tc_json() {
        let j = r#"[{"class":"htb","handle":"1:1","stats":{"bytes":900}},
                    {"class":"htb","handle":"1:10","stats":{"bytes":123}},
                    {"class":"htb","handle":"1:1a","stats":{"bytes":7}},
                    {"class":"fq_codel","handle":":3"}]"#;
        let m = class_bytes(j);
        assert_eq!(m.get(&0x10), Some(&123));
        assert_eq!(m.get(&0x1a), Some(&7));
        assert!(class_bytes("garbage").is_empty());
        let text = "class htb 1:9999 parent 1:1 leaf 8009: prio 1 rate 8Kbit\n Sent 55 bytes 1 pkt (dropped 0)\n\nclass htb 1:1a parent 1:1 leaf 800b: prio 1\n Sent 4096 bytes 3 pkt\n";
        let m = class_bytes(text);
        assert_eq!(m.get(&0x9999), Some(&55));
        assert_eq!(m.get(&0x1a), Some(&4096));
    }

    #[test]
    fn enum_parsing_rejects_unknown() {
        assert!(parse_access("allowed").is_ok());
        assert!(parse_access("root").is_err());
        assert!(parse_priority("urgent").is_err());
    }
}
