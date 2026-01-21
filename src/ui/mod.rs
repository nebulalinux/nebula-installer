pub const SPINNER_LEN: usize = 4;
pub(crate) const SPINNER: [&str; SPINNER_LEN] = ["|", "/", "-", "\\"];
pub(crate) const NEBULA_ART: [&str; 6] = [
    " _   _      _           _       ",
    "| \\ | | ___| |__  _   _| | __ _ ",
    "|  \\| |/ _ \\ '_ \\| | | | |/ _` |",
    "| |\\  |  __/ |_) | |_| | | (_| |",
    "|_| \\_|\\___|_.__/ \\__,_|_|\\__,_|",
    "",
];

// A single item to be displayed on the final review screen
#[derive(Debug, Clone)]
pub struct ReviewItem {
    pub label: String,
    pub value: String,
}

// The number of steps shown in the summary view
pub const SUMMARY_STEP_COUNT: usize = 8;

// Display user's selections in the summary panel
#[derive(Debug, Clone)]
pub struct InstallSummary {
    pub current_index: usize,
    pub network: Option<String>,
    pub drivers: Option<String>,
    pub disk: Option<String>,
    pub keymap: Option<String>,
    pub timezone: Option<String>,
    pub hostname: Option<String>,
    pub username: Option<String>,
    pub encryption: Option<String>,
    pub zram_swap: Option<String>,
    pub include_drivers: bool,
}

// Actions the user can take on the review screen
pub enum ReviewAction {
    Confirm,
    Back,
    Edit,
    Quit,
}

// Actions for the NVIDIA driver selection screen
pub enum NvidiaAction {
    Select(crate::drivers::NvidiaVariant),
    Back,
    Skip,
    Quit,
}

// Generic actions for any selection screen (disk, keymap, timezone)
pub enum SelectionAction<T> {
    Submit(T),
    Back,
    Quit,
}

// Actions for text input screens (hostname, username, password)
pub enum InputAction {
    Submit(String),
    Back,
    Quit,
}

// Actions for confirmation screens (disk erase)
pub enum ConfirmAction {
    Yes,
    No,
    Back,
    Quit,
}

// Actions for the Wi-Fi selection screen
pub enum WifiAction {
    Submit(usize),
    Rescan,
    Refresh,
    Continue,
    Quit,
}

// Actions for the network required screen
pub enum NetworkAction {
    Retry,
    Quit,
}

// UI submodules
mod app_selection;
mod colors;
mod common;
mod confirm;
mod disk;
mod installer;
mod keybinds;
mod keymap;
mod network;
mod review;
mod selectors;
mod text_input;
mod timezone;
mod wifi;

pub use app_selection::run_application_selector;
pub use confirm::run_confirm_selector;
pub use disk::run_disk_selector;
pub use installer::draw_ui;
pub use keymap::run_keymap_selector;
pub use network::run_network_required;
pub use review::run_review;
#[allow(unused_imports)]
pub use selectors::run_nvidia_selector;
pub use text_input::{render_text_input, run_text_input};
pub use timezone::{render_timezone_loading, run_timezone_selector};
pub use wifi::render_wifi_connecting;
pub use wifi::{render_wifi_searching, run_wifi_selector};
