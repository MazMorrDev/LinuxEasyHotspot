//! "Launch at login": runs this binary with `--hidden` when the user logs in.
//!
//! - Linux: an XDG autostart entry in `~/.config/autostart`.
//! - Windows: a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

/// The command line to run at login.
fn command() -> String {
    #[cfg(not(windows))]
    if crate::packaged() {
        return "easyhotspot --hidden".into();
    }
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "easyhotspot".into());
    format!(
        "\"{}\" --hidden",
        exe.replace('\\', if cfg!(windows) { "\\" } else { "\\\\" })
            .replace('"', "\\\"")
    )
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    fn entry_path() -> PathBuf {
        crate::config_store::config_dir()
            .parent()
            .map(|p| p.join("autostart"))
            .unwrap_or_else(|| PathBuf::from("autostart"))
            .join(format!("{}.desktop", crate::APP_ID))
    }

    fn entry() -> String {
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name={}\n\
             Comment=Wi-Fi hotspot (starts in the background)\n\
             Exec={}\n\
             Icon={}\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n\
             X-GNOME-Autostart-Delay=3\n",
            leh_model::APP_NAME,
            super::command(),
            crate::APP_ID
        )
    }

    pub fn set(enabled: bool) {
        let path = entry_path();
        if !enabled {
            if let Err(e) = std::fs::remove_file(&path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("autostart: can't remove {}: {e}", path.display());
            }
            return;
        }
        let want = entry();
        if std::fs::read_to_string(&path).is_ok_and(|have| have == want) {
            return;
        }
        let res = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, want));
        if let Err(e) = res {
            eprintln!("autostart: can't write {}: {e}", path.display());
        }
    }
}

#[cfg(windows)]
mod imp {
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn set(enabled: bool) {
        let key = match windows_registry::CURRENT_USER.create(RUN_KEY) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("autostart: can't open the Run key: {e}");
                return;
            }
        };
        let name = leh_model::APP_NAME;
        if !enabled {
            // Missing already is fine.
            let _ = key.remove_value(name);
            return;
        }
        let want = super::command();
        if key.get_string(name).is_ok_and(|have| have == want) {
            return;
        }
        if let Err(e) = key.set_string(name, &want) {
            eprintln!("autostart: can't write the Run key: {e}");
        }
    }
}

/// Creates or removes the login entry. Rewrites it when the command changed
/// (e.g. the app moved).
pub fn set(enabled: bool) {
    imp::set(enabled);
}
