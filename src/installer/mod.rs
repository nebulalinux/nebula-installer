/////////
/// Installation process
////////
mod commands;
mod pacman;
mod system;
mod themes;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::disks::DiskInfo;
use crate::model::{InstallerEvent, StepStatus};

use commands::{append_temp_installer_log, run_chroot, run_command, run_command_capture};
use pacman::{
    configure_mirrorlist, dedup_packages, ensure_nebula_repo_configured,
    import_nebula_repo_key, install_optional_packages_best_effort, install_pacman_packages,
    run_pacstrap, sync_pacman_databases, validate_offline_base_package,
    validate_offline_packages, write_failed_packages_log, write_hybrid_pacman_conf,
    write_offline_pacman_conf,
};
use system::{
    close_cryptroot_with_retries, configure_hypr_monitors, configure_zram,
    copy_installer_log, detect_microcode_package, get_uuid, install_nebula_hypr,
    schedule_nebula_theme, write_file, write_os_release,
};
use themes::{
    ensure_grub_cmdline_params, install_grub_theme, install_sddm_theme,
    remove_grub_cmdline_params, set_grub_distributor, set_grub_gfx, update_grub_cmdline,
};

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
pub(crate) const TMP_INSTALLER_LOG: &str = "/tmp/nebula-installer.log";
pub(crate) const OFFLINE_PACMAN_CONF_PATH: &str = "/tmp/nebula-pacman.offline.conf";
pub(crate) const TARGET_OFFLINE_PACMAN_CONF_PATH: &str = "/mnt/etc/pacman.offline.conf";
pub(crate) const TARGET_HYBRID_PACMAN_CONF_PATH: &str = "/mnt/etc/pacman.hybrid.conf";
pub(crate) const NEBULA_REPO_KEY_PATH: &str = "/usr/share/nebula/nebula-repo.gpg";

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
        let mut luks_installed = false;
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
                luks_installed = true;
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
        if config.encrypt_disk {
            if luks_installed {
                run_chroot(&tx, &["plymouth-set-default-theme", "nebula-luks"], None)?;
            }
        } else if splash_installed {
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
        if config.encrypt_disk && !luks_installed {
            send_event(
                &tx,
                InstallerEvent::Log(
                    "Plymouth LUKS theme missing! Disabling quiet splash to ensure crypt prompt is visible.".to_string(),
                ),
            );
            remove_grub_cmdline_params(&["quiet", "splash"])?;
        } else {
            ensure_grub_cmdline_params(&["quiet", "splash"])?;
        }

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

fn send_event(tx: &crossbeam_channel::Sender<InstallerEvent>, evt: InstallerEvent) {
    let _ = tx.try_send(evt);
}
