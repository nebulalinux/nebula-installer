use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::model::InstallerEvent;
use crate::monitors::render_hypr_monitors_conf;

use super::commands::{run_chroot, run_command, run_command_capture};
use super::send_event;

const WLR_RANDR_CACHE_PATH: &str = "/tmp/nebula-wlr-randr.txt";

// Detects the CPU
pub(crate) fn detect_microcode_package() -> Result<Option<&'static str>> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").context("read cpuinfo")?;
    for line in cpuinfo.lines() {
        if let Some(rest) = line.strip_prefix("vendor_id") {
            let vendor = rest.split(':').nth(1).map(|s| s.trim());
            return Ok(match vendor {
                Some("GenuineIntel") => Some("intel-ucode"),
                Some("AuthenticAMD") => Some("amd-ucode"),
                _ => None,
            });
        }
    }
    Ok(None)
}

// Writes the zram configuration file
pub(crate) fn configure_zram() -> Result<()> {
    let contents = "[zram0]\nzram-size = ram\n";
    fs::create_dir_all("/mnt/etc/systemd").context("create systemd dir")?;
    fs::write("/mnt/etc/systemd/zram-generator.conf", contents).context("write zram config")?;
    Ok(())
}

// Gets the UUID of a block device
pub(crate) fn get_uuid(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    device: &str,
) -> Result<String> {
    let output = run_command_capture(tx, "blkid", &["-s", "UUID", "-o", "value", device])?;
    Ok(output.trim().to_string())
}

// Installs Hyprland user config from nebula-hypr
pub(crate) fn install_nebula_hypr(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    username: &str,
) -> Result<()> {
    let sources = [
        "/mnt/usr/share/nebula-hypr/run.sh",
        "/usr/share/nebula-hypr/run.sh",
        "/run/archiso/bootmnt/airootfs/usr/share/nebula-hypr/run.sh",
        "/run/archiso/bootmnt/usr/share/nebula-hypr/run.sh",
    ];
    let mut found = None;
    for source in &sources {
        if Path::new(source).exists() {
            found = Some(*source);
            break;
        }
    }

    let script = if let Some(source) = found {
        source
    } else {
        send_event(
            tx,
            InstallerEvent::Log(
                "nebula-hypr installer script not found; skipping Hyprland config install."
                    .to_string(),
            ),
        );
        return Ok(());
    };

    send_event(
        tx,
        InstallerEvent::Log(format!("Installing Nebula Hyprland defaults from {}...", script)),
    );
    run_command(tx, "bash", &[script, "/mnt", username], None)?;
    Ok(())
}

// Installs Hyprland user config from caelestia-meta
pub(crate) fn install_caelestia(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    username: &str,
    selected_browsers: &[String],
    selected_editors: &[String],
) -> Result<()> {
    let sources = [
        "/mnt/usr/share/caelestia/run.sh",
        "/usr/share/caelestia/run.sh",
        "/run/archiso/bootmnt/airootfs/usr/share/caelestia/run.sh",
        "/run/archiso/bootmnt/usr/share/caelestia/run.sh",
    ];
    let mut found = None;
    for source in &sources {
        if Path::new(source).exists() {
            found = Some(*source);
            break;
        }
    }

    let script = if let Some(source) = found {
        source
    } else {
        send_event(
            tx,
            InstallerEvent::Log(
                "caelestia-meta installer script not found; skipping Caelestia config install."
                    .to_string(),
            ),
        );
        return Ok(());
    };

    send_event(
        tx,
        InstallerEvent::Log(format!("Installing Caelestia defaults from {}...", script)),
    );
    run_command(tx, "bash", &[script, "/mnt", username], None)?;

    let hypr_main = format!("/mnt/home/{}/.config/hypr/hyprland.conf", username);
    let monitors_source = "source = ~/.config/hypr/monitors.conf";
    if Path::new(&hypr_main).exists() {
        let existing = fs::read_to_string(&hypr_main).unwrap_or_default();
        if !existing.lines().any(|line| line.trim() == monitors_source) {
            let mut updated = existing;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str("# Nebula monitor config\n");
            updated.push_str(monitors_source);
            updated.push('\n');
            fs::write(&hypr_main, updated).context("append Hyprland monitor include")?;
        }
    }

    install_caelestia_optional_configs(username, selected_browsers, selected_editors)?;
    Ok(())
}

fn install_caelestia_optional_configs(
    username: &str,
    selected_browsers: &[String],
    selected_editors: &[String],
) -> Result<()> {
    let optional_root = Path::new("/mnt/usr/share/caelestia/optional");
    if !optional_root.exists() {
        return Ok(());
    }

    let home_dir = format!("/mnt/home/{}", username);
    let config_dir = format!("{}/.config", home_dir);
    let data_dir = format!("{}/.local/share/nebula/caelestia/optional", home_dir);

    if selected_editors
        .iter()
        .any(|label| label == "Visual Studio Code")
    {
        let vscode_src = optional_root.join("vscode");
        let vscode_user = format!("{}/Code/User", config_dir);
        fs::create_dir_all(&vscode_user).context("create vscode user config dir")?;
        let _ = fs::copy(
            vscode_src.join("settings.json"),
            format!("{}/settings.json", vscode_user),
        );
        let _ = fs::copy(
            vscode_src.join("keybindings.json"),
            format!("{}/keybindings.json", vscode_user),
        );
        let _ = fs::copy(
            vscode_src.join("flags.conf"),
            format!("{}/code-flags.conf", config_dir),
        );
    }

    if selected_editors.iter().any(|label| label == "VSCodium") {
        let vscode_src = optional_root.join("vscode");
        let vscodium_user = format!("{}/VSCodium/User", config_dir);
        fs::create_dir_all(&vscodium_user).context("create vscodium user config dir")?;
        let _ = fs::copy(
            vscode_src.join("settings.json"),
            format!("{}/settings.json", vscodium_user),
        );
        let _ = fs::copy(
            vscode_src.join("keybindings.json"),
            format!("{}/keybindings.json", vscodium_user),
        );
        let _ = fs::copy(
            vscode_src.join("flags.conf"),
            format!("{}/codium-flags.conf", config_dir),
        );
    }

    if selected_browsers.iter().any(|label| label == "Zen Browser") {
        let zen_src = optional_root.join("zen");
        let staged = Path::new(&data_dir).join("zen");
        fs::create_dir_all(&staged).context("create zen staged dir")?;
        let _ = fs::copy(
            zen_src.join("userChrome.css"),
            staged.join("userChrome.css"),
        );
        let _ = fs::copy(
            zen_src.join("native_app/manifest.json"),
            staged.join("manifest.json"),
        );
        let _ = fs::copy(zen_src.join("native_app/app.fish"), staged.join("app.fish"));
    }

    Ok(())
}

// Schedules a GNOME dark theme application on first login via autostart and Hyprland exec-once
pub(crate) fn schedule_nebula_theme(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    username: &str,
) -> Result<()> {
    let home_dir = format!("/mnt/home/{}", username);
    let autostart_dir = format!("{}/.config/autostart", home_dir);
    let autostart_file = format!("{}/nebula-theme.desktop", autostart_dir);
    let script_dir = format!("{}/.local/share/nebula/post-install", home_dir);
    let script_path = format!("{}/run-gnome-theme.sh", script_dir);
    let hypr_dir = format!("{}/.local/share/nebula/hypr", home_dir);
    let hypr_include = format!("{}/nebula-theme.conf", hypr_dir);
    let hypr_include_home = "~/.local/share/nebula/hypr/nebula-theme.conf";
    let hypr_main = format!("{}/.config/hypr/hyprland.conf", home_dir);
    let hypr_source_line = format!("source = {}", hypr_include_home);
    let hypr_exec_line =
        "exec-once = /bin/bash -lc \"$HOME/.local/share/nebula/post-install/run-gnome-theme.sh\"";

    fs::create_dir_all(&autostart_dir).context("create autostart dir")?;
    fs::create_dir_all(&script_dir).context("create theme script dir")?;
    fs::create_dir_all(&hypr_dir).context("create hypr config dir")?;

    let autostart_contents = concat!(
        "[Desktop Entry]\n",
        "Type=Application\n",
        "Name=Nebula Theme Setup\n",
        "Comment=Apply GNOME dark theme on first login\n",
        "Exec=/bin/bash -lc \"$HOME/.local/share/nebula/post-install/run-gnome-theme.sh\"\n",
        "Terminal=false\n",
        "OnlyShowIn=GNOME;\n",
        "X-GNOME-Autostart-enabled=true\n",
    );
    fs::write(&autostart_file, autostart_contents).context("write theme autostart")?;

    let script_contents = concat!(
        "#!/usr/bin/env bash\n",
        "set -euo pipefail\n",
        "theme_marker=\"$HOME/.cache/nebula-theme-applied\"\n",
        "if [[ -f \"$theme_marker\" ]]; then\n",
        "  exit 0\n",
        "fi\n",
        "mkdir -p \"$HOME/.config/dconf\"\n",
        "if command -v gsettings >/dev/null 2>&1; then\n",
        "  gsettings set org.gnome.desktop.interface color-scheme 'prefer-dark' || true\n",
        "  gsettings set org.gnome.desktop.interface gtk-theme 'Adwaita-dark' || true\n",
        "fi\n",
        "mkdir -p \"$(dirname \"$theme_marker\")\"\n",
        "touch \"$theme_marker\"\n",
        "autostart_file=\"$HOME/.config/autostart/nebula-theme.desktop\"\n",
        "if [[ -f \"$autostart_file\" ]]; then\n",
        "  rm -f \"$autostart_file\"\n",
        "fi\n",
    );
    fs::write(&script_path, script_contents).context("write theme script")?;
    run_command(tx, "chmod", &["+x", &script_path], None)?;

    let hypr_include_contents = format!("# Nebula post-install hooks\n{}\n", hypr_exec_line);
    fs::write(&hypr_include, hypr_include_contents).context("write hypr theme include")?;
    if Path::new(&hypr_main).exists() {
        let existing = fs::read_to_string(&hypr_main).unwrap_or_default();
        let mut updated =
            existing.replace(&format!("source = {}", hypr_include), hypr_include_home);
        updated = updated
            .lines()
            .filter(|line| !line.trim_start().starts_with("source = /mnt/home/"))
            .collect::<Vec<_>>()
            .join("\n");
        if !updated.lines().any(|line| line.trim() == hypr_source_line) {
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str("# Nebula post-install hooks\n");
            updated.push_str(&hypr_source_line);
            updated.push('\n');
        }
        if updated != existing {
            fs::write(&hypr_main, updated).context("append hypr theme include")?;
        }
    } else {
        send_event(
            tx,
            InstallerEvent::Log("Hyprland defaults not found; skipping theme hook.".to_string()),
        );
    }

    let chown_user = format!("{}:{}", username, username);
    let chown_autostart = format!("/home/{}/.config/autostart", username);
    let chown_script_dir = format!("/home/{}/.local/share/nebula/post-install", username);
    let chown_hypr_include = format!(
        "/home/{}/.local/share/nebula/hypr/nebula-theme.conf",
        username
    );
    run_chroot(
        tx,
        &[
            "chown",
            "-R",
            &chown_user,
            &chown_autostart,
            &chown_script_dir,
            &chown_hypr_include,
        ],
        None,
    )?;
    Ok(())
}

// Schedules a one-time Caelestia init on first Hyprland login
pub(crate) fn schedule_caelestia_init(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    username: &str,
) -> Result<()> {
    let home_dir = format!("/mnt/home/{}", username);
    let autostart_dir = format!("{}/.config/autostart", home_dir);
    let autostart_file = format!("{}/caelestia-init.desktop", autostart_dir);
    let script_dir = format!("{}/.local/share/nebula/post-install", home_dir);
    let script_path = format!("{}/run-caelestia-init.sh", script_dir);
    let hypr_dir = format!("{}/.local/share/nebula/hypr", home_dir);
    let hypr_include = format!("{}/caelestia-init.conf", hypr_dir);
    let hypr_include_home = "~/.local/share/nebula/hypr/caelestia-init.conf";
    let hypr_main = format!("{}/.config/hypr/hyprland.conf", home_dir);
    let hypr_source_line = format!("source = {}", hypr_include_home);
    let hypr_exec_line = "exec-once = /bin/bash -lc \"$HOME/.local/share/nebula/post-install/run-caelestia-init.sh\"";

    fs::create_dir_all(&autostart_dir).context("create autostart dir")?;
    fs::create_dir_all(&script_dir).context("create caelestia init script dir")?;
    fs::create_dir_all(&hypr_dir).context("create hypr init dir")?;

    let autostart_contents = concat!(
        "[Desktop Entry]\n",
        "Type=Application\n",
        "Name=Caelestia Init\n",
        "Comment=Apply Caelestia scheme on first Hyprland login\n",
        "Exec=/bin/bash -lc \"$HOME/.local/share/nebula/post-install/run-caelestia-init.sh\"\n",
        "Terminal=false\n",
        "OnlyShowIn=Hyprland;\n",
        "X-GNOME-Autostart-enabled=true\n",
    );
    fs::write(&autostart_file, autostart_contents).context("write caelestia init autostart")?;

    let sources = [
        "/mnt/usr/share/caelestia/caelestia-init.sh",
        "/usr/share/caelestia/caelestia-init.sh",
        "/run/archiso/bootmnt/airootfs/usr/share/caelestia/caelestia-init.sh",
        "/run/archiso/bootmnt/usr/share/caelestia/caelestia-init.sh",
    ];
    let mut found = None;
    for source in &sources {
        if Path::new(source).exists() {
            found = Some(*source);
            break;
        }
    }
    let script_source = if let Some(source) = found {
        source
    } else {
        send_event(
            tx,
            InstallerEvent::Log(
                "Caelestia init script not found; skipping init setup.".to_string(),
            ),
        );
        return Ok(());
    };
    fs::copy(script_source, &script_path).context("copy caelestia init script")?;
    run_command(tx, "chmod", &["+x", &script_path], None)?;

    let hypr_include_contents = format!("# Nebula Caelestia init\n{}\n", hypr_exec_line);
    fs::write(&hypr_include, hypr_include_contents).context("write hypr init include")?;
    if Path::new(&hypr_main).exists() {
        let existing = fs::read_to_string(&hypr_main).unwrap_or_default();
        if !existing.lines().any(|line| line.trim() == hypr_source_line) {
            let mut updated = existing;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str("# Nebula Caelestia init\n");
            updated.push_str(&hypr_source_line);
            updated.push('\n');
            fs::write(&hypr_main, updated).context("append hypr init include")?;
        }
    }

    let chown_user = format!("{}:{}", username, username);
    let chown_autostart = format!("/home/{}/.config/autostart", username);
    let chown_script_dir = format!("/home/{}/.local/share/nebula/post-install", username);
    let chown_hypr_include = format!("/home/{}/.local/share/nebula/hypr", username);
    run_chroot(
        tx,
        &[
            "chown",
            "-R",
            &chown_user,
            &chown_autostart,
            &chown_script_dir,
            &chown_hypr_include,
        ],
        None,
    )?;

    Ok(())
}

pub(crate) fn configure_hypr_monitors(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    username: &str,
) -> Result<()> {
    send_event(
        tx,
        InstallerEvent::Log("Generating Hyprland monitor config...".to_string()),
    );
    let output = match get_wlr_randr_output(tx) {
        Some(output) => output,
        None => {
            send_event(
                tx,
                InstallerEvent::Log(
                    "Failed to read wlr-randr output; skipping monitor config.".to_string(),
                ),
            );
            return Ok(());
        }
    };
    let contents = match render_hypr_monitors_conf(&output)? {
        Some(contents) => contents,
        None => {
            send_event(
                tx,
                InstallerEvent::Log("No monitor data found; skipping monitor config.".to_string()),
            );
            return Ok(());
        }
    };

    let config_path = format!("/mnt/home/{}/.config/hypr/monitors.conf", username);
    send_event(
        tx,
        InstallerEvent::Log(format!(
            "Writing Hyprland monitor config to {}",
            config_path
        )),
    );
    let config_parent = Path::new(&config_path)
        .parent()
        .context("monitor config parent")?;
    fs::create_dir_all(config_parent).context("create hypr config dir")?;
    fs::write(&config_path, contents).context("write hypr monitors config")?;
    Ok(())
}

pub(crate) fn get_wlr_randr_output(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
) -> Option<String> {
    if let Ok(contents) = fs::read_to_string(WLR_RANDR_CACHE_PATH) {
        if !contents.trim().is_empty() {
            send_event(
                tx,
                InstallerEvent::Log(format!(
                    "Using cached wlr-randr output from {}",
                    WLR_RANDR_CACHE_PATH
                )),
            );
            return Some(contents);
        }
    }

    match run_wlr_randr(tx) {
        Ok(output) => {
            if let Err(err) = fs::write(WLR_RANDR_CACHE_PATH, &output) {
                send_event(
                    tx,
                    InstallerEvent::Log(format!(
                        "Failed to cache wlr-randr output to {}: {}",
                        WLR_RANDR_CACHE_PATH, err
                    )),
                );
            }
            Some(output)
        }
        Err(err) => {
            send_event(
                tx,
                InstallerEvent::Log(format!(
                    "Failed to run wlr-randr; skipping monitor detection ({})",
                    err
                )),
            );
            None
        }
    }
}

pub(crate) fn run_wlr_randr(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<String> {
    let mut cmd = Command::new("wlr-randr");
    if let Some((runtime_dir, display)) = find_wayland_socket() {
        send_event(
            tx,
            InstallerEvent::Log(format!(
                "Using Wayland socket: XDG_RUNTIME_DIR={} WAYLAND_DISPLAY={}",
                runtime_dir, display
            )),
        );
        cmd.env("XDG_RUNTIME_DIR", runtime_dir)
            .env("WAYLAND_DISPLAY", display);
    } else {
        send_event(
            tx,
            InstallerEvent::Log("No Wayland socket found; using default environment.".to_string()),
        );
    }

    let output = cmd.output().context("run wlr-randr")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Command failed: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let preview: String = stdout
        .lines()
        .take(12)
        .map(|line| line.to_string())
        .collect::<Vec<String>>()
        .join("\\n");
    send_event(
        tx,
        InstallerEvent::Log(format!(
            "wlr-randr output size: {} bytes\\n{}",
            stdout.len(),
            preview
        )),
    );
    Ok(stdout)
}

pub(crate) fn find_wayland_socket() -> Option<(String, String)> {
    let env_runtime = std::env::var("XDG_RUNTIME_DIR").ok();
    let env_display = std::env::var("WAYLAND_DISPLAY").ok();
    if let (Some(runtime_dir), Some(display)) = (env_runtime.clone(), env_display.clone()) {
        if Path::new(&format!("{}/{}", runtime_dir, display)).exists() {
            return Some((runtime_dir, display));
        }
    }

    let run_user = Path::new("/run/user");
    let mut best: Option<(std::time::SystemTime, String, String)> = None;
    if let Ok(entries) = fs::read_dir(run_user) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let runtime_dir = path.display().to_string();
            if let Ok(sock_entries) = fs::read_dir(&path) {
                for sock in sock_entries.flatten() {
                    let sock_path = sock.path();
                    let file_name = match sock_path.file_name() {
                        Some(name) => name.to_string_lossy().to_string(),
                        None => continue,
                    };
                    if !file_name.starts_with("wayland-") {
                        continue;
                    }
                    if let Ok(metadata) = sock_path.metadata() {
                        let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
                        let candidate = (modified, runtime_dir.clone(), file_name);
                        if best.as_ref().map(|b| candidate.0 > b.0).unwrap_or(true) {
                            best = Some(candidate);
                        }
                    }
                }
            }
        }
    }

    best.map(|(_, runtime_dir, display)| (runtime_dir, display))
}

// Writes the /etc/os-release file for the installed system
pub(crate) fn write_os_release() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let contents = format!(
        "NAME=Nebula\nPRETTY_NAME=\"Nebula {}\"\nID=nebula\nID_LIKE=arch\nVERSION_ID={}\nVERSION=\"{}\"\n",
        version, version, version
    );
    fs::write("/mnt/etc/os-release", contents).context("write os-release")?;
    Ok(())
}

pub(crate) fn close_cryptroot_with_retries(tx: &crossbeam_channel::Sender<InstallerEvent>) {
    const MAX_TRIES: usize = 5;
    send_event(tx, InstallerEvent::Log("Closing cryptroot...".to_string()));
    for attempt in 1..=MAX_TRIES {
        match Command::new("cryptsetup")
            .args(["close", "cryptroot"])
            .status()
        {
            Ok(status) if status.success() => {
                send_event(tx, InstallerEvent::Log("cryptroot closed.".to_string()));
                return;
            }
            Ok(status) => {
                send_event(
                    tx,
                    InstallerEvent::Log(format!(
                        "cryptsetup close failed (attempt {}/{}): exit {}",
                        attempt,
                        MAX_TRIES,
                        status.code().unwrap_or(-1)
                    )),
                );
            }
            Err(err) => {
                send_event(
                    tx,
                    InstallerEvent::Log(format!(
                        "cryptsetup close failed (attempt {}/{}): {}",
                        attempt, MAX_TRIES, err
                    )),
                );
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

pub(crate) fn write_file(path: &str, contents: &str) -> Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent).context("create parent dirs")?;
    }
    fs::write(path, contents).context("write file")?;
    Ok(())
}

// Copies the installer log from /tmp to the installed systems /var/log
pub(crate) fn copy_installer_log(tx: &crossbeam_channel::Sender<InstallerEvent>) {
    let src = Path::new("/tmp/nebula-installer.log");
    let dest = Path::new("/mnt/var/log/nebula-installer.log");
    if !src.exists() {
        return;
    }
    if let Some(parent) = dest.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            send_event(
                tx,
                InstallerEvent::Log(format!("Failed to create log dir: {}", err)),
            );
            return;
        }
    }
    match fs::copy(src, dest) {
        Ok(_) => send_event(
            tx,
            InstallerEvent::Log(format!("Saved installer log to {}", dest.display())),
        ),
        Err(err) => send_event(
            tx,
            InstallerEvent::Log(format!("Failed to save installer log: {}", err)),
        ),
    }
}
