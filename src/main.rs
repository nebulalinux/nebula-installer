mod disks;
mod drivers;
mod installer;
mod keymaps;
mod model;
mod monitors;
mod network;
mod packages;
mod selection;
mod timezones;
mod ui;

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, ClearType};
use crossterm::{cursor, execute, terminal::Clear};
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Terminal;

// Import everything from our modules
use crate::disks::{list_disks, DiskInfo};
use crate::drivers::{
    detect_gpu_vendors, driver_packages, format_gpu_summary, nvidia_variant_label, GpuVendor,
    NvidiaVariant,
};
use crate::installer::{run_installer, InstallConfig, STEP_NAMES};
use crate::keymaps::{find_keymap_index, load_keymaps};
use crate::model::{App, InstallerEvent, Step, StepStatus};
use crate::network::{
    active_connection_label, connect_wifi_profile, disconnect_wifi_device, forget_wifi_connection,
    has_wifi_device, is_network_ready, is_wifi_connected, list_wifi_networks, wifi_device_name,
    wifi_device_state,
};
use crate::packages::{hyprland_packages, required_packages};
use crate::selection::{
    labels_for_flags, labels_for_selection, selection_from_app_flags, AppSelectionFlags,
    PackageSelection, BROWSER_CHOICES, COMPOSITOR_LABELS, EDITOR_CHOICES, TERMINAL_CHOICES,
};
use crate::timezones::{
    detect_timezone_geoip, detect_timezone_local, find_timezone_index, load_timezones,
};
use crate::ui::{
    draw_ui, render_text_input, render_timezone_loading, render_wifi_connecting,
    render_wifi_searching, run_application_selector, run_confirm_selector, run_disk_selector,
    run_keymap_selector, run_network_required, run_nvidia_selector, run_review, run_text_input,
    run_timezone_selector, run_wifi_selector, ConfirmAction, InputAction, InstallSummary,
    NetworkAction, NvidiaAction, ReviewAction, ReviewItem, SelectionAction, WifiAction, SPINNER,
    SPINNER_LEN, SUMMARY_STEP_COUNT,
};

// Logging
const LOG_CAPACITY: usize = 200;
const LOG_FILE_PATH: &str = "/tmp/nebula-installer.log";

// Pre-installation setup UI
#[derive(Clone, Copy, Debug)]
enum SetupStep {
    Network,
    Disk,
    ConfirmDisk,
    Keymap,
    Timezone,
    Hostname,
    Username,
    UserPassword,
    EncryptDisk,
    LuksPassword,
    Drivers,
    Swap,
    Applications,
    Review,
}

// Maps the current setup step to an index for the UI summary view
fn summary_current_index(step: SetupStep, include_drivers: bool) -> usize {
    let step_count = SUMMARY_STEP_COUNT + if include_drivers { 1 } else { 0 };
    match step {
        SetupStep::Network => 0,
        SetupStep::Drivers => 1,
        SetupStep::Disk | SetupStep::ConfirmDisk => {
            if include_drivers {
                2
            } else {
                1
            }
        }
        SetupStep::Keymap => {
            if include_drivers {
                3
            } else {
                2
            }
        }
        SetupStep::Timezone => {
            if include_drivers {
                4
            } else {
                3
            }
        }
        SetupStep::Hostname => {
            if include_drivers {
                5
            } else {
                4
            }
        }
        SetupStep::Username | SetupStep::UserPassword => {
            if include_drivers {
                6
            } else {
                5
            }
        }
        SetupStep::EncryptDisk | SetupStep::LuksPassword => {
            if include_drivers {
                7
            } else {
                6
            }
        }
        SetupStep::Swap => {
            if include_drivers {
                8
            } else {
                7
            }
        }
        SetupStep::Applications | SetupStep::Review => step_count,
    }
}

// See if a timezone is a variant of UTC
fn is_utc_variant(value: &str) -> bool {
    matches!(value, "UTC" | "Etc/UTC" | "Etc/GMT" | "GMT")
}

fn build_install_summary(
    step: SetupStep,
    include_drivers: bool,
    network: Option<&str>,
    selected_disk: Option<&DiskInfo>,
    keymap: &str,
    timezone: &str,
    hostname: &str,
    username: &str,
    user_password: &str,
    luks_password: &str,
    encrypt_disk: bool,
    swap_enabled: bool,
    nvidia_variant: Option<NvidiaVariant>,
) -> InstallSummary {
    let drivers = if include_drivers {
        Some(
            nvidia_variant
                .map(nvidia_variant_label)
                .unwrap_or("Skipped")
                .to_string(),
        )
    } else {
        None
    };
    InstallSummary {
        current_index: summary_current_index(step, include_drivers),
        network: network.map(|value| value.to_string()),
        drivers,
        disk: selected_disk.map(|disk| disk.label()),
        keymap: Some(keymap.to_string()),
        timezone: Some(timezone.to_string()),
        hostname: Some(hostname.to_string()),
        username: if user_password.is_empty() || username.is_empty() {
            None
        } else {
            Some(username.to_string())
        },
        encryption: if !encrypt_disk {
            Some("no".to_string())
        } else if luks_password.is_empty() {
            None
        } else {
            Some("Btrfs (LUKS encrypted)".to_string())
        },
        zram_swap: Some(if swap_enabled { "yes" } else { "no" }.to_string()),
        include_drivers,
    }
}

fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    // The installer must be run as root
    let allow_nonroot = std::env::var("NEBULA_DEV_ALLOW_NONROOT").ok().as_deref() == Some("1");
    if unsafe { libc::geteuid() } != 0 && !allow_nonroot {
        println!("nebula should be run as root in the live ISO.");
        println!("If you are testing locally, use sudo.");
        return Ok(());
    }

    // Initial data loading
    let disks = list_disks().context("list disks")?;
    if disks.is_empty() {
        println!("No disks detected.");
        return Ok(());
    }
    let mut base_packages = required_packages();
    base_packages.extend(hyprland_packages());

    // Set up the terminal for TUI interaction
    enable_raw_mode().context("enable raw mode")?;
    clear_screen()?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).context("init terminal")?;

    let mut selected_disk: Option<DiskInfo> = None;
    let mut keymap = "us".to_string();
    let keymaps = load_keymaps().unwrap_or_else(|_| vec!["us".to_string()]);
    let timezones = load_timezones().unwrap_or_else(|_| vec!["UTC".to_string()]);
    let mut timezone = detect_timezone_local(&timezones).unwrap_or_default();
    let mut hostname = "nebula".to_string();
    let mut network_label: Option<String> = None;
    let mut username = String::new();
    let mut user_password = String::new();
    let mut luks_password = String::new();
    let mut encrypt_disk = true;
    let mut swap_enabled = true;
    let mut app_flags = AppSelectionFlags::new();
    let mut app_selection = PackageSelection::default();
    let gpu_vendors = detect_gpu_vendors().unwrap_or_default();
    let include_drivers = gpu_vendors.contains(&GpuVendor::Nvidia);
    let mut nvidia_variant: Option<NvidiaVariant> = None;
    let kernel_package = "linux".to_string();
    let kernel_headers = "linux-headers".to_string();
    let mut force_network = false;
    let offline_only = std::env::var("NEBULA_OFFLINE_ONLY").ok().as_deref() == Some("1");

    // The main setup loop
    let mut step = SetupStep::Network;
    'setup: loop {
        match step {
            SetupStep::Network => {
                if std::env::var("NEBULA_SKIP_NETWORK").ok().as_deref() == Some("1") {
                    network_label = Some("Skipped (dev)".to_string());
                    if gpu_vendors.contains(&GpuVendor::Nvidia) {
                        step = SetupStep::Drivers;
                    } else {
                        step = SetupStep::Disk;
                    }
                    continue;
                }
                let mut editing_network = force_network;
                force_network = false;
                if editing_network && !has_wifi_device().unwrap_or(false) {
                    editing_network = false;
                }
                if !editing_network && is_network_ready().unwrap_or(false) {
                    if network_label.is_none() {
                        network_label = active_connection_label().ok().flatten();
                        if network_label.is_none() {
                            network_label = Some("Connected".to_string());
                        }
                    }
                    if gpu_vendors.contains(&GpuVendor::Nvidia) {
                        step = SetupStep::Drivers;
                    } else {
                        step = SetupStep::Disk;
                    }
                    continue;
                }
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                let wifi_supported = has_wifi_device().unwrap_or(false);
                if !wifi_supported {
                    match run_network_required(&mut terminal, &summary)? {
                        NetworkAction::Retry => {}
                        NetworkAction::Quit => {
                            disable_raw_mode().context("disable raw mode")?;
                            let _ = clear_screen();
                            return Ok(());
                        }
                    }
                    continue;
                }
                let mut status_message: Option<String> = None;
                let mut wifi_connected = false;
                let mut last_connect_at: Option<Instant> = None;
                loop {
                    let mut internet_ready = is_network_ready().unwrap_or(false);
                    if internet_ready && network_label.is_none() {
                        network_label = active_connection_label().ok().flatten();
                        if network_label.is_none() {
                            network_label = Some("Connected".to_string());
                        }
                    }
                    let summary = build_install_summary(
                        step,
                        include_drivers,
                        network_label.as_deref(),
                        selected_disk.as_ref(),
                        &keymap,
                        &timezone,
                        &hostname,
                        &username,
                        &user_password,
                        &luks_password,
                        encrypt_disk,
                        swap_enabled,
                        nvidia_variant,
                    );
                    render_wifi_searching(
                        &mut terminal,
                        status_message.as_deref(),
                        wifi_connected,
                        internet_ready,
                        &summary,
                    )?;
                    let networks = match list_wifi_networks() {
                        Ok(list) => list,
                        Err(err) => {
                            status_message = Some(err.to_string());
                            Vec::new()
                        }
                    };
                    wifi_connected = networks.iter().any(|network| network.in_use);
                    if wifi_connected {
                        last_connect_at = None;
                    } else if let Some(connected_at) = last_connect_at {
                        if connected_at.elapsed() < Duration::from_secs(5) {
                            wifi_connected = true;
                        } else {
                            last_connect_at = None;
                        }
                    }
                    let summary = build_install_summary(
                        step,
                        include_drivers,
                        network_label.as_deref(),
                        selected_disk.as_ref(),
                        &keymap,
                        &timezone,
                        &hostname,
                        &username,
                        &user_password,
                        &luks_password,
                        encrypt_disk,
                        swap_enabled,
                        nvidia_variant,
                    );
                    match run_wifi_selector(
                        &mut terminal,
                        &networks,
                        status_message.as_deref(),
                        wifi_connected,
                        internet_ready,
                        &summary,
                    )? {
                        WifiAction::Submit(index) => {
                            let Some(network) = networks.get(index) else {
                                continue;
                            };
                            let needs_password = !network.is_open();
                            let mut password: Option<String> = None;
                            if needs_password {
                                let mut password_error: Option<String> = None;
                                let controls = vec![
                                    Line::from(vec![
                                        Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
                                        Span::raw(" or "),
                                        Span::styled("Backspace", Style::default().fg(Color::Cyan)),
                                        Span::raw(" clears the input"),
                                    ]),
                                    Line::from(format!("Enter password for \"{}\".", network.ssid)),
                                ];
                                loop {
                                    let info = if let Some(error_message) = &password_error {
                                        vec![Line::from(Span::styled(
                                            error_message,
                                            Style::default().fg(Color::Red),
                                        ))]
                                    } else {
                                        vec![Line::from("Press Enter to connect.")]
                                    };
                                    let summary = build_install_summary(
                                        step,
                                        include_drivers,
                                        network_label.as_deref(),
                                        selected_disk.as_ref(),
                                        &keymap,
                                        &timezone,
                                        &hostname,
                                        &username,
                                        &user_password,
                                        &luks_password,
                                        encrypt_disk,
                                        swap_enabled,
                                        nvidia_variant,
                                    );
                                    match run_text_input(
                                        &mut terminal,
                                        "Wi-Fi password",
                                        &controls,
                                        &info,
                                        "Wi-Fi password",
                                        None,
                                        true,
                                        &summary,
                                    )? {
                                        InputAction::Submit(value) => {
                                            if value.is_empty() {
                                                continue;
                                            }
                                            let start = Instant::now();
                                            let spinner = SPINNER[0];
                                            let connecting_info = vec![Line::from(Span::styled(
                                                format!("Connecting... {} (starting)", spinner),
                                                Style::default().fg(Color::Green),
                                            ))];
                                            render_text_input(
                                                &mut terminal,
                                                "Wi-Fi password",
                                                &controls,
                                                &connecting_info,
                                                "Wi-Fi password",
                                                &value,
                                                true,
                                                &summary,
                                            )?;
                                            let _ = disconnect_wifi_device();
                                            let _ = forget_wifi_connection(&network.ssid);
                                            let device = wifi_device_name().ok().flatten();
                                            let connection_name =
                                                format!("nebula-{}", network.ssid);
                                            match connect_wifi_profile(
                                                &network.ssid,
                                                Some(&value),
                                                device.as_deref(),
                                                Some(&connection_name),
                                            ) {
                                                Ok(()) => {
                                                    while start.elapsed() < Duration::from_secs(8) {
                                                        let spinner_idx =
                                                            (start.elapsed().as_millis() / 200)
                                                                % SPINNER_LEN as u128;
                                                        let spinner = SPINNER[spinner_idx as usize];
                                                        let state = wifi_device_state()
                                                            .ok()
                                                            .flatten()
                                                            .unwrap_or_else(|| {
                                                                "unknown".to_string()
                                                            });
                                                        let connecting_info =
                                                            vec![Line::from(Span::styled(
                                                                format!(
                                                                    "Connecting... {} ({})",
                                                                    spinner, state
                                                                ),
                                                                Style::default().fg(Color::Green),
                                                            ))];
                                                        render_text_input(
                                                            &mut terminal,
                                                            "Wi-Fi password",
                                                            &controls,
                                                            &connecting_info,
                                                            "Wi-Fi password",
                                                            &value,
                                                            true,
                                                            &summary,
                                                        )?;
                                                        if is_wifi_connected().unwrap_or(false) {
                                                            password = Some(value);
                                                            wifi_connected = true;
                                                            last_connect_at = Some(Instant::now());
                                                            break;
                                                        }
                                                        std::thread::sleep(Duration::from_millis(
                                                            200,
                                                        ));
                                                    }
                                                    if password.is_some() {
                                                        break;
                                                    }
                                                    let state = wifi_device_state()
                                                        .ok()
                                                        .flatten()
                                                        .unwrap_or_else(|| "unknown".to_string());
                                                    password_error = Some(format!(
                                                        "Connection failed (state: {}). Please try again.",
                                                        state
                                                    ));
                                                    continue;
                                                }
                                                Err(err) => {
                                                    let err_msg = err.to_string();
                                                    if is_wifi_auth_error(&err_msg) {
                                                        password_error =
                                                            Some("Incorrect password.".to_string());
                                                        let _ =
                                                            forget_wifi_connection(&network.ssid);
                                                        continue;
                                                    }
                                                    status_message = Some(err_msg);
                                                    break;
                                                }
                                            }
                                        }
                                        InputAction::Back => break,
                                        InputAction::Quit => {
                                            disable_raw_mode().context("disable raw mode")?;
                                            let _ = clear_screen();
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                            if needs_password && password.is_none() {
                                continue;
                            }
                            if network.is_open() {
                                let _ = disconnect_wifi_device();
                                let _ = forget_wifi_connection(&network.ssid);
                                let device = wifi_device_name().ok().flatten();
                                let connection_name = format!("nebula-{}", network.ssid);
                                if let Err(err) = connect_wifi_profile(
                                    &network.ssid,
                                    None,
                                    device.as_deref(),
                                    Some(&connection_name),
                                ) {
                                    status_message = Some(err.to_string());
                                    continue;
                                }
                                let start = Instant::now();
                                while start.elapsed() < Duration::from_secs(8) {
                                    let spinner_idx =
                                        (start.elapsed().as_millis() / 200) % SPINNER_LEN as u128;
                                    let spinner = SPINNER[spinner_idx as usize];
                                    let summary = build_install_summary(
                                        step,
                                        include_drivers,
                                        network_label.as_deref(),
                                        selected_disk.as_ref(),
                                        &keymap,
                                        &timezone,
                                        &hostname,
                                        &username,
                                        &user_password,
                                        &luks_password,
                                        encrypt_disk,
                                        swap_enabled,
                                        nvidia_variant,
                                    );
                                    render_wifi_connecting(
                                        &mut terminal,
                                        index,
                                        &networks,
                                        status_message.as_deref(),
                                        wifi_connected,
                                        internet_ready,
                                        &summary,
                                        spinner,
                                    )?;
                                    if is_wifi_connected().unwrap_or(false) {
                                        wifi_connected = true;
                                        last_connect_at = Some(Instant::now());
                                        break;
                                    }
                                    std::thread::sleep(Duration::from_millis(200));
                                }
                                if !wifi_connected {
                                    status_message =
                                        Some("Connection failed. Please try again.".to_string());
                                    continue;
                                }
                            }
                            internet_ready = is_network_ready().unwrap_or(false);
                            if internet_ready {
                                network_label = active_connection_label().ok().flatten();
                                if network_label.is_none() {
                                    network_label = Some(network.ssid.clone());
                                }
                                status_message = None;
                            } else {
                                status_message =
                                    Some("Connected to Wi-Fi but no internet access.".to_string());
                            }
                            continue;
                        }
                        WifiAction::Rescan => {
                            status_message = None;
                        }
                        WifiAction::Refresh => {} // No-op, handled by loop
                        WifiAction::Continue => {
                            if internet_ready {
                                if gpu_vendors.contains(&GpuVendor::Nvidia) {
                                    step = SetupStep::Drivers;
                                } else {
                                    step = SetupStep::Disk;
                                }
                                break;
                            }
                        }
                        WifiAction::Quit => {
                            disable_raw_mode().context("disable raw mode")?;
                            let _ = clear_screen();
                            return Ok(());
                        }
                    }
                }
            }
            SetupStep::Disk => {
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_disk_selector(&mut terminal, &disks, 0, &summary)? {
                    SelectionAction::Submit(index) => {
                        selected_disk = disks.get(index).cloned();
                        step = SetupStep::ConfirmDisk;
                    }
                    SelectionAction::Back => {
                        if gpu_vendors.contains(&GpuVendor::Nvidia) {
                            step = SetupStep::Drivers;
                        } else {
                            force_network = true;
                            step = SetupStep::Network;
                        }
                    }
                    SelectionAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::ConfirmDisk => {
                let Some(disk) = &selected_disk else {
                    step = SetupStep::Disk;
                    continue;
                };
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                let warning_lines = vec![
                    Line::from(Span::styled(
                        "This will ERASE the selected disk:",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(vec![
                        Span::styled(" ", Style::default().fg(Color::White)),
                        Span::styled(" 󰋊  ", Style::default().fg(Color::LightBlue)),
                        Span::styled(disk.label(), Style::default().add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(""),
                ];
                let info_lines = vec![
                    Line::from(Span::styled(
                        "All data on this disk will be lost. This action cannot be undone.",
                        Style::default().fg(Color::Magenta),
                    )),
                    Line::from(Span::styled(
                        "Choose Yes to continue or No to go back",
                        Style::default().fg(Color::White),
                    )),
                ];
                match run_confirm_selector(
                    &mut terminal,
                    "Confirm disk erase",
                    &warning_lines,
                    &info_lines,
                    &summary,
                )? {
                    ConfirmAction::Yes => step = SetupStep::Keymap,
                    ConfirmAction::No => step = SetupStep::Disk,
                    ConfirmAction::Back => step = SetupStep::Disk,
                    ConfirmAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::Keymap => {
                let initial = find_keymap_index(&keymaps, &keymap).unwrap_or(0);
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_keymap_selector(&mut terminal, &keymaps, initial, &summary)? {
                    SelectionAction::Submit(index) => {
                        if let Some(value) = keymaps.get(index) {
                            keymap = value.to_string();
                        }
                        step = SetupStep::Timezone;
                    }
                    SelectionAction::Back => step = SetupStep::ConfirmDisk,
                    SelectionAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::Timezone => {
                if timezone.is_empty() || is_utc_variant(&timezone) {
                    if std::env::var("NEBULA_SKIP_NETWORK").ok().as_deref() != Some("1")
                        && std::env::var("NEBULA_OFFLINE_ONLY").ok().as_deref() != Some("1")
                    {
                        render_timezone_loading(
                            &mut terminal,
                            &build_install_summary(
                                step,
                                include_drivers,
                                network_label.as_deref(),
                                selected_disk.as_ref(),
                                &keymap,
                                &timezone,
                                &hostname,
                                &username,
                                &user_password,
                                &luks_password,
                                encrypt_disk,
                                swap_enabled,
                                nvidia_variant,
                            ),
                        )?;
                    }
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("/run/nebula/timezone-detect.log")
                        .and_then(|mut file| {
                            use std::io::Write;
                            writeln!(file, "detect_timezone: retry at timezone step")
                        });
                    if let Some(value) = detect_timezone_geoip(&timezones) {
                        timezone = value;
                    }
                }
                let initial = find_timezone_index(&timezones, &timezone).unwrap_or(0);
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_timezone_selector(&mut terminal, &timezones, initial, &summary)? {
                    SelectionAction::Submit(index) => {
                        if let Some(value) = timezones.get(index) {
                            timezone = value.to_string();
                        }
                        step = SetupStep::Hostname;
                    }
                    SelectionAction::Back => step = SetupStep::Keymap,
                    SelectionAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::Hostname => {
                let controls = vec![
                    Line::from(vec![
                        Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
                        Span::raw(" or "),
                        Span::styled("Backspace", Style::default().fg(Color::Cyan)),
                        Span::raw(" clears the input "),
                        Span::styled("Esc", Style::default().fg(Color::Cyan)),
                        Span::raw(" to go back"),
                    ]),
                    Line::from("Type to enter a hostname"),
                ];
                let info = vec![
                    Line::from("Enter hostname (letters, numbers, and hyphens)"),
                    Line::from("Example: my-hostname"),
                ];
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_text_input(
                    &mut terminal,
                    "Hostname",
                    &controls,
                    &info,
                    "Hostname",
                    Some(&hostname),
                    false,
                    &summary,
                )? {
                    InputAction::Submit(value) => {
                        let value = value.trim();
                        if value.is_empty() {
                            hostname = "nebula".to_string();
                            step = SetupStep::Username;
                        } else if valid_hostname(value) {
                            hostname = value.to_string();
                            step = SetupStep::Username;
                        }
                    }
                    InputAction::Back => step = SetupStep::Timezone,
                    InputAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::Username => {
                let controls = vec![
                    Line::from(vec![
                        Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
                        Span::raw(" or "),
                        Span::styled("Backspace", Style::default().fg(Color::Cyan)),
                        Span::raw(" clears the input "),
                        Span::styled("Esc", Style::default().fg(Color::Cyan)),
                        Span::raw(" to go back"),
                    ]),
                    Line::from("Type to enter your username"),
                ];
                let info = vec![
                    Line::from("Use lowercase letters, numbers, and hyphens only"),
                    Line::from("Example: kevin"),
                ];
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_text_input(
                    &mut terminal,
                    "User account",
                    &controls,
                    &info,
                    "Username",
                    Some(&username),
                    false,
                    &summary,
                )? {
                    InputAction::Submit(value) => {
                        let value = value.trim();
                        if valid_username(value) {
                            username = value.to_string();
                            step = SetupStep::UserPassword;
                        }
                    }
                    InputAction::Back => step = SetupStep::Hostname,
                    InputAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::UserPassword => {
                let controls = vec![
                    Line::from(vec![
                        Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
                        Span::raw(" or "),
                        Span::styled("Backspace", Style::default().fg(Color::Cyan)),
                        Span::raw(" clears the input "),
                        Span::styled("Esc", Style::default().fg(Color::Cyan)),
                        Span::raw(" to go back"),
                    ]),
                    Line::from("Type to enter your password"),
                ];
                let info = vec![
                    Line::from("Set a password for the sudo user"),
                    Line::from("Press Enter to submit"),
                ];
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_text_input(
                    &mut terminal,
                    "User password",
                    &controls,
                    &info,
                    "Password",
                    None,
                    true,
                    &summary,
                )? {
                    InputAction::Submit(value) => {
                        if value.is_empty() {
                            continue;
                        }
                        let confirm_controls = vec![
                            Line::from(vec![
                                Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
                                Span::raw(" or "),
                                Span::styled("Backspace", Style::default().fg(Color::Cyan)),
                                Span::raw(" clears the input "),
                                Span::styled("Esc", Style::default().fg(Color::Cyan)),
                                Span::raw(" to go back"),
                            ]),
                            Line::from("Type to confirm your password"),
                        ];
                        let confirm_info = vec![Line::from("Re-enter the password to confirm")];
                        let summary = build_install_summary(
                            step,
                            include_drivers,
                            network_label.as_deref(),
                            selected_disk.as_ref(),
                            &keymap,
                            &timezone,
                            &hostname,
                            &username,
                            &user_password,
                            &luks_password,
                            encrypt_disk,
                            swap_enabled,
                            nvidia_variant,
                        );
                        match run_text_input(
                            &mut terminal,
                            "Confirm password",
                            &confirm_controls,
                            &confirm_info,
                            "Re-enter password",
                            None,
                            true,
                            &summary,
                        )? {
                            InputAction::Submit(confirm) => {
                                if confirm == value {
                                    user_password = value;
                                    step = SetupStep::EncryptDisk;
                                }
                            }
                            InputAction::Back => {} // Handled by outer match
                            InputAction::Quit => {
                                disable_raw_mode().context("disable raw mode")?;
                                let _ = clear_screen();
                                return Ok(());
                            }
                        }
                    }
                    InputAction::Back => step = SetupStep::Username,
                    InputAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::EncryptDisk => {
                let info_lines = vec![
                    Line::from("Encrypt the disk with a LUKS passphrase"),
                    Line::from("Highly recommended to protect your data at rest"),
                    Line::from("Choose Yes to set a passphrase or No to skip"),
                ];
                let warning_lines: Vec<Line> = Vec::new();
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_confirm_selector(
                    &mut terminal,
                    "Disk encryption",
                    &warning_lines,
                    &info_lines,
                    &summary,
                )? {
                    ConfirmAction::Yes => {
                        encrypt_disk = true;
                        step = SetupStep::LuksPassword;
                    }
                    ConfirmAction::No => {
                        encrypt_disk = false;
                        luks_password.clear();
                        step = SetupStep::Swap;
                    }
                    ConfirmAction::Back => step = SetupStep::UserPassword,
                    ConfirmAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::LuksPassword => {
                encrypt_disk = true;
                let controls = vec![
                    Line::from(vec![
                        Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
                        Span::raw(" or "),
                        Span::styled("Backspace", Style::default().fg(Color::Cyan)),
                        Span::raw(" clears the input "),
                        Span::styled("Esc", Style::default().fg(Color::Cyan)),
                        Span::raw(" to go back"),
                    ]),
                    Line::from("Type to enter the disk passphrase"),
                ];
                let info = vec![
                    Line::from("Set a disk encryption passphrase"),
                    Line::from("This unlocks your system at boot"),
                ];
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_text_input(
                    &mut terminal,
                    "Disk encryption passphrase",
                    &controls,
                    &info,
                    "Encryption passphras",
                    None,
                    true,
                    &summary,
                )? {
                    InputAction::Submit(value) => {
                        if value.is_empty() {
                            continue;
                        }
                        let confirm_controls = vec![
                            Line::from(vec![
                                Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
                                Span::raw(" or "),
                                Span::styled("Backspace", Style::default().fg(Color::Cyan)),
                                Span::raw(" clears the input "),
                                Span::styled("Esc", Style::default().fg(Color::Cyan)),
                                Span::raw(" to go back"),
                            ]),
                            Line::from("Type to confirm the passphrase"),
                        ];
                        let confirm_info = vec![Line::from("Re-enter the passphrase to confirm")];
                        let summary = build_install_summary(
                            step,
                            include_drivers,
                            network_label.as_deref(),
                            selected_disk.as_ref(),
                            &keymap,
                            &timezone,
                            &hostname,
                            &username,
                            &user_password,
                            &luks_password,
                            encrypt_disk,
                            swap_enabled,
                            nvidia_variant,
                        );
                        match run_text_input(
                            &mut terminal,
                            "Confirm passphrase",
                            &confirm_controls,
                            &confirm_info,
                            "Re-enter encryption passphras",
                            None,
                            true,
                            &summary,
                        )? {
                            InputAction::Submit(confirm) => {
                                if confirm == value {
                                    luks_password = value;
                                    step = SetupStep::Swap;
                                }
                            }
                            InputAction::Back => {} // Handled by outer match
                            InputAction::Quit => {
                                disable_raw_mode().context("disable raw mode")?;
                                let _ = clear_screen();
                                return Ok(());
                            }
                        }
                    }
                    InputAction::Back => step = SetupStep::EncryptDisk,
                    InputAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::Drivers => {
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_nvidia_selector(&mut terminal, &summary)? {
                    NvidiaAction::Select(variant) => {
                        nvidia_variant = Some(variant);
                        step = SetupStep::Disk;
                    }
                    NvidiaAction::Skip => {
                        nvidia_variant = None;
                        step = SetupStep::Disk;
                    }
                    NvidiaAction::Back => {
                        force_network = has_wifi_device().unwrap_or(false);
                        step = SetupStep::Network;
                    }
                    NvidiaAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::Swap => {
                let info_lines = vec![
                    Line::from("Enable zram-based swap (in-memory compressed)"),
                    Line::from("Recommended to improve responsiveness under memory pressure"),
                ];
                let warning_lines: Vec<Line> = Vec::new();
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_confirm_selector(
                    &mut terminal,
                    "Enable swap",
                    &warning_lines,
                    &info_lines,
                    &summary,
                )? {
                    ConfirmAction::Yes => {
                        swap_enabled = true;
                        step = SetupStep::Applications;
                    }
                    ConfirmAction::No => {
                        swap_enabled = false;
                        step = SetupStep::Applications;
                    }
                    ConfirmAction::Back => {
                        if encrypt_disk {
                            step = SetupStep::LuksPassword;
                        } else {
                            step = SetupStep::EncryptDisk;
                        }
                    }
                    ConfirmAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::Applications => {
                let summary = build_install_summary(
                    step,
                    include_drivers,
                    network_label.as_deref(),
                    selected_disk.as_ref(),
                    &keymap,
                    &timezone,
                    &hostname,
                    &username,
                    &user_password,
                    &luks_password,
                    encrypt_disk,
                    swap_enabled,
                    nvidia_variant,
                );
                match run_application_selector(&mut terminal, &app_flags, &summary)? {
                    SelectionAction::Submit(flags) => {
                        app_flags = flags;
                        app_selection = selection_from_app_flags(&app_flags);
                        step = SetupStep::Review;
                    }
                    SelectionAction::Back => step = SetupStep::Swap,
                    SelectionAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
            SetupStep::Review => {
                let Some(disk) = &selected_disk else {
                    step = SetupStep::Disk;
                    continue;
                };
                let compositor_labels =
                    labels_for_flags(&app_flags.compositors, &COMPOSITOR_LABELS);
                let browser_labels = labels_for_selection(&app_selection, &BROWSER_CHOICES);
                let editor_labels = labels_for_selection(&app_selection, &EDITOR_CHOICES);
                let terminal_labels = labels_for_selection(&app_selection, &TERMINAL_CHOICES);
                let system_items = vec![
                    ReviewItem {
                        label: "Network".to_string(),
                        value: network_label
                            .clone()
                            .unwrap_or_else(|| "Not connected".to_string()),
                    },
                    ReviewItem {
                        label: "Disk".to_string(),
                        value: disk.label(),
                    },
                    ReviewItem {
                        label: "Filesystem".to_string(),
                        value: if encrypt_disk {
                            "Btrfs (LUKS encrypted)".to_string()
                        } else {
                            "Btrfs".to_string()
                        },
                    },
                    ReviewItem {
                        label: "GPU".to_string(),
                        value: format_gpu_summary(&gpu_vendors, nvidia_variant)
                            .unwrap_or_else(|| "Not detected".to_string()),
                    },
                    ReviewItem {
                        label: "Swap".to_string(),
                        value: if swap_enabled {
                            "Enabled (zram)".to_string()
                        } else {
                            "Disabled".to_string()
                        },
                    },
                    ReviewItem {
                        label: "Hostname".to_string(),
                        value: hostname.clone(),
                    },
                    ReviewItem {
                        label: "Username".to_string(),
                        value: username.clone(),
                    },
                    ReviewItem {
                        label: "Keyboard".to_string(),
                        value: keymap.clone(),
                    },
                    ReviewItem {
                        label: "Timezone".to_string(),
                        value: timezone.clone(),
                    },
                ];
                let package_items = vec![
                    ReviewItem {
                        label: "Compositor".to_string(),
                        value: if compositor_labels.is_empty() {
                            "None".to_string()
                        } else {
                            compositor_labels.join(", ")
                        },
                    },
                    ReviewItem {
                        label: "Browsers".to_string(),
                        value: if browser_labels.is_empty() {
                            "None".to_string()
                        } else {
                            browser_labels.join(", ")
                        },
                    },
                    ReviewItem {
                        label: "Editors".to_string(),
                        value: if editor_labels.is_empty() {
                            "None".to_string()
                        } else {
                            editor_labels.join(", ")
                        },
                    },
                    ReviewItem {
                        label: "Terminals".to_string(),
                        value: if terminal_labels.is_empty() {
                            "None".to_string()
                        } else {
                            terminal_labels.join(", ")
                        },
                    },
                ];
                let selected_packages = compositor_labels.len()
                    + browser_labels.len()
                    + editor_labels.len()
                    + terminal_labels.len();
                match run_review(
                    &mut terminal,
                    &system_items,
                    &package_items,
                    selected_packages,
                )? {
                    ReviewAction::Confirm => break 'setup,
                    ReviewAction::Back => step = SetupStep::Applications,
                    ReviewAction::Edit => step = SetupStep::Network,
                    ReviewAction::Quit => {
                        disable_raw_mode().context("disable raw mode")?;
                        let _ = clear_screen();
                        return Ok(());
                    }
                }
            }
        }
    }

    // Create the installation configuration
    let config = InstallConfig {
        disk: selected_disk.expect("disk selection"),
        keymap,
        timezone,
        hostname,
        username,
        user_password,
        luks_password,
        encrypt_disk,
        swap_enabled,
        driver_packages: driver_packages(&gpu_vendors, nvidia_variant),
        kernel_package,
        kernel_headers,
        base_packages,
        extra_pacman_packages: app_selection.pacman,
        extra_aur_packages: app_selection.yay,
        offline_only,
        hyprland_selected: true,
    };

    let (tx, rx) = crossbeam_channel::unbounded();
    let installer_tx = tx.clone();
    thread::spawn(move || {
        if let Err(err) = run_installer(installer_tx, &config) {
            let _ = tx.send(InstallerEvent::Done(Some(err.to_string())));
        }
    });

    // Set up the UI for the installation progress screen
    clear_screen()?;
    let step_names: Vec<String> = STEP_NAMES.iter().map(|name| (*name).to_string()).collect();

    let logs = VecDeque::from(vec!["Starting nebula installer...".to_string()]);
    let log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(LOG_FILE_PATH)
        .ok();

    let mut app = App {
        steps: step_names
            .iter()
            .map(|name| Step {
                name: name.to_string(),
                status: StepStatus::Pending,
                err: None,
            })
            .collect(),
        progress: 0.0,
        logs,
        spinner_idx: 0,
        done: false,
        err: None,
        log_file,
    };
    if app.log_file.is_some() {
        let line = format!("Logging to {}", LOG_FILE_PATH);
        push_log(&mut app.logs, line.clone());
        append_log_file(&mut app.log_file, &line);
    }

    terminal.clear().context("clear terminal")?;
    terminal.draw(|f| draw_ui(f.size(), f, &app))?;

    // Installation progress screen
    let mut last_tick = Instant::now();
    let mut reboot_requested = false;
    let mut shutdown_requested = false;
    loop {
        terminal.draw(|f| draw_ui(f.size(), f, &app))?;

        let timeout = Duration::from_millis(100);
        if event::poll(timeout).context("poll events")? {
            if let Event::Key(key) = event::read().context("read event")? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            break
                        }
                        KeyCode::Char('r') | KeyCode::Char('R')
                            if app.done && app.err.is_none() =>
                        {
                            reboot_requested = true;
                            break;
                        }
                        KeyCode::Char('s') | KeyCode::Char('S')
                            if app.done && app.err.is_none() =>
                        {
                            shutdown_requested = true;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        while let Ok(evt) = rx.try_recv() {
            handle_event(&mut app, evt);
        }

        // Update the spinner animation
        if last_tick.elapsed() >= Duration::from_millis(120) {
            app.spinner_idx = (app.spinner_idx + 1) % SPINNER_LEN;
            last_tick = Instant::now();
        }
    }

    // Clean up the terminal before exiting
    disable_raw_mode().context("disable raw mode")?;
    let _ = clear_screen();
    if reboot_requested {
        Command::new("systemctl")
            .arg("reboot")
            .status()
            .context("reboot system")?;
    } else if shutdown_requested {
        Command::new("systemctl")
            .arg("poweroff")
            .status()
            .context("power off system")?;
    }
    Ok(())
}

// Clear the terminal screen
fn clear_screen() -> Result<()> {
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0)).context("clear screen")?;
    Ok(())
}

fn handle_event(app: &mut App, evt: InstallerEvent) {
    match evt {
        InstallerEvent::Log(line) => {
            push_log(&mut app.logs, line.clone());
            append_log_file(&mut app.log_file, &line);
        }
        InstallerEvent::Progress(value) => app.progress = value,
        InstallerEvent::Step { index, status, err } => {
            if let Some(step) = app.steps.get_mut(index) {
                step.status = status;
                step.err = err.clone();
                let status_label = match step.status {
                    StepStatus::Pending => "PENDING",
                    StepStatus::Running => "RUNNING",
                    StepStatus::Done => "OK",
                    StepStatus::Skipped => "SKIP",
                    StepStatus::Failed => "FAIL",
                };
                let line = format!("STEP {}: {}", step.name, status_label);
                append_log_file(&mut app.log_file, &line);
                if let Some(err) = err {
                    append_log_file(&mut app.log_file, &format!("ERROR: {}", err));
                }
            }
        }
        InstallerEvent::Done(err) => {
            app.done = true;
            app.err = err.clone();
            if let Some(err) = err {
                append_log_file(&mut app.log_file, &format!("DONE: {}", err));
            } else {
                append_log_file(&mut app.log_file, "DONE: ok");
                if Path::new("/mnt/var/log/nebula-failed-packages.txt").exists() {
                    let line = "Optional packages failed. See /var/log/nebula-failed-packages.txt on the installed system.";
                    push_log(&mut app.logs, line.to_string());
                    append_log_file(&mut app.log_file, line);
                }
            }
        }
    }
}

// New log line
fn push_log(logs: &mut VecDeque<String>, line: String) {
    if logs.len() >= LOG_CAPACITY {
        logs.pop_front();
    }
    logs.push_back(line);
}

fn append_log_file(log_file: &mut Option<std::fs::File>, line: &str) {
    if let Some(file) = log_file.as_mut() {
        let _ = writeln!(file, "{}", line);
        let _ = file.flush();
    }
}

fn valid_username(value: &str) -> bool {
    if value.is_empty() || value == "root" {
        return false;
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

// Validates a hostname
fn valid_hostname(value: &str) -> bool {
    if value.is_empty() || value.len() > 63 {
        return false;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

// Checks if an error message indicates a Wi-Fi authentication failure
fn is_wifi_auth_error(message: &str) -> bool {
    let msg = message.to_lowercase();
    msg.contains("password")
        || msg.contains("secrets")
        || msg.contains("auth")
        || msg.contains("authentication")
        || msg.contains("access denied")
}
