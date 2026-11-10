//! Manual test driver for the controller.
//!
//! ```sh
//! cargo run -p leh-nm --example cli -- status
//! cargo run -p leh-nm --example cli -- start [ssid] [password] [router|bridge:<eth>|local]
//! cargo run -p leh-nm --example cli -- clients
//! cargo run -p leh-nm --example cli -- stop
//! ```
//! `start` keeps running and prints state changes and clients until Ctrl+C,
//! then stops the hotspot.

use std::time::Duration;

use futures_util::StreamExt;
use leh_model::{HotspotConfig, HotspotState, Mode};
use leh_nm::{Controller, Snapshot, clients};

async fn settled(c: &Controller) -> Snapshot {
    let mut rx = c.subscribe();
    // Give init a moment; the first snapshot is the empty default.
    let _ = tokio::time::timeout(Duration::from_secs(3), rx.wait_for(|s| s.caps.is_some())).await;
    rx.borrow().clone()
}

fn print_status(s: &Snapshot) {
    println!("state:   {:?}", s.state);
    if let Some(caps) = &s.caps {
        println!(
            "nm:      {} (local-only: {})",
            caps.version, caps.has_ipv4_forwarding
        );
    }
    for d in &s.devices {
        println!(
            "device:  {:<12} {:?} state={} ap={} 5ghz={}",
            d.iface,
            d.kind,
            d.state,
            d.is_ap_capable(),
            d.supports_5ghz()
        );
    }
    if let Some(r) = &s.running {
        println!("running: {r:?}");
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let c = Controller::spawn();
    let snap = settled(&c).await;

    match args.first().map(String::as_str) {
        Some("start") => {
            let mut cfg = HotspotConfig::first_run("leh-cli-test");
            if let Some(s) = args.get(1) {
                cfg.ssid = s.clone();
            }
            if let Some(p) = args.get(2) {
                cfg.password = p.clone();
            }
            match args.get(3).map(String::as_str) {
                Some("local") => cfg.mode = Mode::LocalOnly,
                Some(b) if b.starts_with("bridge:") => {
                    cfg.mode = Mode::Bridge;
                    cfg.upstream_iface = Some(b["bridge:".len()..].to_owned());
                }
                _ => {}
            }
            println!("starting {:?} (password {})", cfg.ssid, cfg.password);
            c.start(cfg);
            let mut rx = c.subscribe();
            let mut clients_task = None;
            let mut started = false;
            loop {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => break,
                    r = rx.changed() => {
                        if r.is_err() { break }
                        let s = rx.borrow_and_update().clone();
                        println!("-> {:?}", s.state);
                        started |= s.state.is_running() || matches!(s.state, HotspotState::Error(_));
                        if s.state == HotspotState::Active && clients_task.is_none() && let Some(run) = &s.running {
                            println!("   {run:?}");
                            let scope = run.client_scope();
                            clients_task = Some(tokio::spawn(async move {
                                let mut w = clients::watch(scope, Duration::from_secs(3));
                                while let Some(list) = w.next().await {
                                    println!("   clients: {list:?}");
                                }
                            }));
                        }
                        if started && matches!(s.state, HotspotState::Error(_) | HotspotState::Idle) { break }
                    }
                }
            }
            if c.snapshot().state.is_running() {
                c.stop();
                let mut rx = c.subscribe();
                let _ = tokio::time::timeout(
                    Duration::from_secs(10),
                    rx.wait_for(|s| !s.state.is_running() && !s.state.is_busy()),
                )
                .await;
                println!("-> {:?}", c.snapshot().state);
            }
        }
        Some("stop") => {
            c.stop();
            let mut rx = c.subscribe();
            let _ = tokio::time::timeout(
                Duration::from_secs(10),
                rx.wait_for(|s| !s.state.is_running() && !s.state.is_busy()),
            )
            .await;
            print_status(&c.snapshot());
        }
        Some("clients") => match &snap.running {
            Some(run) => println!("{:#?}", clients::list(&run.client_scope()).await),
            None => println!("hotspot is not running"),
        },
        _ => print_status(&snap),
    }
}
