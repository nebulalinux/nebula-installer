use crate::config::config;

pub fn required_packages() -> Vec<String> {
    config().packages.required.clone()
}

pub fn hyprland_packages() -> Vec<String> {
    config().packages.hyprland.clone()
}
