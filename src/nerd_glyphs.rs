pub fn find_nerd_font() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let localappdata = std::env::var("LOCALAPPDATA").unwrap_or_default();

    let dirs = [
        format!("{home}/Library/Fonts"),
        format!("{home}/.local/share/fonts"),
        "/usr/share/fonts".to_string(),
        "/usr/local/share/fonts".to_string(),
        "C:/Windows/Fonts".to_string(),
        format!("{localappdata}\\Microsoft\\Windows\\Fonts"),
    ];

    for dir in &dirs {
        let path = std::path::Path::new(dir);
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        let mut fallback: Option<std::path::PathBuf> = None;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(".ttf") && !lower.ends_with(".otf") {
                continue;
            }
            if is_symbols_only(&name) {
                return Some(entry.path());
            }
            if looks_like_nerd_font(&name) {
                fallback.get_or_insert(entry.path());
            }
        }
        if fallback.is_some() {
            return fallback;
        }
    }
    None
}

fn is_symbols_only(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("symbols") && lower.contains("nerd") && lower.contains("font")
}

fn looks_like_nerd_font(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("nerd") && lower.contains("font")
}
