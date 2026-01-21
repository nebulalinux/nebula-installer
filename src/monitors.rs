use anyhow::Result;

#[derive(Debug)]
struct MonitorMode {
    name: String,
    width: u32,
    height: u32,
    refresh: f64,
}

#[derive(Debug)]
struct ModeCandidate {
    width: u32,
    height: u32,
    refresh: f64,
    is_current: bool,
    is_preferred: bool,
}

fn parse_wlr_mode(line: &str) -> Option<ModeCandidate> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let res_token = tokens.get(0)?;
    if !res_token.contains('x') {
        return None;
    }
    let mut res_iter = res_token.split('x');
    let width: u32 = res_iter.next()?.parse().ok()?;
    let height: u32 = res_iter.next()?.parse().ok()?;

    let mut refresh: Option<f64> = None;
    for token in tokens.iter().skip(1) {
        let cleaned = token.trim_end_matches(['*', '+']);
        if let Some(value) = cleaned.strip_suffix("Hz") {
            refresh = value.parse().ok();
            break;
        }
        if cleaned.chars().all(|c| c.is_ascii_digit() || c == '.') {
            refresh = cleaned.parse().ok();
            break;
        }
    }

    let is_current = trimmed.contains('*') || trimmed.contains("(current)");
    let is_preferred = trimmed.contains('+') || trimmed.contains("(preferred)");

    Some(ModeCandidate {
        width,
        height,
        refresh: refresh.unwrap_or(60.0),
        is_current,
        is_preferred,
    })
}

fn parse_wlr_randr(output: &str) -> Vec<MonitorMode> {
    let mut monitors = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_modes: Vec<ModeCandidate> = Vec::new();
    let mut _current_scale: Option<f64> = None;

    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            if let Some(name) = current_name.take() {
                if let Some(mode) = current_modes
                    .iter()
                    .find(|m| m.is_current)
                    .or_else(|| current_modes.iter().find(|m| m.is_preferred))
                    .or_else(|| current_modes.first())
                {
                    monitors.push(MonitorMode {
                        name,
                        width: mode.width,
                        height: mode.height,
                        refresh: mode.refresh,
                    });
                }
            }
            current_name = line.split_whitespace().next().map(|s| s.to_string());
            current_modes.clear();
            _current_scale = None;
            continue;
        }

        let trimmed = line.trim();
        if trimmed.starts_with("Scale:") {
            if let Some(value) = trimmed.split_whitespace().nth(1) {
                _current_scale = value.parse().ok();
            }
        }
        if let Some(mode) = parse_wlr_mode(trimmed) {
            current_modes.push(mode);
        }
    }

    if let Some(name) = current_name.take() {
        if let Some(mode) = current_modes
            .iter()
            .find(|m| m.is_current)
            .or_else(|| current_modes.iter().find(|m| m.is_preferred))
            .or_else(|| current_modes.first())
        {
            monitors.push(MonitorMode {
                name,
                width: mode.width,
                height: mode.height,
                refresh: mode.refresh,
            });
        }
    }

    monitors
}

pub fn render_hypr_monitors_conf(output: &str) -> Result<Option<String>> {
    let monitors = parse_wlr_randr(output);
    if monitors.is_empty() {
        return Ok(None);
    }

    let mut contents = String::from("# Auto-generated\n");
    let mut x_offset: i32 = 0;
    for monitor in monitors {
        let scale = if monitor.width > 2560 || monitor.height > 1440 {
            1.5
        } else {
            1.0
        };
        contents.push_str(&format!(
            "monitor = {}, {}x{}@{:.2}, {}x0, {:.1}\n",
            monitor.name, monitor.width, monitor.height, monitor.refresh, x_offset, scale
        ));
        let logical_width = ((monitor.width as f64) / scale).round() as i32;
        x_offset += logical_width.max(0);
    }

    Ok(Some(contents))
}
