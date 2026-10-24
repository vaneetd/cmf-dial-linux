//! `dial-daemon install` / `uninstall`: per-user setup that needs no repository checkout.
//!
//! - copies the running binary to `~/.local/bin/dial-daemon` (unless it already runs from there)
//! - writes an autostart entry so Dial starts at login
//! - installs the udev rule (the only step that needs `sudo`) that lets the logged-in user read
//!   the earbuds' key device and use `/dev/uinput`

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const UDEV_RULE: &str = include_str!("../assets/70-dial.rules");
const DESKTOP_ENTRY: &str = include_str!("../assets/dial-daemon.desktop");
const UDEV_PATH: &str = "/etc/udev/rules.d/70-dial.rules";

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| "HOME is not set".to_string())
}

fn sudo(args: &[&str], input: Option<&str>) -> Result<(), String> {
    let mut child = Command::new("sudo")
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::inherit() })
        .spawn()
        .map_err(|e| format!("cannot run sudo: {e}"))?;
    if let (Some(text), Some(stdin)) = (input, child.stdin.as_mut()) {
        stdin.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    drop(child.stdin.take());
    let status = child.wait().map_err(|e| e.to_string())?;
    status.success().then_some(()).ok_or_else(|| format!("`sudo {}` failed", args.join(" ")))
}

pub fn install() -> Result<(), String> {
    let home = home()?;
    let target = home.join(".local/bin/dial-daemon");
    let current = std::env::current_exe().map_err(|e| e.to_string())?;

    if current != target {
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        // Remove first: a running binary can't be overwritten in place ("text file busy").
        let _ = std::fs::remove_file(&target);
        std::fs::copy(&current, &target).map_err(|e| format!("copy to {}: {e}", target.display()))?;
        println!("installed {}", target.display());
    }

    let autostart = home.join(".config/autostart/dial-daemon.desktop");
    std::fs::create_dir_all(autostart.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&autostart, DESKTOP_ENTRY.replace("@EXEC@", &target.display().to_string())).map_err(|e| e.to_string())?;
    println!("wrote {}", autostart.display());

    println!("installing the udev rule (needs sudo)");
    sudo(&["tee", UDEV_PATH], Some(UDEV_RULE))?;
    sudo(&["udevadm", "control", "--reload"], None)?;
    sudo(&["udevadm", "trigger", "--subsystem-match=input", "--subsystem-match=misc"], None)?;

    println!(
        "\nDone. Dial starts at your next login. To start it now:\n\n    nohup {} >/dev/null 2>&1 &\n\nClose the case lid: the dial only talks with the lid closed.",
        target.display()
    );
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let home = home()?;
    for path in [home.join(".local/bin/dial-daemon"), home.join(".config/autostart/dial-daemon.desktop")] {
        match std::fs::remove_file(&path) {
            Ok(()) => println!("removed {}", path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    sudo(&["rm", "-f", UDEV_PATH], None)?;
    sudo(&["udevadm", "control", "--reload"], None)?;
    println!("Uninstalled. A running dial-daemon keeps running until you stop it (pkill dial-daemon).");
    Ok(())
}
