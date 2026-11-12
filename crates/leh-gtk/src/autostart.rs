//! "Launch at login": an XDG autostart entry in ~/.config/autostart.

use std::path::PathBuf;

fn entry_path() -> PathBuf {
    crate::config_store::config_dir()
        .parent()
        .map(|p| p.join("autostart"))
        .unwrap_or_else(|| PathBuf::from("autostart"))
        .join(format!("{}.desktop", crate::APP_ID))
}

/// The command the desktop runs at login.
fn exec() -> String {
    if crate::live::packaged() {
        return "linuxeasyhotspot --hidden".into();
    }
    // Running from a source tree: point at this exact binary.
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "linuxeasyhotspot".into());
    format!(
        "\"{}\" --hidden",
        exe.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn entry() -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Linux Easy Hotspot\n\
         Comment=Wi-Fi hotspot (starts in the background)\n\
         Exec={}\n\
         Icon={}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         X-GNOME-Autostart-Delay=3\n",
        exec(),
        crate::APP_ID
    )
}

/// Creates or removes the autostart entry. Rewrites it when the command
/// changed (e.g. the app moved from a source build to the .deb).
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
