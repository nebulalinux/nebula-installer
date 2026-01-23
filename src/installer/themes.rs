use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::model::InstallerEvent;

use super::commands::run_command;
use super::send_event;
use super::system::get_wlr_randr_output;
use super::system::write_file;

// Updates the GRUB command line for an encrypted root filesystem
pub(crate) fn update_grub_cmdline(root_uuid: &str) -> Result<()> {
    let path = "/mnt/etc/default/grub";
    let contents = fs::read_to_string(path).context("read grub config")?;
    let mut updated = String::new();
    let mut replaced = false;
    for line in contents.lines() {
        if line.starts_with("GRUB_CMDLINE_LINUX=") {
            let value = format!(
                "GRUB_CMDLINE_LINUX=\"cryptdevice=UUID={}:cryptroot root=/dev/mapper/cryptroot quiet splash\"",
                root_uuid
            );
            updated.push_str(&value);
            updated.push('\n');
            replaced = true;
        } else {
            updated.push_str(line);
            updated.push('\n');
        }
    }
    if !replaced {
        updated.push_str(&format!(
            "GRUB_CMDLINE_LINUX=\"cryptdevice=UUID={}:cryptroot root=/dev/mapper/cryptroot quiet splash\"\n",
            root_uuid
        ));
    }
    fs::write(path, updated).context("write grub config")?;
    Ok(())
}

// Ensures that specific parameters are present in the GRUB command line
pub(crate) fn ensure_grub_cmdline_params(params: &[&str]) -> Result<()> {
    let path = "/mnt/etc/default/grub";
    let contents = fs::read_to_string(path).context("read grub config")?;
    let mut updated = String::new();
    let mut replaced = false;

    for line in contents.lines() {
        if line.starts_with("GRUB_CMDLINE_LINUX=") {
            let mut value = String::new();
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if end > start {
                        let inner = &line[start + 1..end];
                        let mut parts: Vec<&str> = inner.split_whitespace().collect();
                        for param in params {
                            if !parts.iter().any(|existing| existing == param) {
                                parts.push(param);
                            }
                        }
                        value = format!("GRUB_CMDLINE_LINUX=\" { }\"", parts.join(" "));
                    }
                }
            }
            if value.is_empty() {
                value = format!("GRUB_CMDLINE_LINUX=\" { }\"", params.join(" "));
            }
            updated.push_str(&value);
            updated.push('\n');
            replaced = true;
        } else {
            updated.push_str(line);
            updated.push('\n');
        }
    }

    if !replaced {
        updated.push_str(&confirm_cmdline(params));
    }

    fs::write(path, updated).context("write grub config")?;
    Ok(())
}

fn confirm_cmdline(params: &[&str]) -> String {
    format!("GRUB_CMDLINE_LINUX=\" { }\"\n", params.join(" "))
}

pub(crate) fn remove_grub_cmdline_params(params: &[&str]) -> Result<()> {
    let path = "/mnt/etc/default/grub";
    let contents = fs::read_to_string(path).context("read grub config")?;
    let mut updated = String::new();
    let mut replaced = false;

    for line in contents.lines() {
        if line.starts_with("GRUB_CMDLINE_LINUX=") {
            let mut value = String::new();
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if end > start {
                        let inner = &line[start + 1..end];
                        let mut parts: Vec<&str> = inner.split_whitespace().collect();
                        parts.retain(|part| !params.iter().any(|param| param == part));
                        value = format!("GRUB_CMDLINE_LINUX=\" {}\"", parts.join(" "));
                    }
                }
            }
            if value.is_empty() {
                value = "GRUB_CMDLINE_LINUX=\" \"".to_string();
            }
            updated.push_str(&value);
            updated.push('\n');
            replaced = true;
        } else {
            updated.push_str(line);
            updated.push('\n');
        }
    }

    if !replaced {
        updated.push_str("GRUB_CMDLINE_LINUX=\" \"\n");
    }

    fs::write(path, updated).context("write grub config")?;
    Ok(())
}

// Installs the custom Nebula GRUB theme
pub(crate) fn install_grub_theme(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
    let theme_dest = "/mnt/boot/grub/themes/nebula-vimix-grub";

    let theme_src = if let Some(source) = find_grub_theme_source(tx) {
        source
    } else {
        send_event(
            tx,
            InstallerEvent::Log(
                "GRUB theme not found at any known path; skipping theme install.".to_string(),
            ),
        );
        return Ok(());
    };

    let (selection, detected) = detect_grub_theme_selection(tx);
    if let Some((width, height)) = detected {
        send_event(
            tx,
            InstallerEvent::Log(format!(
                "Detected monitor resolution: {}x{}; using GRUB theme variant: {}",
                width, height, selection.folder
            )),
        );
    } else {
        send_event(
            tx,
            InstallerEvent::Log(format!(
                "Monitor resolution not detected; using default GRUB theme variant: {}",
                selection.folder
            )),
        );
    }

    let variant_src = format!("{}/{}", theme_src, selection.folder);
    let variant_src = if Path::new(&variant_src).exists() {
        variant_src
    } else {
        let fallback = format!("{}/1080p", theme_src);
        send_event(
            tx,
            InstallerEvent::Log(format!(
                "GRUB theme variant not found at {}; falling back to 1080p",
                variant_src
            )),
        );
        fallback
    };

    send_event(
        tx,
        InstallerEvent::Log(format!(
            "Installing GRUB theme from {} (variant: {})",
            theme_src, selection.folder
        )),
    );
    run_command(tx, "mkdir", &["-p", "/mnt/boot/grub/themes"], None)?;
    run_command(tx, "mkdir", &["-p", theme_dest], None)?;
    let theme_src_copy = format!("{}/.", theme_src);
    let variant_src_copy = format!("{}/.", variant_src);
    run_command(tx, "cp", &["-a", &theme_src_copy, theme_dest], None)?;
    run_command(tx, "cp", &["-a", &variant_src_copy, theme_dest], None)?;

    let grub_theme_path = "/boot/grub/themes/nebula-vimix-grub/theme.txt";
    let path = "/mnt/etc/default/grub";
    let contents = fs::read_to_string(path).context("read grub config")?;
    let mut updated = String::new();
    let mut replaced = false;

    for line in contents.lines() {
        if line.starts_with("GRUB_THEME=") {
            updated.push_str(&format!("GRUB_THEME=\"{}\"\n", grub_theme_path));
            replaced = true;
        } else {
            updated.push_str(line);
            updated.push('\n');
        }
    }

    if !replaced {
        updated.push_str(&format!("GRUB_THEME=\"{}\"\n", grub_theme_path));
    }

    fs::write(path, updated).context("write grub config")?;
    Ok(())
}

pub(crate) fn find_grub_theme_source(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
) -> Option<String> {
    let theme_sources = [
        "/usr/share/grub/themes/nebula-vimix-grub",
        "/boot/grub/themes/nebula-vimix-grub",
        "/run/archiso/bootmnt/boot/grub/themes/nebula-vimix-grub",
        "/run/archiso/bootmnt/grub/themes/nebula-vimix-grub",
        "/run/archiso/bootmnt/EFI/BOOT/grub/themes/nebula-vimix-grub",
        "/run/archiso/airootfs/usr/share/grub/themes/nebula-vimix-grub",
        "/run/archiso/bootmnt/airootfs/usr/share/grub/themes/nebula-vimix-grub",
    ];

    for source in theme_sources {
        let exists = Path::new(source).exists();
        send_event(
            tx,
            InstallerEvent::Log(format!(
                "Checking GRUB theme path {}: {}",
                source,
                if exists { "found" } else { "missing" }
            )),
        );
        if exists {
            return Some(source.to_string());
        }
    }

    if let Some(found) = find_theme_under("/run/archiso/bootmnt", "nebula-vimix-grub", 5) {
        let found = found.to_string_lossy().to_string();
        send_event(
            tx,
            InstallerEvent::Log(format!("Found GRUB theme via search: {}", found)),
        );
        return Some(found);
    }

    if let Some(found) = find_theme_under("/run/archiso/airootfs", "nebula-vimix-grub", 5) {
        let found = found.to_string_lossy().to_string();
        send_event(
            tx,
            InstallerEvent::Log(format!("Found GRUB theme via airootfs search: {}", found)),
        );
        return Some(found);
    }

    send_event(
        tx,
        InstallerEvent::Log("No GRUB theme found under archiso mounts.".to_string()),
    );
    None
}

// Installs and configures the custom Nebula SDDM theme
pub(crate) fn install_sddm_theme(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
    let theme_sources = [
        "/usr/share/sddm/themes/nebula-sddm",
        "/run/archiso/bootmnt/airootfs/usr/share/sddm/themes/nebula-sddm",
        "/run/archiso/bootmnt/usr/share/sddm/themes/nebula-sddm",
    ];
    let theme_dest = "/mnt/usr/share/sddm/themes/nebula-sddm";

    let mut found = None;
    for source in &theme_sources {
        if Path::new(source).exists() {
            found = Some(*source);
            break;
        }
    }

    let theme_src = if let Some(source) = found {
        source
    } else {
        send_event(
            tx,
            InstallerEvent::Log(
                "SDDM theme not found at any known path; skipping theme install.".to_string(),
            ),
        );
        return Ok(());
    };

    run_command(tx, "mkdir", &["-p", "/mnt/usr/share/sddm/themes"], None)?;
    run_command(tx, "cp", &["-a", theme_src, theme_dest], None)?;
    write_file("/mnt/etc/sddm.conf", "[Theme]\nCurrent=nebula-sddm\n")?;
    fs::create_dir_all("/mnt/etc/sddm.conf.d").context("create sddm.conf.d")?;
    write_file(
        "/mnt/etc/sddm.conf.d/virtualkbd.conf",
        "[General]\nInputMethod=qtvirtualkeyboard\n",
    )?;
    let wlr_output = get_wlr_randr_output(tx);
    let scale = wlr_output
        .as_deref()
        .and_then(detect_scale_from_wlr_randr)
        .or_else(detect_display_scale);
    let scale_value = scale.unwrap_or(1.0);
    if let Some(scale) = scale {
        send_event(
            tx,
            InstallerEvent::Log(format!("SDDM scale factor detected: {:.2}", scale)),
        );
    } else {
        send_event(
            tx,
            InstallerEvent::Log("SDDM scale factor not detected; using auto scaling.".to_string()),
        );
    }
    let greeter_env = if scale.is_some() {
        format!(
            "[General]\nGreeterEnvironment=QT_SCALE_FACTOR={:.2},QT_AUTO_SCREEN_SCALE_FACTOR=0\n\n[Wayland]\nEnableHiDPI=true\n",
            scale_value
        )
    } else {
        "[General]\nGreeterEnvironment=QT_AUTO_SCREEN_SCALE_FACTOR=1\n\n[Wayland]\nEnableHiDPI=true\n".to_string()
    };
    write_file("/mnt/etc/sddm.conf.d/nebula-scale.conf", &greeter_env)?;
    send_event(
        tx,
        InstallerEvent::Log("Installed SDDM theme: nebula-sddm".to_string()),
    );

    Ok(())
}

// Sets the GRUB distributor to "Nebula"
pub(crate) fn set_grub_distributor() -> Result<()> {
    let path = "/mnt/etc/default/grub";
    let contents = fs::read_to_string(path).context("read grub config")?;
    let mut updated = String::new();
    let mut found = false;

    for line in contents.lines() {
        if line.starts_with("GRUB_DISTRIBUTOR=") {
            updated.push_str("GRUB_DISTRIBUTOR=\"Nebula\"\n");
            found = true;
        } else {
            updated.push_str(line);
            updated.push('\n');
        }
    }

    if !found {
        updated.push_str("GRUB_DISTRIBUTOR=\"Nebula\"\n");
    }

    fs::write(path, updated).context("write grub config")?;
    Ok(())
}

// Sets the GRUB menu resolution and keeps it for the kernel payload
pub(crate) fn set_grub_gfx(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
    let path = "/mnt/etc/default/grub";
    let contents = fs::read_to_string(path).context("read grub config")?;
    let mut updated = String::new();
    let mut found_gfx = false;
    let mut found_payload = false;
    let (selection, detected) = detect_grub_theme_selection(tx);
    if let Some((width, height)) = detected {
        send_event(
            tx,
            InstallerEvent::Log(format!(
                "Detected monitor resolution for GRUB gfxmode: {}x{}",
                width, height
            )),
        );
    }

    for line in contents.lines() {
        if line.starts_with("GRUB_GFXMODE=") {
            updated.push_str(&format!("GRUB_GFXMODE={}\n", selection.gfxmode));
            found_gfx = true;
        } else if line.starts_with("GRUB_GFXPAYLOAD_LINUX=") {
            updated.push_str("GRUB_GFXPAYLOAD_LINUX=keep\n");
            found_payload = true;
        } else {
            updated.push_str(line);
            updated.push('\n');
        }
    }

    if !found_gfx {
        updated.push_str(&format!("GRUB_GFXMODE={}\n", selection.gfxmode));
    }
    if !found_payload {
        updated.push_str("GRUB_GFXPAYLOAD_LINUX=keep\n");
    }

    fs::write(path, updated).context("write grub config")?;
    Ok(())
}

// Detects the display scale factor based on EDID information (for SDDM scaling)
fn detect_display_scale() -> Option<f32> {
    let drm_path = Path::new("/sys/class/drm");
    let entries = fs::read_dir(drm_path).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let status_path = path.join("status");
        if !status_path.exists() {
            continue;
        }
        let status = fs::read_to_string(&status_path).ok()?;
        if status.trim() != "connected" {
            continue;
        }
        let mode_path = path.join("modes");
        let modes = fs::read_to_string(&mode_path).ok()?;
        let mode = modes.lines().next()?;
        let (width, height) = parse_mode(mode)?;
        return Some(scale_from_resolution(width, height));
    }
    None
}

fn detect_scale_from_wlr_randr(output: &str) -> Option<f32> {
    let (width, height) = detect_resolution_from_wlr_randr(output)?;
    Some(scale_from_resolution(width, height))
}

#[derive(Clone, Copy)]
struct GrubThemeSelection {
    folder: &'static str,
    gfxmode: &'static str,
}

fn detect_grub_theme_selection(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
) -> (GrubThemeSelection, Option<(u32, u32)>) {
    let detected = detect_grub_resolution(tx);
    let selection = detected
        .map(|(width, height)| select_grub_theme_selection(width, height))
        .unwrap_or_else(default_grub_theme_selection);
    (selection, detected)
}

fn detect_grub_resolution(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Option<(u32, u32)> {
    if let Some(output) = get_wlr_randr_output(tx) {
        if let Some(resolution) = detect_resolution_from_wlr_randr(&output) {
            return Some(resolution);
        }
    }
    detect_resolution_from_drm()
}

fn detect_resolution_from_wlr_randr(output: &str) -> Option<(u32, u32)> {
    let mut best: Option<(u32, u32)> = None;
    for line in output.lines() {
        let line = line.trim_start();
        let first = match line.chars().next() {
            Some(first) => first,
            None => continue,
        };
        if !first.is_ascii_digit() {
            continue;
        }
        let token = match line.split_whitespace().next() {
            Some(token) => token,
            None => continue,
        };
        let is_current = line.contains("current") || token.ends_with('*') || line.contains('*');
        if !is_current {
            continue;
        }
        if let Some((width, height)) = parse_wlr_mode(token) {
            let area = width as u64 * height as u64;
            match best {
                None => best = Some((width, height)),
                Some((best_w, best_h)) => {
                    let best_area = best_w as u64 * best_h as u64;
                    if area > best_area {
                        best = Some((width, height));
                    }
                }
            }
        }
    }
    best
}

fn detect_resolution_from_drm() -> Option<(u32, u32)> {
    let drm_path = Path::new("/sys/class/drm");
    let entries = fs::read_dir(drm_path).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let status_path = path.join("status");
        if !status_path.exists() {
            continue;
        }
        let status = fs::read_to_string(&status_path).ok()?;
        if status.trim() != "connected" {
            continue;
        }
        let mode_path = path.join("modes");
        let modes = fs::read_to_string(&mode_path).ok()?;
        for mode in modes.lines() {
            if let Some((width, height)) = parse_mode(mode) {
                return Some((width, height));
            }
        }
    }
    None
}

fn find_theme_under(root: &str, theme_dir: &str, max_depth: usize) -> Option<PathBuf> {
    let root_path = Path::new(root);
    let mut stack = vec![(root_path.to_path_buf(), 0)];
    while let Some((path, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name()?.to_string_lossy() == theme_dir {
                        return Some(path);
                    }
                    stack.push((path, depth + 1));
                }
            }
        }
    }
    None
}

fn select_grub_theme_selection(width: u32, height: u32) -> GrubThemeSelection {
    if width >= 3840 || height >= 2160 {
        GrubThemeSelection {
            folder: "4k",
            gfxmode: "3840x2160",
        }
    } else if width >= 3440 && height >= 1440 {
        GrubThemeSelection {
            folder: "ultrawide2k",
            gfxmode: "3440x1440",
        }
    } else if width >= 2560 && height <= 1080 {
        GrubThemeSelection {
            folder: "ultrawide",
            gfxmode: "2560x1080",
        }
    } else if width >= 2560 || height >= 1440 {
        GrubThemeSelection {
            folder: "2k",
            gfxmode: "2560x1440",
        }
    } else {
        GrubThemeSelection {
            folder: "1080p",
            gfxmode: "1920x1080",
        }
    }
}

fn default_grub_theme_selection() -> GrubThemeSelection {
    GrubThemeSelection {
        folder: "1080p",
        gfxmode: "1920x1080",
    }
}

fn parse_wlr_mode(token: &str) -> Option<(u32, u32)> {
    let token = token.trim_end_matches(|c| c == '*' || c == '+');
    let mut parts = token.split('x');
    let width = parts.next()?.parse::<u32>().ok()?;
    let height = parts.next()?.parse::<u32>().ok()?;
    Some((width, height))
}

fn scale_from_resolution(width: u32, height: u32) -> f32 {
    if width >= 3840 || height >= 2160 {
        2.0
    } else if width >= 2560 || height >= 1440 {
        1.5
    } else {
        1.0
    }
}

fn parse_mode(mode: &str) -> Option<(u32, u32)> {
    let mut parts = mode.trim().split('x');
    let width = parts.next()?.parse::<u32>().ok()?;
    let height = parts.next()?.parse::<u32>().ok()?;
    Some((width, height))
}
