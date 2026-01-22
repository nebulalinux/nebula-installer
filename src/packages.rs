use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PackageConfig {
    packages: PackageLists,
}

#[derive(Debug, Deserialize)]
struct PackageLists {
    required: Vec<String>,
    hyprland: Vec<String>,
}

static CONFIG: OnceLock<PackageConfig> = OnceLock::new();

fn config() -> &'static PackageConfig {
    CONFIG.get_or_init(|| {
        let raw = include_str!("../config.toml");
        toml::from_str(raw).expect("Invalid nebula-installer config.toml")
    })
}

pub fn required_packages() -> Vec<String> {
    config().packages.required.clone()
}

pub fn hyprland_packages() -> Vec<String> {
    config().packages.hyprland.clone()
}
