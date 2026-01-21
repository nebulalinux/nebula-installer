const REQUIRED_PACKAGES: [&str; 51] = [
    "mesa",
    "dunst",
    "grim",
    "slurp",
    "gnome-themes-extra",
    "qt5-multimedia",
    "qt6-multimedia",
    "pipewire",
    "pipewire-alsa",
    "pipewire-audio",
    "pipewire-jack",
    "pipewire-pulse",
    "zsh",
    "networkmanager",
    "network-manager-applet",
    "bluez",
    "bluez-utils",
    "vim",
    "neovim",
    "htop",
    "kitty",
    "alacritty",
    "fastfetch",
    "exa",
    "foot",
    "yay",
    "nautilus",
    "gvfs",
    "gvfs-mtp",
    "noto-fonts",
    "noto-fonts-emoji",
    "ttf-cascadia-code-nerd",
    "ttf-cascadia-mono-nerd",
    "ttf-nerd-fonts-symbols",
    "sddm",
    "nebula-keybind-menu",
    "nebula-oh-my-zsh",
    "xdg-desktop-portal",
    "xdg-desktop-portal-hyprland",
    "xdg-desktop-portal-gtk",
    "xdg-utils",
    "xdg-user-dirs",
    "polkit-gnome",
    "wl-clipboard",
    "waybar",
    "wayland",
    "wayland-protocols",
    "qt5-wayland",
    "qt6-wayland",
    "rofi",
    "jq",
];

const HYPRLAND_PACKAGES: [&str; 10] = [
    "hyprland",
    "hyprlock",
    "hyprpicker",
    "hyprpaper",
    "hypridle",
    "hyprland-guiutils",
    "hyprsunset",
    "hyprutils",
    "hyprtoolkit",
    "nebula-hypr",
];

pub fn required_packages() -> Vec<String> {
    REQUIRED_PACKAGES
        .iter()
        .map(|pkg| (*pkg).to_string())
        .collect()
}

pub fn hyprland_packages() -> Vec<String> {
    HYPRLAND_PACKAGES
        .iter()
        .map(|pkg| (*pkg).to_string())
        .collect()
}
