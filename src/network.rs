use anyhow::{Context, Result};
use std::process::Command;

// Detected Wi-Fi network
#[derive(Clone, Debug)]
pub struct WifiNetwork {
    pub ssid: String,     // Service Set Identifier (network name)
    pub signal: u8,       // Signal strength in percentage
    pub security: String, // Security type (e.g., "WPA2")
    pub in_use: bool,     // Whether this network is currently connected
}

impl WifiNetwork {
    // Checks if the Wi-Fi network is open (unsecured)
    pub fn is_open(&self) -> bool {
        let security = self.security.trim();
        security.is_empty() || security == "--"
    }
}

// Current internet connectivity status
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connectivity {
    Full,    // Full internet access
    Limited, // Limited connectivity
    Portal,  // Requires captive portal login
    None,    // No connectivity
    Unknown, // Status could not be determined
}

// Queries `nmcli` to get the system's overall internet connectivity status
pub fn connectivity_status() -> Result<Connectivity> {
    let output = run_nmcli(&["-t", "-f", "CONNECTIVITY", "networking", "connectivity"])?;
    Ok(match output.trim() {
        "full" => Connectivity::Full,
        "limited" => Connectivity::Limited,
        "portal" => Connectivity::Portal,
        "none" => Connectivity::None,
        _ => Connectivity::Unknown,
    })
}

// Determines if the network is "ready" for installation
pub fn is_network_ready() -> Result<bool> {
    match connectivity_status()? {
        Connectivity::Full | Connectivity::Limited => Ok(true),
        Connectivity::Portal | Connectivity::None => Ok(false),
        Connectivity::Unknown => has_connected_device(), // Fallback if connectivity status is unknown
    }
}

// Currently active network connection
// For wired connections, it returns "Wired", for Wi-Fi, it returns the SSID
pub fn active_connection_label() -> Result<Option<String>> {
    let output = run_nmcli(&["-t", "-f", "TYPE,STATE,CONNECTION", "dev", "status"])?;
    for line in output.lines() {
        let mut parts = line.split(':');
        let conn_type = parts.next().unwrap_or("");
        let state = parts.next().unwrap_or("");
        let connection = parts.next().unwrap_or("").trim();
        if state != "connected" {
            continue;
        }
        let label = match conn_type {
            "ethernet" => "Wired",
            "wifi" => connection,
            _ => connection, // Use connection name for other types.
        };
        if !label.is_empty() {
            return Ok(Some(label.to_string()));
        }
    }
    Ok(None)
}

// Checks if any Wi-Fi devices available
pub fn has_wifi_device() -> Result<bool> {
    let output = run_nmcli(&["-t", "-f", "TYPE", "dev", "status"])?;
    Ok(output.lines().any(|line| line.trim() == "wifi"))
}

// Returns the first Wi-Fi device name, if present.
pub fn wifi_device_name() -> Result<Option<String>> {
    let output = run_nmcli(&["-t", "-f", "DEVICE,TYPE", "dev", "status"])?;
    for line in output.lines() {
        let mut parts = line.split(':');
        let device = parts.next().unwrap_or("").trim();
        let dev_type = parts.next().unwrap_or("").trim();
        if dev_type == "wifi" && !device.is_empty() {
            return Ok(Some(device.to_string()));
        }
    }
    Ok(None)
}

// Disconnects the Wi-Fi device to clear any stuck state.
pub fn disconnect_wifi_device() -> Result<()> {
    if let Some(device) = wifi_device_name()? {
        let _ = run_nmcli_status(&["dev", "disconnect", &device]);
    }
    Ok(())
}

// Checks if the Wi-Fi device reports a connected state.
pub fn is_wifi_connected() -> Result<bool> {
    let Some(device) = wifi_device_name()? else {
        return Ok(false);
    };
    let output = run_nmcli(&["-t", "-f", "DEVICE,STATE", "dev", "status"])?;
    for line in output.lines() {
        let mut parts = line.split(':');
        let dev = parts.next().unwrap_or("").trim();
        let state = parts.next().unwrap_or("").trim();
        if dev == device && state == "connected" {
            return Ok(true);
        }
    }
    Ok(false)
}

// Returns the Wi-Fi device state, if available.
pub fn wifi_device_state() -> Result<Option<String>> {
    let Some(device) = wifi_device_name()? else {
        return Ok(None);
    };
    let output = run_nmcli(&["-t", "-f", "DEVICE,STATE", "dev", "status"])?;
    for line in output.lines() {
        let mut parts = line.split(':');
        let dev = parts.next().unwrap_or("").trim();
        let state = parts.next().unwrap_or("").trim();
        if dev == device && !state.is_empty() {
            return Ok(Some(state.to_string()));
        }
    }
    Ok(None)
}

// Scans for and lists available Wi-Fi networks
pub fn list_wifi_networks() -> Result<Vec<WifiNetwork>> {
    // `nmcli dev wifi list --rescan yes` forces a rescan before listing
    let output = run_nmcli(&[
        "-t",
        "-f",
        "IN-USE,SSID,SIGNAL,SECURITY",
        "dev",
        "wifi",
        "list",
        "--rescan",
        "yes",
    ])?;
    let mut networks = Vec::new();
    for line in output.lines() {
        let mut parts = line.split(':');
        let in_use = parts.next().unwrap_or("").trim() == "*";
        let ssid = parts.next().unwrap_or("").trim();
        if ssid.is_empty() {
            continue;
        }
        let signal = parts
            .next()
            .unwrap_or("0")
            .trim()
            .parse::<u8>()
            .unwrap_or(0);
        let security = parts.next().unwrap_or("").trim().to_string();
        networks.push(WifiNetwork {
            ssid: ssid.to_string(),
            signal,
            security,
            in_use,
        });
    }
    networks.sort_by(|a, b| b.signal.cmp(&a.signal).then_with(|| a.ssid.cmp(&b.ssid)));
    Ok(networks)
}

// Connects to a specified Wi-Fi network
// Connects to a Wi-Fi network with an explicit connection profile and device (if provided).
pub fn connect_wifi_profile(
    ssid: &str,
    password: Option<&str>,
    device: Option<&str>,
    name: Option<&str>,
) -> Result<()> {
    let name = name.unwrap_or(ssid);
    let _ = run_nmcli_status(&["connection", "delete", "id", name]);
    let mut add_args = vec![
        "connection",
        "add",
        "type",
        "wifi",
        "con-name",
        name,
        "ssid",
        ssid,
    ];
    if let Some(device) = device {
        if !device.trim().is_empty() {
            add_args.push("ifname");
            add_args.push(device);
        }
    }
    run_nmcli_status(&add_args)?;
    if let Some(password) = password {
        if !password.trim().is_empty() {
            run_nmcli_status(&[
                "connection",
                "modify",
                name,
                "wifi-sec.key-mgmt",
                "wpa-psk",
                "wifi-sec.psk",
                password,
            ])?;
        }
    }
    run_nmcli_status(&["connection", "up", "id", name])
}

// Removes saved Wi-Fi connection profiles to avoid stale credentials
pub fn forget_wifi_connection(_ssid: &str) -> Result<()> {
    let output = run_nmcli(&["-t", "-f", "NAME,TYPE", "connection", "show"])?;
    for line in output.lines() {
        let mut parts = line.split(':');
        let name = parts.next().unwrap_or("").trim();
        let conn_type = parts.next().unwrap_or("").trim();
        if conn_type == "wifi" && !name.is_empty() {
            let _ = run_nmcli_status(&["connection", "delete", "id", name]);
        }
    }
    Ok(())
}

// Checks if any network device is currently in a "connected" state
fn has_connected_device() -> Result<bool> {
    let output = run_nmcli(&["-t", "-f", "DEVICE,TYPE,STATE", "dev", "status"])?;
    for line in output.lines() {
        let mut parts = line.split(':');
        let _device = parts.next().unwrap_or(""); // Device name (ignored)
        let _conn_type = parts.next().unwrap_or(""); // Connection type (ignored)
        let state = parts.next().unwrap_or(""); // Connection state
        if state == "connected" {
            return Ok(true);
        }
    }
    Ok(false)
}

// Run `nmcli` commands and return the standard output as a string
fn run_nmcli(args: &[&str]) -> Result<String> {
    let output = Command::new("nmcli")
        .args(args)
        .output()
        .with_context(|| format!("run nmcli {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if !stderr.is_empty() { stderr } else { stdout };
        anyhow::bail!("nmcli failed: {}", message);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// Run `nmcli` commands where only the success status is important
fn run_nmcli_status(args: &[&str]) -> Result<()> {
    let output = Command::new("nmcli")
        .args(args)
        .output()
        .with_context(|| format!("run nmcli {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if !stderr.is_empty() { stderr } else { stdout };
        anyhow::bail!("{}", message);
    }
    Ok(())
}
