use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn run(quiet: bool) -> Result<(), String> {
    let config_dir = find_config_dir()
        .ok_or_else(|| "could not determine config directory (HOME not set)".to_string())?;

    fs::create_dir_all(&config_dir)
        .map_err(|e| format!("failed to create config directory: {e}"))?;

    let binary_name = if cfg!(windows) {
        "statusline.exe"
    } else {
        "statusline"
    };
    let install_path = config_dir.join(binary_name);

    let current_exe =
        std::env::current_exe().map_err(|e| format!("failed to locate current executable: {e}"))?;

    fs::copy(&current_exe, &install_path)
        .map_err(|e| format!("failed to copy binary to {}: {e}", install_path.display()))?;

    let settings_path = config_dir.join("settings.json");
    update_settings(&settings_path, &install_path, quiet)?;

    if !quiet {
        println!(
            "\x1b[1;32m✓\x1b[0m Installed to \x1b[1m{}\x1b[0m",
            install_path.display()
        );
    }
    Ok(())
}

fn find_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(home_var)
        .ok()
        .map(|h| PathBuf::from(h).join(".claude"))
}

fn update_settings(settings_path: &Path, install_path: &Path, quiet: bool) -> Result<(), String> {
    let command = install_path.to_string_lossy().into_owned();
    let our_entry = serde_json::json!({
        "type": "command",
        "command": command,
        "padding": 2
    });

    let mut settings = if settings_path.exists() {
        let content = fs::read_to_string(settings_path)
            .map_err(|e| format!("failed to read settings.json: {e}"))?;
        match serde_json::from_str::<Value>(&content)
            .map_err(|e| format!("failed to parse settings.json: {e}"))?
        {
            Value::Object(map) => Value::Object(map),
            _ => return Err("settings.json is not a JSON object".to_string()),
        }
    } else {
        Value::Object(serde_json::Map::new())
    };

    if !quiet {
        if let Some(existing) = settings.get("statusLine") {
            if existing != &our_entry {
                eprintln!("\x1b[1;33m⚠️  statusLine already configured — overwriting\x1b[0m");
                eprintln!("  \x1b[2mwas:\x1b[0m \x1b[31m{existing}\x1b[0m");
                eprintln!("  \x1b[2mnow:\x1b[0m \x1b[32m{our_entry}\x1b[0m");
            }
        }
    }

    settings["statusLine"] = our_entry;

    let content = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("failed to serialise settings: {e}"))?;
    fs::write(settings_path, content).map_err(|e| format!("failed to write settings.json: {e}"))?;

    Ok(())
}
