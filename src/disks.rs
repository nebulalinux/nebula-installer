use anyhow::{Context, Result};
use std::process::Command;

#[derive(Clone, Debug)]
pub struct DiskInfo {
    pub name: String,
    pub size: String,
    pub model: String,
}

impl DiskInfo {
    pub fn device_path(&self) -> String {
        format!("/dev/{}", self.name)
    }

    pub fn partition_path(&self, index: u8) -> String {
        let needs_p = self
            .name
            .chars()
            .last()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
        if needs_p {
            format!("/dev/{}p{}", self.name, index)
        } else {
            format!("/dev/{}{}", self.name, index)
        }
    }

    pub fn label(&self) -> String {
        if self.model.is_empty() {
            format!("{} ({})", self.name, self.size)
        } else {
            format!("{} ({}) {}", self.name, self.size, self.model)
        }
    }
}

pub fn list_disks() -> Result<Vec<DiskInfo>> {
    let output = Command::new("lsblk")
        .args(["-dn", "-P", "-o", "NAME,SIZE,TYPE,MODEL"])
        .output()
        .context("lsblk")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("lsblk failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut disks = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_lsblk_kv(line);
        if fields.get("TYPE").map(|v| v.as_str()) != Some("disk") {
            continue;
        }
        let name = fields.get("NAME").cloned().unwrap_or_default();
        let size = fields.get("SIZE").cloned().unwrap_or_default();
        let model = fields.get("MODEL").cloned().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        disks.push(DiskInfo { name, size, model });
    }

    Ok(disks)
}

fn parse_lsblk_kv(line: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut rest = line.trim();
    while !rest.is_empty() {
        let Some(eq_idx) = rest.find("=\"") else {
            break;
        };
        let key = &rest[..eq_idx];
        let after_eq = &rest[eq_idx + 2..];
        let Some(end_quote) = after_eq.find('"') else {
            break;
        };
        let value = &after_eq[..end_quote];
        map.insert(key.to_string(), value.to_string());
        rest = after_eq[end_quote + 1..].trim_start();
    }
    map
}
