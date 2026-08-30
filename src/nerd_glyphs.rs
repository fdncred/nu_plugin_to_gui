pub fn find_nerd_font_family(font_names: &[String]) -> Option<String> {
    let priorities: &[&str] = &[
        "Symbols Nerd Font Mono",
        "Symbols Nerd Font",
        "Nerd Font Mono",
        "Nerd Font",
    ];
    for pattern in priorities {
        if let Some(name) = font_names.iter().find(|n| n.contains(pattern)) {
            return Some(name.clone());
        }
    }
    None
}
