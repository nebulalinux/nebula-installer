// Lists of packages to be installed
#[derive(Default, Clone)]
pub struct PackageSelection {
    pub pacman: Vec<String>,
    pub yay: Vec<String>,
}

impl PackageSelection {}

use crate::config::{config, ChoiceConfig};

// Single installable application choice in the UI
pub type InstallChoice = ChoiceConfig;

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
        let mut compositors = vec![false; compositor_labels().len()];
        if let Some(flag) = compositors.first_mut() {
            *flag = true;
        }
        let mut browsers = vec![false; browser_choices().len()];
        if let Some((idx, _)) = browser_choices()
            .iter()
            .enumerate()
            .find(|(_, choice)| choice.label == "Zen Browser")
        {
            if let Some(flag) = browsers.get_mut(idx) {
                *flag = true;
            }
        }
        let mut editors = vec![false; editor_choices().len()];
        if let Some((idx, _)) = editor_choices()
            .iter()
            .enumerate()
            .find(|(_, choice)| choice.label == "Visual Studio Code")
        {
            if let Some(flag) = editors.get_mut(idx) {
                *flag = true;
            }
        }
        Self {
            compositors,
            browsers,
            editors,
            terminals: vec![false; terminal_choices().len()],
        }
    }

    pub fn enforce_defaults(&mut self) {
        if let Some(flag) = self.compositors.first_mut() {
            *flag = true;
        }
        if let Some((idx, _)) = browser_choices()
            .iter()
            .enumerate()
            .find(|(_, choice)| choice.label == "Zen Browser")
        {
            if let Some(flag) = self.browsers.get_mut(idx) {
                *flag = true;
            }
        }
        if let Some((idx, _)) = editor_choices()
            .iter()
            .enumerate()
            .find(|(_, choice)| choice.label == "Visual Studio Code")
        {
            if let Some(flag) = self.editors.get_mut(idx) {
                *flag = true;
            }
        }
    }
}

// Default implementation for AppSelectionFlags
impl Default for AppSelectionFlags {
    fn default() -> Self {
        Self::new()
    }
}

pub fn compositor_labels() -> &'static [String] {
    &config().selections.compositors
}

pub fn browser_choices() -> &'static [InstallChoice] {
    &config().selections.browsers
}

pub fn editor_choices() -> &'static [InstallChoice] {
    &config().selections.editors
}

pub fn terminal_choices() -> &'static [InstallChoice] {
    &config().selections.terminals
}

// Converts the application selection flags from the UI into a PackageSelection
pub fn selection_from_app_flags(flags: &AppSelectionFlags) -> PackageSelection {
    let mut selection = PackageSelection::default();
    merge_selection(
        &mut selection,
        selection_from_flags_for(&flags.browsers, browser_choices()),
    );
    merge_selection(
        &mut selection,
        selection_from_flags_for(&flags.editors, editor_choices()),
    );
    merge_selection(
        &mut selection,
        selection_from_flags_for(&flags.terminals, terminal_choices()),
    );
    selection
}

// Creates a PackageSelection from a set of flags and corresponding install choices
pub fn selection_from_flags_for(flags: &[bool], choices: &[InstallChoice]) -> PackageSelection {
    let mut selection = PackageSelection::default();
    for (flag, choice) in flags.iter().copied().zip(choices.iter()) {
        if flag {
            extend_unique(&mut selection.pacman, &choice.pacman);
            extend_unique(&mut selection.yay, &choice.yay);
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
pub fn labels_for_flags(flags: &[bool], labels: &[String]) -> Vec<String> {
    let mut selected = Vec::new();
    for (flag, label) in flags.iter().copied().zip(labels.iter()) {
        if flag {
            selected.push(label.to_string());
        }
    }
    selected
}

// Checks if a specific install choice is selected based on the package lists
fn choice_selected(selection: &PackageSelection, choice: &InstallChoice) -> bool {
    for pkg in &choice.pacman {
        if !selection.pacman.iter().any(|installed| installed == pkg) {
            return false;
        }
    }
    for pkg in &choice.yay {
        if !selection.yay.iter().any(|installed| installed == pkg) {
            return false;
        }
    }
    !choice.pacman.is_empty() || !choice.yay.is_empty()
}

// Add elements to a vector, ensuring no duplicates
fn extend_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.iter().any(|existing| existing == value) {
            target.push(value.clone());
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
