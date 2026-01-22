use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub packages: PackagesConfig,
    pub selections: SelectionsConfig,
}

#[derive(Debug, Deserialize)]
pub struct PackagesConfig {
    pub required: Vec<String>,
    pub hyprland: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SelectionsConfig {
    pub compositors: Vec<String>,
    pub browsers: Vec<ChoiceConfig>,
    pub editors: Vec<ChoiceConfig>,
    pub terminals: Vec<ChoiceConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ChoiceConfig {
    pub label: String,
    #[serde(default)]
    pub pacman: Vec<String>,
    #[serde(default)]
    pub yay: Vec<String>,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

pub fn config() -> &'static Config {
    CONFIG.get_or_init(|| {
        let raw = include_str!("../config.toml");
        let parsed: Config = toml::from_str(raw).expect("Invalid nebula-installer config.toml");
        validate_config(&parsed).expect("Invalid nebula-installer config.toml");
        parsed
    })
}

fn validate_config(cfg: &Config) -> Result<(), String> {
    if cfg.packages.required.is_empty() {
        return Err("packages.required must not be empty".to_string());
    }
    if cfg.packages.hyprland.is_empty() {
        return Err("packages.hyprland must not be empty".to_string());
    }
    if cfg.selections.compositors.is_empty() {
        return Err("selections.compositors must not be empty".to_string());
    }
    if cfg.selections.browsers.is_empty() {
        return Err("selections.browsers must not be empty".to_string());
    }
    if cfg.selections.editors.is_empty() {
        return Err("selections.editors must not be empty".to_string());
    }
    if cfg.selections.terminals.is_empty() {
        return Err("selections.terminals must not be empty".to_string());
    }

    validate_choices("selections.browsers", &cfg.selections.browsers)?;
    validate_choices("selections.editors", &cfg.selections.editors)?;
    validate_choices("selections.terminals", &cfg.selections.terminals)?;

    Ok(())
}

fn validate_choices(section: &str, choices: &[ChoiceConfig]) -> Result<(), String> {
    for (idx, choice) in choices.iter().enumerate() {
        if choice.label.trim().is_empty() {
            return Err(format!("{section}[{idx}].label must not be empty"));
        }
        if choice.pacman.is_empty() && choice.yay.is_empty() {
            return Err(format!(
                "{section}[{idx}] must include at least one pacman or yay package"
            ));
        }
    }
    Ok(())
}
