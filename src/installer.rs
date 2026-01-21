/////////
/// Installation process
////////
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::disks::DiskInfo;
use crate::model::{InstallerEvent, StepStatus};
use crate::monitors::render_hypr_monitors_conf;

// Configuration choices made by the user
pub struct InstallConfig {
    pub disk: DiskInfo,
    pub keymap: String,
    pub timezone: String,
    pub hostname: String,
    pub username: String,
    pub user_password: String,
    pub luks_password: String,
    pub encrypt_disk: bool,
    pub swap_enabled: bool,
    pub driver_packages: Vec<String>,
    pub kernel_package: String,
    pub kernel_headers: String,
    pub base_packages: Vec<String>,
    pub extra_pacman_packages: Vec<String>,
    pub extra_aur_packages: Vec<String>,
    pub offline_only: bool,
    pub hyprland_selected: bool,
}

// Installation steps
pub const STEP_NAMES: [&str; 11] = [
    "Partitioning Disk",
    "Encrypting Disk",
    "Creating File System",
    "Mounting File System",
    "Configuring Zram Swap",
    "Installing Base System",
    "Generating Fstab",
    "Configuring Base System",
    "Installing Packages",
    "Installing Bootloader",
    "Finalizing",
];

const STEP_COUNT: f64 = STEP_NAMES.len() as f64;
const TMP_INSTALLER_LOG: &str = "/tmp/nebula-installer.log";
const OFFLINE_PACMAN_CONF_PATH: &str = "/tmp/nebula-pacman.offline.conf";
const TARGET_OFFLINE_PACMAN_CONF_PATH: &str = "/mnt/etc/pacman.offline.conf";
const TARGET_HYBRID_PACMAN_CONF_PATH: &str = "/mnt/etc/pacman.hybrid.conf";
const NEBULA_REPO_KEY_PATH: &str = "/usr/share/nebula/nebula-repo.gpg";

// The main entry point for the installer logic
pub fn run_installer(
    tx: crossbeam_channel::Sender<InstallerEvent>,
    config: &InstallConfig,
) -> Result<()> {
    let disk_path = config.disk.device_path();
    let efi_part = config.disk.partition_path(1);
    let root_part = config.disk.partition_path(2);
    let root_label = if config.encrypt_disk {
        "cryptroot"
    } else {
        "root"
    };
    let root_device = if config.encrypt_disk {
        "/dev/mapper/cryptroot".to_string()
    } else {
        root_part.clone()
    };
    let offline_repo_available = Path::new("/opt/nebula-repo").exists();
    let mut offline_repo_mounted = false;

    // Step 0: Partition the disk
    run_step(&tx, 0, || {
        send_event(&tx, InstallerEvent::Log(format!("Wiping {}...", disk_path)));
        run_command(&tx, "wipefs", &["-af", &disk_path], None)?;
        run_command(&tx, "parted", &["-s", &disk_path, "mklabel", "gpt"], None)?;
        run_command(
            &tx,
            "parted",
            &["-s", &disk_path, "mkpart", "ESP", "fat32", "1MiB", "513MiB"],
            None,
        )?;
        run_command(
            &tx,
            "parted",
            &["-s", &disk_path, "set", "1", "esp", "on"],
            None,
        )?;
        run_command(
            &tx,
            "parted",
            &["-s", &disk_path, "mkpart", root_label, "513MiB", "100%"],
            None,
        )?;
        Ok(())
    })?;

    // Step 1: Encrypt the disk
    if config.encrypt_disk {
        run_step(&tx, 1, || {
            send_event(&tx, InstallerEvent::Log("Setting up LUKS...".to_string()));
            let luks_input = format!("{}\n{}\n", config.luks_password, config.luks_password);
            run_command(
                &tx,
                "cryptsetup",
                &["luksFormat", "--type", "luks2", "--batch-mode", &root_part],
                Some(&luks_input),
            )?;
            let open_input = format!("{}\n", config.luks_password);
            run_command(
                &tx,
                "cryptsetup",
                &["open", &root_part, "cryptroot"],
                Some(&open_input),
            )?;
            Ok(())
        })?;
    } else {
        skip_step(&tx, 1);
    }

    // Step 2: Create filesystems
    run_step(&tx, 2, || {
        send_event(
            &tx,
            InstallerEvent::Log("Formatting filesystems...".to_string()),
        );
        run_command(&tx, "mkfs.fat", &["-F32", &efi_part], None)?;
        run_command(&tx, "mkfs.btrfs", &["-f", &root_device], None)?;
        Ok(())
    })?;

    // Step 3: Mount filesystems and create Btrfs subvolumes
    run_step(&tx, 3, || {
        run_command(&tx, "mount", &[&root_device, "/mnt"], None)?;
        run_command(&tx, "btrfs", &["subvolume", "create", "/mnt/@"], None)?;
        run_command(&tx, "btrfs", &["subvolume", "create", "/mnt/@home"], None)?;
        run_command(&tx, "umount", &["/mnt"], None)?;
        run_command(
            &tx,
            "mount",
            &["-o", "subvol=@,compress=zstd", &root_device, "/mnt"],
            None,
        )?;
        run_command(&tx, "mkdir", &["-p", "/mnt/home"], None)?;
        run_command(
            &tx,
            "mount",
            &[
                "-o",
                "subvol=@home,compress=zstd",
                &root_device,
                "/mnt/home",
            ],
            None,
        )?;
        run_command(&tx, "mkdir", &["-p", "/mnt/boot"], None)?;
        run_command(&tx, "mount", &[&efi_part, "/mnt/boot"], None)?;
        Ok(())
    })?;

    // Step 4: Configure zram swap
    run_step(&tx, 4, || {
        if config.swap_enabled {
            send_event(
                &tx,
                InstallerEvent::Log("Configuring zram swap...".to_string()),
            );
            configure_zram()?;
        } else {
            send_event(&tx, InstallerEvent::Log("Swap disabled.".to_string()));
        }
        Ok(())
    })?;

    // Step 5: Install the base system using pacstrap
    run_step(&tx, 5, || {
        if config.offline_only && !offline_repo_available {
            anyhow::bail!("Offline repo not found at /opt/nebula-repo");
        }
        let use_offline_base = offline_repo_available || config.offline_only;
        send_event(
            &tx,
            InstallerEvent::Log("Initializing pacman keyring...".to_string()),
        );
        run_command(&tx, "pacman-key", &["--init"], None)?;
        run_command(&tx, "pacman-key", &["--populate", "archlinux"], None)?;
        if use_offline_base {
            send_event(
                &tx,
                InstallerEvent::Log(
                    "Offline repo detected; using it for base system install.".to_string(),
                ),
            );
        } else {
            send_event(
                &tx,
                InstallerEvent::Log(
                    "Setting pacman mirror to geo.mirror.pkgbuild.com...".to_string(),
                ),
            );
            configure_mirrorlist("/etc/pacman.d/mirrorlist")?;
        }

        let mut packages = vec![
            "base",
            "linux-firmware",
            "btrfs-progs",
            "grub",
            "efibootmgr",
            "networkmanager",
            "plymouth",
            "sudo",
            "vim",
            "zram-generator",
        ];
        packages.push(config.kernel_package.as_str());
        for pkg in &config.driver_packages {
            if !packages.iter().any(|existing| existing == pkg) {
                packages.push(pkg.as_str());
            }
        }
        if config
            .driver_packages
            .iter()
            .any(|pkg| pkg == "nvidia-dkms" || pkg == "nvidia-open-dkms")
        {
            packages.push(config.kernel_headers.as_str());
        }
        if let Some(ucode) = detect_microcode_package()? {
            send_event(
                &tx,
                InstallerEvent::Log(format!("Detected CPU microcode: {}", ucode)),
            );
            packages.push(ucode);
        }
        if use_offline_base {
            write_offline_pacman_conf(OFFLINE_PACMAN_CONF_PATH)?;
            validate_offline_base_package()?;
            validate_offline_packages(&packages)?;
        }

        let mut args = Vec::new();
        if use_offline_base {
            args.push("-C".to_string());
            args.push(OFFLINE_PACMAN_CONF_PATH.to_string());
        }
        args.push("/mnt".to_string());
        for pkg in packages {
            args.push(pkg.to_string());
        }
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        send_event(
            &tx,
            InstallerEvent::Log("Downloading and installing packages...".to_string()),
        );
        run_pacstrap(&tx, &args_ref)?;
        configure_mirrorlist("/mnt/etc/pacman.d/mirrorlist")?;
        Ok(())
    })?;

    // Step 6: Generate fstab
    run_step(&tx, 6, || {
        let output = run_command_capture(&tx, "genfstab", &["-U", "/mnt"])?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/mnt/etc/fstab")
            .context("open fstab")?;
        file.write_all(output.as_bytes()).context("write fstab")?;
        Ok(())
    })?;

    // Step 7: Configure the installed system
    run_step(&tx, 7, || {
        write_file("/mnt/etc/hostname", &format!("{}\n", config.hostname))?;
        write_file(
            "/mnt/etc/hosts",
            &format!(
                "127.0.0.1\tlocalhost\n::1\tlocalhost\n127.0.1.1\t{}\n",
                config.hostname
            ),
        )?;
        write_file(
            "/mnt/etc/vconsole.conf",
            &format!("KEYMAP={}\n", config.keymap),
        )?;

        let tz_path = format!("/mnt/usr/share/zoneinfo/{}", config.timezone);
        if !std::path::Path::new(&tz_path).exists() {
            anyhow::bail!("Timezone not found: {}", config.timezone);
        }
        run_chroot(
            &tx,
            &[
                "ln",
                "-sf",
                &format!("/usr/share/zoneinfo/{}", config.timezone),
                "/etc/localtime",
            ],
            None,
        )?;
        run_chroot(&tx, &["hwclock", "--systohc"], None)?;
        run_chroot(&tx, &["timedatectl", "set-ntp", "true"], None)?;
        run_chroot(
            &tx,
            &[
                "sed",
                "-i",
                "s/^#en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/",
                "/etc/locale.gen",
            ],
            None,
        )?;
        run_chroot(&tx, &["locale-gen"], None)?;
        run_chroot(
            &tx,
            &["bash", "-c", "echo LANG=en_US.UTF-8 > /etc/locale.conf"],
            None,
        )?;

        write_os_release()?;
        set_grub_distributor()?;
        set_grub_gfx(&tx)?;

        run_chroot(
            &tx,
            &[
                "useradd",
                "-m",
                "-G",
                "wheel",
                "-s",
                "/bin/zsh",
                &config.username,
            ],
            None,
        )?;
        let pass_input = format!(
            "{}:{}
",
            config.username, config.user_password
        );
        run_chroot(&tx, &["chpasswd"], Some(&pass_input))?;
        run_chroot(&tx, &["passwd", "-l", "root"], None)?;
        run_chroot(
            &tx,
            &[
                "sed",
                "-i",
                "s/^# %wheel ALL=(ALL:ALL) ALL/%wheel ALL=(ALL:ALL) ALL/",
                "/etc/sudoers",
            ],
            None,
        )?;

        let splash_theme_src = "/usr/share/plymouth/themes/nebula-splash";
        let luks_theme_src = "/usr/share/plymouth/themes/nebula-luks";
        let mut splash_installed = false;
        if Path::new(splash_theme_src).exists() {
            run_command(
                &tx,
                "mkdir",
                &["-p", "/mnt/usr/share/plymouth/themes"],
                None,
            )?;
            run_command(
                &tx,
                "cp",
                &["-a", splash_theme_src, "/mnt/usr/share/plymouth/themes/"],
                None,
            )?;
            splash_installed = true;
        } else {
            send_event(
                &tx,
                InstallerEvent::Log(format!(
                    "Plymouth splash theme not found at {}; skipping splash theme install.",
                    splash_theme_src
                )),
            );
        }

        if config.encrypt_disk {
            if Path::new(luks_theme_src).exists() {
                run_command(
                    &tx,
                    "mkdir",
                    &["-p", "/mnt/usr/share/plymouth/themes"],
                    None,
                )?;
                run_command(
                    &tx,
                    "cp",
                    &["-a", luks_theme_src, "/mnt/usr/share/plymouth/themes/"],
                    None,
                )?;
                run_chroot(&tx, &["plymouth-set-default-theme", "nebula-luks"], None)?;
            } else {
                send_event(
                    &tx,
                    InstallerEvent::Log(format!(
                        "Plymouth LUKS theme not found at {}; skipping LUKS theme install.",
                        luks_theme_src
                    )),
                );
            }
        } else if splash_installed {
            run_chroot(&tx, &["plymouth-set-default-theme", "nebula-splash"], None)?;
        }

        install_grub_theme(&tx)?;
        install_sddm_theme(&tx)?;

        let hooks_line = if config.encrypt_disk {
            "s/^HOOKS=.*/HOOKS=(base udev autodetect modconf block keyboard keymap plymouth encrypt filesystems)/"
        } else {
            "s/^HOOKS=.*/HOOKS=(base udev autodetect modconf block keyboard keymap plymouth filesystems)/"
        };
        run_chroot(
            &tx,
            &["sed", "-i", hooks_line, "/etc/mkinitcpio.conf"],
            None,
        )?;
        run_chroot(&tx, &["mkinitcpio", "-P"], None)?;
        if config.encrypt_disk && splash_installed {
            run_chroot(&tx, &["plymouth-set-default-theme", "nebula-splash"], None)?;
        }

        if config.encrypt_disk {
            let root_uuid = get_uuid(&tx, &root_part)?;
            write_file(
                "/mnt/etc/crypttab",
                &format!("cryptroot UUID={} none luks\n", root_uuid),
            )?;
            update_grub_cmdline(&root_uuid)?;
        }
        ensure_grub_cmdline_params(&["quiet", "splash"])?;

        Ok(())
    })?;

    // Step 8: Install additional packages
    run_step(&tx, 8, || {
        send_event(
            &tx,
            InstallerEvent::Log("Installing selected apps and packages...".to_string()),
        );
        let required_pacman_packages = dedup_packages(config.base_packages.clone());
        let mut optional_packages = Vec::new();
        optional_packages.extend(config.extra_pacman_packages.iter().cloned());
        optional_packages.extend(config.extra_aur_packages.iter().cloned());
        let optional_packages = dedup_packages(optional_packages);
        let optional_needs_nebula_repo = optional_packages
            .iter()
            .any(|pkg| pkg == "yay" || pkg == "yay-bin")
            || !config.extra_aur_packages.is_empty();

        if config.offline_only && optional_needs_nebula_repo {
            send_event(
                &tx,
                InstallerEvent::Log(
                    "Offline-only enabled; skipping nebula repo setup.".to_string(),
                ),
            );
        }
        if offline_repo_available {
            fs::create_dir_all("/mnt/opt/nebula-repo").context("create offline repo dir")?;
            run_command(
                &tx,
                "mount",
                &["--bind", "/opt/nebula-repo", "/mnt/opt/nebula-repo"],
                None,
            )?;
            offline_repo_mounted = true;
            write_offline_pacman_conf(TARGET_OFFLINE_PACMAN_CONF_PATH)?;
            if !config.offline_only {
                write_hybrid_pacman_conf(
                    TARGET_HYBRID_PACMAN_CONF_PATH,
                    optional_needs_nebula_repo,
                )?;
            }
        }
        if offline_repo_available && Path::new(NEBULA_REPO_KEY_PATH).exists() {
            import_nebula_repo_key(&tx)?;
        }
        if !config.offline_only || Path::new("/mnt/usr/share/nebula/nebula-repo.gpg").exists() {
            ensure_nebula_repo_configured(&tx)?;
        }
        let mut system_db_synced = false;
        if !required_pacman_packages.is_empty() {
            let required_conf = if offline_repo_available || config.offline_only {
                Some("/etc/pacman.offline.conf")
            } else {
                None
            };
            sync_pacman_databases(&tx, required_conf)?;
            if required_conf.is_none() {
                system_db_synced = true;
            }
            install_pacman_packages(&tx, &required_pacman_packages, required_conf)?;
        }
        if !optional_packages.is_empty() {
            let optional_conf = if config.offline_only {
                Some("/etc/pacman.offline.conf")
            } else if offline_repo_available {
                Some("/etc/pacman.hybrid.conf")
            } else {
                None
            };
            if optional_conf != Some("/etc/pacman.offline.conf") {
                sync_pacman_databases(&tx, optional_conf)?;
                if optional_conf.is_none() {
                    system_db_synced = true;
                }
            }
            let failed =
                install_optional_packages_best_effort(&tx, &optional_packages, optional_conf)?;
            if !failed.is_empty() {
                send_event(
                    &tx,
                    InstallerEvent::Log(
                        "Some optional packages failed to install. See /var/log/nebula-failed-packages.txt".to_string(),
                    ),
                );
                write_failed_packages_log(&failed)?;
                append_temp_installer_log(
                    "Optional packages failed. See /var/log/nebula-failed-packages.txt",
                );
            }
        }
        if !config.offline_only && !system_db_synced {
            send_event(
                &tx,
                InstallerEvent::Log("Syncing nebula repo database for first boot...".to_string()),
            );
            if let Err(err) = sync_pacman_databases(&tx, None) {
                send_event(
                    &tx,
                    InstallerEvent::Log(format!(
                        "Warning: failed to sync package databases: {}",
                        err
                    )),
                );
            }
        }

        // Ensure the primary user gets the default .zshrc if it didn't exist at user creation time.
        let zsh_setup_cmd = format!(
            "if [ -f /etc/skel/.zshrc ] && [ ! -f /home/{0}/.zshrc ]; then \
             cp /etc/skel/.zshrc /home/{0}/.zshrc; \
             chown {0}:{0} /home/{0}/.zshrc; \
             fi; \
             if [ -d /etc/skel/.config/oh-my-zsh/custom/plugins ]; then \
             mkdir -p /home/{0}/.config/oh-my-zsh/custom; \
             cp -a -n /etc/skel/.config/oh-my-zsh/custom/plugins /home/{0}/.config/oh-my-zsh/custom/; \
             chown -R {0}:{0} /home/{0}/.config/oh-my-zsh/custom; \
             fi",
            config.username
        );
        run_chroot(&tx, &["bash", "-c", &zsh_setup_cmd], None)?;

        Ok(())
    })?;

    // Step 9: Install the GRUB bootloader
    run_step(&tx, 9, || {
        run_chroot(
            &tx,
            &[
                "grub-install",
                "--target=x86_64-efi",
                "--efi-directory=/boot",
                "--bootloader-id=GRUB",
            ],
            None,
        )?;
        run_chroot(&tx, &["grub-mkconfig", "-o", "/boot/grub/grub.cfg"], None)?;
        Ok(())
    })?;

    // Step 10: Finalize the installation
    run_step(&tx, 10, || {
        run_chroot(&tx, &["systemctl", "enable", "NetworkManager"], None)?;
        if config.base_packages.iter().any(|pkg| pkg == "sddm") {
            run_chroot(&tx, &["systemctl", "enable", "sddm"], None)?;
        } else {
            send_event(
                &tx,
                InstallerEvent::Log(
                    "SDDM not in base package list; skipping service enable.".to_string(),
                ),
            );
        }
        if config.hyprland_selected {
            install_nebula_hypr(&tx, &config.username)?;
            configure_hypr_monitors(&tx, &config.username)?;
            schedule_nebula_theme(&tx, &config.username)?;
        }
        let home_config = format!("/home/{}/.config", config.username);
        let home_local = format!("/home/{}/.local", config.username);
        let home_owner = format!("{}:{}", config.username, config.username);
        if let Err(err) = run_chroot(
            &tx,
            &["chown", "-R", &home_owner, &home_config, &home_local],
            None,
        ) {
            send_event(
                &tx,
                InstallerEvent::Log(format!("Failed to chown home dirs: {}", err)),
            );
        }
        if let Err(err) = run_chroot(
            &tx,
            &["sudo", "-u", &config.username, "xdg-user-dirs-update"],
            None,
        ) {
            send_event(
                &tx,
                InstallerEvent::Log(format!("xdg-user-dirs-update failed: {}", err)),
            );
        }
        copy_installer_log(&tx);
        run_command(&tx, "sync", &[], None)?;
        if offline_repo_mounted {
            run_command(&tx, "umount", &["/mnt/opt/nebula-repo"], None)?;
        }
        run_command(&tx, "umount", &["-R", "/mnt"], None)?;
        if config.encrypt_disk {
            close_cryptroot_with_retries(&tx);
        }
        Ok(())
    })?;

    send_event(&tx, InstallerEvent::Done(None));
    Ok(())
}

fn run_step<F>(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    index: usize,
    action: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    send_event(
        tx,
        InstallerEvent::Step {
            index,
            status: StepStatus::Running,
            err: None,
        },
    );

    if let Err(err) = action() {
        send_event(
            tx,
            InstallerEvent::Step {
                index,
                status: StepStatus::Failed,
                err: Some(err.to_string()),
            },
        );
        return Err(err);
    }

    send_event(
        tx,
        InstallerEvent::Step {
            index,
            status: StepStatus::Done,
            err: None,
        },
    );
    let progress = (index as f64 + 1.0) / STEP_COUNT;
    send_event(tx, InstallerEvent::Progress(progress));
    Ok(())
}

// Skips an installation step
fn skip_step(tx: &crossbeam_channel::Sender<InstallerEvent>, index: usize) {
    send_event(
        tx,
        InstallerEvent::Step {
            index,
            status: StepStatus::Skipped,
            err: None,
        },
    );
    let progress = (index as f64 + 1.0) / STEP_COUNT;
    send_event(tx, InstallerEvent::Progress(progress));
}

// Detects the CPU
fn detect_microcode_package() -> Result<Option<&'static str>> {
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
fn configure_zram() -> Result<()> {
    let contents = "[zram0]\nzram-size = ram\n";
    fs::create_dir_all("/mnt/etc/systemd").context("create systemd dir")?;
    fs::write("/mnt/etc/systemd/zram-generator.conf", contents).context("write zram config")?;
    Ok(())
}

// Configures the pacman mirrorlist
fn configure_mirrorlist(path: &str) -> Result<()> {
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
fn write_offline_pacman_conf(path: &str) -> Result<()> {
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
fn write_hybrid_pacman_conf(path: &str, include_nebula_repo: bool) -> Result<()> {
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
fn validate_offline_packages(packages: &[&str]) -> Result<()> {
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
fn validate_offline_base_package() -> Result<()> {
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

// Appends a line to the temporary installer log
fn append_temp_installer_log(line: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(TMP_INSTALLER_LOG)
    {
        let _ = writeln!(file, "{}", line);
    }
}

// Tries to install optional packages individually if the batch install fails
fn install_optional_packages_best_effort(
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
fn write_failed_packages_log(packages: &[String]) -> Result<()> {
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

// Copies the installer log from /tmp to the installed systems /var/log
fn copy_installer_log(tx: &crossbeam_channel::Sender<InstallerEvent>) {
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

// Removes duplicate packages from a list
fn dedup_packages(mut packages: Vec<String>) -> Vec<String> {
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
fn ensure_nebula_repo_configured(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
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

fn import_nebula_repo_key(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
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
fn install_pacman_packages(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    packages: &[String],
    pacman_conf: Option<&str>,
) -> Result<()> {
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
    for pkg in packages {
        args.push(pkg.to_string());
    }
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_chroot_stream(
        tx,
        &args_ref,
        None,
        Some("Still installing packages..."),
        Some(&[("PACMAN_COLOR", "never")]),
    )
}

fn sync_pacman_databases(
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

// Helper to run a command inside the arch-chroot environment
fn run_chroot(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    args: &[&str],
    input: Option<&str>,
) -> Result<()> {
    let mut cmd = vec!["/mnt".to_string()];
    cmd.extend(args.iter().map(|s| s.to_string()));
    let args_ref: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    run_command(tx, "arch-chroot", &args_ref, input)
}

// Helper to run a streaming command inside the arch-chroot environment
fn run_chroot_stream(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    args: &[&str],
    input: Option<&str>,
    heartbeat: Option<&str>,
    envs: Option<&[(&str, &str)]>,
) -> Result<()> {
    let mut cmd = vec!["/mnt".to_string()];
    cmd.extend(args.iter().map(|s| s.to_string()));
    let args_ref: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    run_command_stream(tx, "arch-chroot", &args_ref, input, heartbeat, envs)
}

// A generic helper to run an external command and stream its output
fn run_command(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    command: &str,
    args: &[&str],
    input: Option<&str>,
) -> Result<()> {
    let cmdline = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    send_event(tx, InstallerEvent::Log(format!("$ {}", cmdline)));

    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {}", command))?;

    if let Some(data) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(data.as_bytes()).context("write stdin")?;
        }
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tx_out = tx.clone();
    let tx_err = tx.clone();

    let out_handle = stdout.map(|out| {
        thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines().flatten() {
                send_event(&tx_out, InstallerEvent::Log(line));
            }
        })
    });

    let err_handle = stderr.map(|err| {
        thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines().flatten() {
                send_event(&tx_err, InstallerEvent::Log(line));
            }
        })
    });

    let status = child.wait().context("wait")?;
    if let Some(handle) = out_handle {
        let _ = handle.join();
    }
    if let Some(handle) = err_handle {
        let _ = handle.join();
    }

    if !status.success() {
        anyhow::bail!("Command failed: {}", cmdline);
    }
    Ok(())
}

// A more advanced command runner that streams output line-by-line and provides a heartbeat
fn run_command_stream(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    command: &str,
    args: &[&str],
    input: Option<&str>,
    heartbeat: Option<&str>,
    envs: Option<&[(&str, &str)]>,
) -> Result<()> {
    let cmdline = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    send_event(tx, InstallerEvent::Log(format!("$ {}", cmdline)));

    let mut cmd = Command::new(command);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(envs) = envs {
        for (key, value) in envs {
            cmd.env(key, value);
        }
    }
    let mut child = cmd.spawn().with_context(|| format!("spawn {}", command))?;

    if let Some(data) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(data.as_bytes()).context("write stdin")?;
        }
    }

    let running = Arc::new(AtomicBool::new(true));
    if let Some(message) = heartbeat {
        let running = Arc::clone(&running);
        let tx = tx.clone();
        let message = message.to_string();
        thread::spawn(move || {
            send_event(&tx, InstallerEvent::Log(message.clone()));
            while running.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(10));
                if running.load(Ordering::Relaxed) {
                    send_event(&tx, InstallerEvent::Log(message.clone()));
                }
            }
        });
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tx_out = tx.clone();
    let tx_err = tx.clone();

    let out_handle = stdout.map(|out| thread::spawn(move || stream_command_output(out, &tx_out)));

    let err_handle = stderr.map(|err| thread::spawn(move || stream_command_output(err, &tx_err)));

    let status = child.wait().context("wait")?;
    running.store(false, Ordering::Relaxed);
    if let Some(handle) = out_handle {
        let _ = handle.join();
    }
    if let Some(handle) = err_handle {
        let _ = handle.join();
    }

    if !status.success() {
        anyhow::bail!("Command failed: {}", cmdline);
    }
    Ok(())
}

// Special handler for pacstrap, which can have weird output buffering
fn run_pacstrap(tx: &crossbeam_channel::Sender<InstallerEvent>, args: &[&str]) -> Result<()> {
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

// Runs a command and captures its stdout
fn run_command_capture(
    tx: &crossbeam_channel::Sender<InstallerEvent>,
    command: &str,
    args: &[&str],
) -> Result<String> {
    let cmdline = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    send_event(tx, InstallerEvent::Log(format!("$ {}", cmdline)));

    let output = Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("run {}", command))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Command failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// Gets the UUID of a block device
fn get_uuid(tx: &crossbeam_channel::Sender<InstallerEvent>, device: &str) -> Result<String> {
    let output = run_command_capture(tx, "blkid", &["-s", "UUID", "-o", "value", device])?;
    Ok(output.trim().to_string())
}

// Updates the GRUB command line for an encrypted root filesystem
fn update_grub_cmdline(root_uuid: &str) -> Result<()> {
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
fn ensure_grub_cmdline_params(params: &[&str]) -> Result<()> {
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
        updated.push_str(&format!("GRUB_CMDLINE_LINUX=\" { }\"\n", params.join(" ")));
    }

    fs::write(path, updated).context("write grub config")?;
    Ok(())
}

// Installs the custom Nebula GRUB theme
fn install_grub_theme(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
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

fn find_grub_theme_source(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Option<String> {
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
fn install_sddm_theme(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
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

// Installs Hyprland user config from nebula-hypr
fn install_nebula_hypr(
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
        InstallerEvent::Log(format!("Installing Hyprland defaults from {}...", script)),
    );
    run_command(tx, "bash", &[script, "/mnt", username], None)?;
    Ok(())
}

// Schedules a GNOME dark theme application on first login via autostart and Hyprland exec-once
fn schedule_nebula_theme(
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
    let hypr_main = "/mnt/usr/share/nebula-hypr/hyprland.conf";
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
    if Path::new(hypr_main).exists() {
        let existing = fs::read_to_string(hypr_main).unwrap_or_default();
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
            fs::write(hypr_main, updated).context("append hypr theme include")?;
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

fn configure_hypr_monitors(
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

const WLR_RANDR_CACHE_PATH: &str = "/tmp/nebula-wlr-randr.txt";

fn get_wlr_randr_output(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Option<String> {
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

fn run_wlr_randr(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<String> {
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

fn find_wayland_socket() -> Option<(String, String)> {
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
fn write_os_release() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let contents = format!(
        "NAME=Nebula\nPRETTY_NAME=\"Nebula {}\"\nID=nebula\nID_LIKE=arch\nVERSION_ID={}\nVERSION=\"{}\"\n",
        version, version, version
    );
    fs::write("/mnt/etc/os-release", contents).context("write os-release")?;
    Ok(())
}

// Sets the GRUB distributor to "Nebula"
fn set_grub_distributor() -> Result<()> {
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
fn set_grub_gfx(tx: &crossbeam_channel::Sender<InstallerEvent>) -> Result<()> {
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

// Streams the output of a command, sending each line as a log event
fn stream_command_output<R: std::io::Read>(
    reader: R,
    tx: &crossbeam_channel::Sender<InstallerEvent>,
) {
    let mut buffer = [0u8; 4096];
    let mut line = String::new();
    let mut pending_cr = false;
    let mut reader = reader;
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };
        let chunk = String::from_utf8_lossy(&buffer[..count]);
        for ch in chunk.chars() {
            if pending_cr {
                if ch == '\n' {
                    let trimmed = sanitize_log_line(&line);
                    if !trimmed.is_empty() {
                        send_event(tx, InstallerEvent::Log(trimmed));
                    }
                    line.clear();
                    pending_cr = false;
                    continue;
                }
                line.clear();
                pending_cr = false;
            }
            if ch == '\r' {
                pending_cr = true;
                continue;
            }
            if ch == '\n' {
                let trimmed = sanitize_log_line(&line);
                if !trimmed.is_empty() {
                    send_event(tx, InstallerEvent::Log(trimmed));
                }
                line.clear();
            } else {
                line.push(ch);
            }
        }
    }
    if pending_cr {
        let trimmed = sanitize_log_line(&line);
        if !trimmed.is_empty() {
            send_event(tx, InstallerEvent::Log(trimmed));
        }
        return;
    }
    let trimmed = sanitize_log_line(&line);
    if !trimmed.is_empty() {
        send_event(tx, InstallerEvent::Log(trimmed));
    }
}

// Removes ANSI escape codes and other control characters from log lines
fn sanitize_log_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    b'[' => {
                        i += 1;
                        while i < bytes.len() {
                            let b = bytes[i];
                            if (0x40..=0x7e).contains(&b) {
                                i += 1;
                                break;
                            }
                            i += 1;
                        }
                    }
                    b']' => {
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            b if b.is_ascii_control() => {
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    let cleaned = String::from_utf8_lossy(&out);
    cleaned.trim().to_string()
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
        let edid_path = path.join("edid");
        let edid = fs::read(&edid_path).ok()?;
        if edid.len() < 23 {
            continue;
        }
        let width_cm = edid[21] as f32;
        let height_cm = edid[22] as f32;
        if width_cm <= 0.0 || height_cm <= 0.0 {
            continue;
        }
        let width_in = width_cm / 2.54;
        let height_in = height_cm / 2.54;
        if width_in <= 0.0 || height_in <= 0.0 {
            continue;
        }
        let dpi_x = width as f32 / width_in;
        let dpi_y = height as f32 / height_in;
        let dpi = (dpi_x + dpi_y) / 2.0;
        let scale = if dpi >= 220.0 {
            2.0
        } else if dpi >= 170.0 {
            1.5
        } else if dpi >= 140.0 {
            1.25
        } else {
            1.0
        };
        return Some(scale);
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
        let mode = modes.lines().next()?;
        return parse_mode(mode);
    }
    None
}

fn find_theme_under(root: &str, theme_dir: &str, max_depth: usize) -> Option<std::path::PathBuf> {
    let root_path = std::path::PathBuf::from(root);
    if !root_path.exists() {
        return None;
    }
    let mut stack = vec![(root_path, 0usize)];
    while let Some((path, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if !entry_path.is_dir() {
                continue;
            }
            let name = match entry_path.file_name() {
                Some(name) => name.to_string_lossy(),
                None => continue,
            };
            if name == theme_dir {
                return Some(entry_path);
            }
            if depth < max_depth {
                stack.push((entry_path, depth + 1));
            }
        }
    }
    None
}

fn select_grub_theme_selection(width: u32, height: u32) -> GrubThemeSelection {
    if width >= 3840 && height >= 2160 {
        GrubThemeSelection {
            folder: "4k",
            gfxmode: "3840x2160,auto",
        }
    } else if width >= 3440 && height >= 1440 {
        GrubThemeSelection {
            folder: "ultrawide2k",
            gfxmode: "3440x1440,auto",
        }
    } else if width >= 2560 && height >= 1440 {
        GrubThemeSelection {
            folder: "2k",
            gfxmode: "2560x1440,auto",
        }
    } else if width >= 2560 && height >= 1080 {
        GrubThemeSelection {
            folder: "ultrawide",
            gfxmode: "2560x1080,auto",
        }
    } else {
        default_grub_theme_selection()
    }
}

fn default_grub_theme_selection() -> GrubThemeSelection {
    GrubThemeSelection {
        folder: "1080p",
        gfxmode: "1920x1080,auto",
    }
}

fn parse_wlr_mode(token: &str) -> Option<(u32, u32)> {
    let token = token.trim_end_matches('*');
    let mode = token.split('@').next().unwrap_or(token);
    parse_mode(mode)
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

fn close_cryptroot_with_retries(tx: &crossbeam_channel::Sender<InstallerEvent>) {
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
        log_busy_mounts(tx);
        thread::sleep(Duration::from_secs(1));
    }
    send_event(
        tx,
        InstallerEvent::Log(
            "cryptroot still busy after retries; continuing without closing.".to_string(),
        ),
    );
}

fn log_busy_mounts(tx: &crossbeam_channel::Sender<InstallerEvent>) {
    if let Ok(mounts) = fs::read_to_string("/proc/mounts") {
        let mut count = 0;
        for line in mounts.lines() {
            if line.contains(" /mnt") {
                send_event(tx, InstallerEvent::Log(format!("mount: {}", line)));
                count += 1;
                if count >= 20 {
                    send_event(tx, InstallerEvent::Log("mount: (truncated)".to_string()));
                    break;
                }
            }
        }
    }

    if let Ok(output) = Command::new("fuser").args(["-vm", "/mnt"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        for line in stdout.lines().chain(stderr.lines()) {
            if !line.trim().is_empty() {
                send_event(tx, InstallerEvent::Log(format!("fuser: {}", line)));
            }
        }
    }
}

// Write a string to a file
fn write_file(path: &str, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("write {}", path))
}

// A wrapper for sending events that ignores send errors
fn send_event(tx: &crossbeam_channel::Sender<InstallerEvent>, evt: InstallerEvent) {
    let _ = tx.try_send(evt);
}
