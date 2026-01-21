// Lists of packages to be installed
#[derive(Default, Clone)]
pub struct PackageSelection {
    pub pacman: Vec<String>,
    pub yay: Vec<String>,
}

impl PackageSelection {}

// Single installable application choice in the UI
pub struct InstallChoice {
    pub label: &'static str,             // The name displayed in the UI
    pub pacman: &'static [&'static str], // Pacman packages required for this choice
    pub yay: &'static [&'static str],    // Yay (AUR) packages required for this choice
}

// State of the checkboxes in the application selection screen
#[derive(Clone)]
pub struct AppSelectionFlags {
    pub compositors: Vec<bool>,
    pub browsers: Vec<bool>,
    pub editors: Vec<bool>,
    pub terminals: Vec<bool>,
}

impl AppSelectionFlags {
    // Creates a new set of application selection flags with default values
    pub fn new() -> Self {
        let mut compositors = vec![false; COMPOSITOR_LABELS.len()];
        if let Some(flag) = compositors.first_mut() {
            *flag = true;
        }
        Self {
            compositors,
            browsers: vec![false; BROWSER_CHOICES.len()],
            editors: vec![false; EDITOR_CHOICES.len()],
            terminals: vec![false; TERMINAL_CHOICES.len()],
        }
    }

    pub fn enforce_defaults(&mut self) {
        if let Some(flag) = self.compositors.first_mut() {
            *flag = true;
        }
    }
}

// Default implementation for AppSelectionFlags
impl Default for AppSelectionFlags {
    fn default() -> Self {
        Self::new()
    }
}

// Packages
const FIREFOX_PACMAN: [&str; 2] = ["firefox", "firefox-ublock-origin"];
const CHROMIUM_PACMAN: [&str; 1] = ["chromium"];
const UNGOOGLED_YAY: [&str; 1] = ["ungoogled-chromium-bin"];
const HELIUM_YAY: [&str; 1] = ["helium-browser-bin"];
const BRAVE_YAY: [&str; 1] = ["brave-bin"];
const ZEN_YAY: [&str; 1] = ["zen-browser-bin"];
const LIBREWOLF_YAY: [&str; 1] = ["librewolf-bin"];
const MULLVAD_YAY: [&str; 1] = ["mullvad-browser-bin"];
const QUTEBROWSER_PACMAN: [&str; 1] = ["qutebrowser"];
const GHOSTTY_PACMAN: [&str; 1] = ["ghostty"];
const KITTY_PACMAN: [&str; 1] = ["kitty"];
const ALACRITTY_PACMAN: [&str; 1] = ["alacritty"];
const ZED_PACMAN: [&str; 1] = ["zed"];
const CURSOR_YAY: [&str; 1] = ["cursor-bin"];
const VSCODE_YAY: [&str; 1] = ["visual-studio-code-bin"];
const VSCODIUM_YAY: [&str; 1] = ["vscodium-bin"];
const SUBLIME_YAY: [&str; 1] = ["sublime-text-4"];

pub const COMPOSITOR_LABELS: [&str; 1] = ["Hyprland"];

// Lists of available choices
pub const BROWSER_CHOICES: [InstallChoice; 9] = [
    InstallChoice {
        label: "Firefox",
        pacman: &FIREFOX_PACMAN,
        yay: &[],
    },
    InstallChoice {
        label: "Chromium",
        pacman: &CHROMIUM_PACMAN,
        yay: &[],
    },
    InstallChoice {
        label: "Ungoogled Chromium",
        pacman: &[],
        yay: &UNGOOGLED_YAY,
    },
    InstallChoice {
        label: "Helium",
        pacman: &[],
        yay: &HELIUM_YAY,
    },
    InstallChoice {
        label: "Brave",
        pacman: &[],
        yay: &BRAVE_YAY,
    },
    InstallChoice {
        label: "Zen Browser",
        pacman: &[],
        yay: &ZEN_YAY,
    },
    InstallChoice {
        label: "LibreWolf",
        pacman: &[],
        yay: &LIBREWOLF_YAY,
    },
    InstallChoice {
        label: "Mullvad",
        pacman: &[],
        yay: &MULLVAD_YAY,
    },
    InstallChoice {
        label: "qutebrowser",
        pacman: &QUTEBROWSER_PACMAN,
        yay: &[],
    },
];

pub const TERMINAL_CHOICES: [InstallChoice; 3] = [
    InstallChoice {
        label: "Ghostty",
        pacman: &GHOSTTY_PACMAN,
        yay: &[],
    },
    InstallChoice {
        label: "Kitty",
        pacman: &KITTY_PACMAN,
        yay: &[],
    },
    InstallChoice {
        label: "Alacritty",
        pacman: &ALACRITTY_PACMAN,
        yay: &[],
    },
];

pub const EDITOR_CHOICES: [InstallChoice; 5] = [
    InstallChoice {
        label: "Zed",
        pacman: &ZED_PACMAN,
        yay: &[],
    },
    InstallChoice {
        label: "Cursor",
        pacman: &[],
        yay: &CURSOR_YAY,
    },
    InstallChoice {
        label: "Visual Studio Code",
        pacman: &[],
        yay: &VSCODE_YAY,
    },
    InstallChoice {
        label: "VSCodium",
        pacman: &[],
        yay: &VSCODIUM_YAY,
    },
    InstallChoice {
        label: "Sublime Text 4",
        pacman: &[],
        yay: &SUBLIME_YAY,
    },
];

// Converts the application selection flags from the UI into a PackageSelection
pub fn selection_from_app_flags(flags: &AppSelectionFlags) -> PackageSelection {
    let mut selection = PackageSelection::default();
    merge_selection(
        &mut selection,
        selection_from_flags_for(&flags.browsers, &BROWSER_CHOICES),
    );
    merge_selection(
        &mut selection,
        selection_from_flags_for(&flags.editors, &EDITOR_CHOICES),
    );
    merge_selection(
        &mut selection,
        selection_from_flags_for(&flags.terminals, &TERMINAL_CHOICES),
    );
    selection
}

// Creates a PackageSelection from a set of flags and corresponding install choices
pub fn selection_from_flags_for(flags: &[bool], choices: &[InstallChoice]) -> PackageSelection {
    let mut selection = PackageSelection::default();
    for (flag, choice) in flags.iter().copied().zip(choices.iter()) {
        if flag {
            extend_unique(&mut selection.pacman, choice.pacman);
            extend_unique(&mut selection.yay, choice.yay);
        }
    }
    selection
}

// Gets the labels for the currently selected packages
pub fn labels_for_selection(
    selection: &PackageSelection,
    choices: &[InstallChoice],
) -> Vec<String> {
    let mut labels = Vec::new();
    for choice in choices {
        if choice_selected(selection, choice) {
            labels.push(choice.label.to_string());
        }
    }
    labels
}

// Gets the labels corresponding to a set of boolean flags
pub fn labels_for_flags(flags: &[bool], labels: &[&str]) -> Vec<String> {
    let mut selected = Vec::new();
    for (flag, label) in flags.iter().copied().zip(labels.iter().copied()) {
        if flag {
            selected.push(label.to_string());
        }
    }
    selected
}

// Checks if a specific install choice is selected based on the package lists
fn choice_selected(selection: &PackageSelection, choice: &InstallChoice) -> bool {
    for pkg in choice.pacman {
        if !selection.pacman.iter().any(|installed| installed == pkg) {
            return false;
        }
    }
    for pkg in choice.yay {
        if !selection.yay.iter().any(|installed| installed == pkg) {
            return false;
        }
    }
    !choice.pacman.is_empty() || !choice.yay.is_empty()
}

// Add elements to a vector, ensuring no duplicates
fn extend_unique(target: &mut Vec<String>, values: &[&str]) {
    for value in values {
        if !target.iter().any(|existing| existing == value) {
            target.push(value.to_string());
        }
    }
}

// Merges one PackageSelection into another, avoiding duplicate packages
fn merge_selection(target: &mut PackageSelection, source: PackageSelection) {
    extend_unique_owned(&mut target.pacman, source.pacman);
    extend_unique_owned(&mut target.yay, source.yay);
}

// Add owned strings to a vector, ensuring no duplicates
fn extend_unique_owned(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
}
