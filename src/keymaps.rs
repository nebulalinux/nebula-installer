use anyhow::Result;
use std::process::Command;

pub fn load_keymaps() -> Result<Vec<String>> {
    let output = Command::new("localectl").arg("list-keymaps").output();

    if let Ok(output) = output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut maps: Vec<String> = stdout
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect();
            maps.sort(); // Sort the keymaps alphabetically
            maps.dedup(); // Remove any duplicate entries
            if !maps.is_empty() {
                return Ok(maps);
            }
        }
    }

    // Fallback to "us" keymap if detection fails or yields no results
    Ok(vec!["us".to_string()])
}

// Returns `None` if the keymap is not found
pub fn find_keymap_index(maps: &[String], value: &str) -> Option<usize> {
    maps.iter().position(|map| map == value)
}
