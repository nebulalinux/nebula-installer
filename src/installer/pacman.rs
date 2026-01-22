use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::model::InstallerEvent;

use super::commands::{run_chroot, run_chroot_stream, run_command, run_command_stream};
use super::system::write_file;
use super::{send_event, NEBULA_REPO_KEY_PATH, OFFLINE_PACMAN_CONF_PATH};

// Configures the pacman mirrorlist
pub(crate) fn configure_mirrorlist(path: &str) -> Result<()> {
    let contents = if let Ok(mirrorlist) = env::var("NEBULA_PACMAN_MIRRORLIST") {
        let trimmed = mirrorlist.trim();
        if trimmed.is_empty() {
            String::new()
        } else if trimmed.ends_with('\n') {
            trimmed.to_string()
        } else {
            format!("{trimmed}\n")
        }
    } else if let Ok(mirror) = env::var("NEBULA_PACMAN_MIRROR") {
        let trimmed = mirror.trim();
        if trimmed.is_empty() {
            String::new()
        } else {
            let base = trimmed.trim_end_matches('/');
            format!("Server = {base}/$repo/os/$arch\n")
        }
    } else {
        concat!("Server = https://mirror.nebulalinux.com/stable/$repo/os/$arch\n",).to_string()
    };
    fs::write(path, contents).context("write mirrorlist")?;
    Ok(())
}

// Writes a pacman.conf file for offline installations
pub(crate) fn write_offline_pacman_conf(path: &str) -> Result<()> {
    let contents = concat!(
        "[options]\n",
        "HoldPkg     = pacman glibc\n",
        "Architecture = auto\n",
        "ParallelDownloads = 5\n",
        "SigLevel = Required DatabaseOptional\n",
        "LocalFileSigLevel = Optional\n",
        "\n",
        "[nebula-offline]\n",
        "SigLevel = Optional TrustAll\n",
        "Server = file:///opt/nebula-repo\n",
    );
    fs::write(path, contents).context("write offline pacman.conf")?;
    Ok(())
}

// Writes a pacman.conf file for offline-first installs (offline repo + online fallback)
pub(crate) fn write_hybrid_pacman_conf(path: &str, include_nebula_repo: bool) -> Result<()> {
    let mut contents = String::from(
        "[options]\n\
HoldPkg     = pacman glibc\n\
Architecture = auto\n\
ParallelDownloads = 5\n\
SigLevel = Required DatabaseOptional\n\
LocalFileSigLevel = Optional\n\
\n\
[nebula-offline]\n\
SigLevel = Optional TrustAll\n\
Server = file:///opt/nebula-repo\n\
\n",
    );
    if include_nebula_repo {
        contents.push_str(
            "[nebula]\nSigLevel = Required DatabaseOptional\nServer = https://pkgs.nebulalinux.com/stable/$arch\n\n",
        );
    }
    contents.push_str(
        "[core]\n\
Include = /etc/pacman.d/mirrorlist\n\
\n\
[extra]\n\
Include = /etc/pacman.d/mirrorlist\n\
\n\
[multilib]\n\
Include = /etc/pacman.d/mirrorlist\n",
    );
    fs::write(path, contents).context("write hybrid pacman.conf")?;
    Ok(())
}

// Validates that the required packages
pub(crate) fn validate_offline_packages(packages: &[&str]) -> Result<()> {
    let repo_path = Path::new("/opt/nebula-repo");
    let mut missing = Vec::new();
    for pkg in packages {
        if *pkg == "base" {
            continue;
        }
        let pattern = format!("{}-*.pkg.tar.zst", pkg);
        if !repo_path.join(&pattern).exists() {
            let glob = format!("/opt/nebula-repo/{}", pattern);
            let found = std::fs::read_dir(repo_path)
                .ok()
                .map(|entries| {
                    entries.filter_map(|entry| entry.ok()).any(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(&format!("{}-", pkg))
                            && entry
                                .file_name()
                                .to_string_lossy()
                                .ends_with(".pkg.tar.zst")
                    })
                })
                .unwrap_or(false);
            if !found {
                missing.push(glob);
            }
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "Offline repo missing required packages: {}",
        missing.join(", ")
    );
}

// Validates that the base package group
pub(crate) fn validate_offline_base_package() -> Result<()> {
    let sync_status = Command::new("pacman")
        .args(["--config", OFFLINE_PACMAN_CONF_PATH, "-Sy", "--noconfirm"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("sync offline repo database")?;
    if !sync_status.success() {
        anyhow::bail!("Offline repo sync failed");
    }
    let output = Command::new("pacman")
        .args(["--config", OFFLINE_PACMAN_CONF_PATH, "-Si", "base"])
        .output()
        .context("check base package in offline repo")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Offline repo missing base package: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        anyhow::bail!("Offline repo missing base package");
    }
    Ok(())
}

// Tries to install optional packages individually if the batch install fails
pub(crate) fn install_optional_packages_best_effort(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    packages: &[String],
    pacman_conf: Option<&str>,
) -> Result<Vec<String>> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    if install_pacman_packages(tx, packages, pacman_conf).is_ok() {
        return Ok(Vec::new());
    }
    send_event(
        tx,
        InstallerEvent::Log(
            "Optional package batch install failed. Retrying individually...".to_string(),
        ),
    );
    let mut failed = Vec::new();
    for pkg in packages {
        if let Err(err) = install_pacman_packages(tx, &[pkg.clone()], pacman_conf) {
            send_event(
                tx,
                InstallerEvent::Log(format!("Optional package failed: {} ({})", pkg, err)),
            );
            failed.push(pkg.clone());
        }
    }
    Ok(failed)
}

// Writes a log of failed optional packages to the installed system
pub(crate) fn write_failed_packages_log(packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    fs::create_dir_all("/mnt/var/log").context("create log dir")?;
    let mut contents = String::from("Failed optional packages:\n");
    for pkg in packages {
        contents.push_str(pkg);
        contents.push('\n');
    }
    write_file("/mnt/var/log/nebula-failed-packages.txt", &contents)?;
    Ok(())
}

// Removes duplicate packages from a list
pub(crate) fn dedup_packages(mut packages: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    packages.retain(|pkg| {
        if seen.iter().any(|existing: &String| existing == pkg) {
            false
        } else {
            seen.push(pkg.clone());
            true
        }
    });
    packages
}

// Ensures the Nebula custom package repository is configured in the target system.
pub(crate) fn ensure_nebula_repo_configured(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
) -> Result<()> {
    let key_path = "/usr/share/nebula/nebula-repo.gpg";
    if Path::new(&format!("/mnt{}", key_path)).exists() {
        run_chroot(tx, &["pacman-key", "--add", key_path], None)?;
    } else {
        run_chroot(
            tx,
            &[
                "bash",
                "-c",
                "curl -fsSL https://pkgs.nebulalinux.com/nebula-repo.gpg | pacman-key --add -",
            ],
            None,
        )?;
    }
    run_chroot(
        tx,
        &[
            "pacman-key",
            "--lsign-key",
            "7CB33A71D4C4C529149862B799EC53F7C03BE297",
        ],
        None,
    )?;
    run_chroot(
        tx,
        &[
            "bash",
            "-c",
            r"if ! grep -q '^\[nebula\]' /etc/pacman.conf; then sed -i '/^\[core\]/i [nebula]\nSigLevel = Required DatabaseOptional\nServer = https://pkgs.nebulalinux.com/stable/\$arch\n' /etc/pacman.conf; fi",
        ],
        None,
    )?;
    Ok(())
}

pub(crate) fn import_nebula_repo_key(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
    fs::create_dir_all("/mnt/usr/share/nebula").context("create nebula key dir")?;
    run_command(
        tx,
        "cp",
        &[
            NEBULA_REPO_KEY_PATH,
            "/mnt/usr/share/nebula/nebula-repo.gpg",
        ],
        None,
    )?;
    run_chroot(
        tx,
        &["pacman-key", "--add", "/usr/share/nebula/nebula-repo.gpg"],
        None,
    )?;
    run_chroot(
        tx,
        &[
            "pacman-key",
            "--lsign-key",
            "7CB33A71D4C4C529149862B799EC53F7C03BE297",
        ],
        None,
    )?;
    Ok(())
}

// Installs packages using pacman inside the chroot
pub(crate) fn install_pacman_packages(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    packages: &[String],
    pacman_conf: Option<&str>,
) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let mut args = vec![
        "pacman".to_string(),
        "-S".to_string(),
        "--noconfirm".to_string(),
        "--needed".to_string(),
    ];
    if let Some(conf_path) = pacman_conf {
        args.push("--config".to_string());
        args.push(conf_path.to_string());
    }
    args.extend(packages.iter().cloned());
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_chroot_stream(
        tx,
        &args_ref,
        None,
        Some("Installing packages..."),
        Some(&[("PACMAN_COLOR", "never")]),
    )
}

pub(crate) fn sync_pacman_databases(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    pacman_conf: Option<&str>,
) -> Result<()> {
    let mut args = vec![
        "pacman".to_string(),
        "-Sy".to_string(),
        "--noconfirm".to_string(),
    ];
    if let Some(conf_path) = pacman_conf {
        args.push("--config".to_string());
        args.push(conf_path.to_string());
    }
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_chroot_stream(
        tx,
        &args_ref,
        None,
        Some("Syncing package databases..."),
        Some(&[("PACMAN_COLOR", "never")]),
    )
}

// Special handler for pacstrap, which can have weird output buffering
pub(crate) fn run_pacstrap(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    args: &[&str],
) -> Result<()> {
    let cmdline = format!("pacstrap {}", args.join(" "));
    send_event(
        tx,
        InstallerEvent::Log("Downloading and installing packages...".to_string()),
    );
    send_event(tx, InstallerEvent::Log(format!("$ {}", cmdline)));

    let use_script = Command::new("script")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if use_script {
        let mut pacstrap_cmd = String::from("PACMAN_COLOR=never pacstrap ");
        pacstrap_cmd.insert_str(0, "SYSTEMD_OFFLINE=1 ");
        pacstrap_cmd.push_str(&args.join(" "));
        return run_command_stream(
            tx,
            "script",
            &["-qec", &pacstrap_cmd, "/dev/null"],
            None,
            Some("Still downloading packages..."),
            None,
        );
    }

    run_command_stream(
        tx,
        "pacstrap",
        args,
        None,
        Some("Still downloading packages..."),
        Some(&[("SYSTEMD_OFFLINE", "1"), ("PACMAN_COLOR", "never")]),
    )
}
