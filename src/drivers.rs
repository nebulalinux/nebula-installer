// Detecting GPU hardware on the system
use std::collections::HashSet;
use std::fs;
use std::process::Command;

use anyhow::Result;

// GPU manufacturers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuVendor {
    Amd,
    Intel,
    Nvidia,
}

// Driver options for NVIDIA GPUs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvidiaVariant {
    Open,        // Open-source kernel module (for newer cards)
    Proprietary, // Nvidia's proprietary driver
    Nouveau,     // Open-source Nouveau driver
}

// Detects the GPU vendors present in the system
pub fn detect_gpu_vendors() -> Result<HashSet<GpuVendor>> {
    let mut vendors = HashSet::new();
    if let Some(dev_override) = dev_gpu_override() {
        vendors.extend(dev_override);
        return Ok(vendors);
    }

    // Attempt detection via /sys/class/drm
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("card") {
                continue;
            }
            let vendor_path = entry.path().join("device/vendor");
            if let Ok(contents) = fs::read_to_string(vendor_path) {
                if let Some(vendor) = parse_vendor_id(contents.trim()) {
                    vendors.insert(vendor);
                }
            }
        }
    }

    // Fallback detection using lspci
    if vendors.is_empty() {
        if let Ok(output) = Command::new("lspci").arg("-nn").output() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if !is_gpu_line(line) {
                    continue;
                }
                if let Some(vendor_id) = parse_vendor_from_lspci(line) {
                    if let Some(vendor) = parse_vendor_id(&vendor_id) {
                        vendors.insert(vendor);
                    }
                }
            }
        }
    }

    Ok(vendors)
}

fn dev_gpu_override() -> Option<HashSet<GpuVendor>> {
    let value = std::env::var("NEBULA_DEV_GPU").ok()?;
    if value.trim().is_empty() {
        return None;
    }
    let mut vendors = HashSet::new();
    for entry in value.split(',') {
        match entry.trim().to_ascii_lowercase().as_str() {
            "amd" => {
                vendors.insert(GpuVendor::Amd);
            }
            "intel" => {
                vendors.insert(GpuVendor::Intel);
            }
            "nvidia" => {
                vendors.insert(GpuVendor::Nvidia);
            }
            _ => {}
        }
    }
    if vendors.is_empty() {
        None
    } else {
        Some(vendors)
    }
}

// Returns a list of recommended driver packages based on detected GPUs and Nvidia variant choice
pub fn driver_packages(
    vendors: &HashSet<GpuVendor>,
    nvidia_variant: Option<NvidiaVariant>,
) -> Vec<String> {
    let mut packages = Vec::new();

    if vendors.contains(&GpuVendor::Amd) {
        extend_unique(
            &mut packages,
            &[
                "mesa",
                "vulkan-radeon",
                "xf86-video-amdgpu",
                "xf86-video-ati",
            ],
        );
    }
    if vendors.contains(&GpuVendor::Intel) {
        extend_unique(
            &mut packages,
            &[
                "intel-media-driver",
                "libva-intel-driver",
                "mesa",
                "vulkan-intel",
            ],
        );
    }
    if vendors.contains(&GpuVendor::Nvidia) {
        if let Some(variant) = nvidia_variant {
            match variant {
                NvidiaVariant::Open => extend_unique(
                    &mut packages,
                    &["dkms", "libva-nvidia-driver", "nvidia-open-dkms"],
                ),
                NvidiaVariant::Proprietary => extend_unique(
                    &mut packages,
                    &["dkms", "libva-nvidia-driver", "nvidia-dkms"],
                ),
                NvidiaVariant::Nouveau => extend_unique(
                    &mut packages,
                    &["mesa", "vulkan-nouveau", "xf86-video-nouveau"],
                ),
            }
        }
    }
    packages
}

// Summary of detected GPUs and the chosen Nvidia driver
pub fn format_gpu_summary(
    vendors: &HashSet<GpuVendor>,
    nvidia_variant: Option<NvidiaVariant>,
) -> Option<String> {
    if vendors.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if vendors.contains(&GpuVendor::Amd) {
        parts.push("AMD");
    }
    if vendors.contains(&GpuVendor::Intel) {
        parts.push("Intel");
    }
    if vendors.contains(&GpuVendor::Nvidia) {
        parts.push("NVIDIA");
    }
    let mut line = format!("Detected GPU: {}", parts.join(", "));
    if let Some(variant) = nvidia_variant {
        line.push_str(&format!(
            " (NVIDIA driver: {})",
            nvidia_variant_label(variant)
        ));
    }
    Some(line)
}

// Nvidia driver variant
pub fn nvidia_variant_label(variant: NvidiaVariant) -> &'static str {
    match variant {
        NvidiaVariant::Open => "open",
        NvidiaVariant::Proprietary => "proprietary",
        NvidiaVariant::Nouveau => "nouveau",
    }
}

// Parses a hexadecimal vendor ID string into a GpuVendor enum
fn parse_vendor_id(value: &str) -> Option<GpuVendor> {
    let trimmed = value.trim().trim_start_matches("0x");
    match trimmed.to_ascii_lowercase().as_str() {
        "1002" => Some(GpuVendor::Amd),
        "8086" => Some(GpuVendor::Intel),
        "10de" => Some(GpuVendor::Nvidia),
        _ => None,
    }
}

// Checks if a given `lspci` output line describes a GPU
fn is_gpu_line(line: &str) -> bool {
    line.contains("VGA compatible controller")
        || line.contains("3D controller")
        || line.contains("Display controller")
}

// Extracts the vendor ID from an `lspci` output line
fn parse_vendor_from_lspci(line: &str) -> Option<String> {
    for part in line.split('[').skip(1) {
        let candidate = part.split(':').next()?;
        if candidate.len() == 4 && candidate.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(candidate.to_ascii_lowercase());
        }
    }
    None
}

// Add new elements to a vector only if they are not already present
fn extend_unique(target: &mut Vec<String>, values: &[&str]) {
    for value in values {
        if !target.iter().any(|existing| existing == value) {
            target.push((*value).to_string());
        }
    }
}
