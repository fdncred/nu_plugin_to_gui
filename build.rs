#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn main() {
    let required_libs = [
        ("xcb", "libxcb"),
        ("xkbcommon", "libxkbcommon"),
        ("xkbcommon-x11", "libxkbcommon-x11"),
    ];

    let mut missing = Vec::new();

    for (pkg_name, display_name) in required_libs {
        if pkg_config::Config::new().probe(pkg_name).is_err() {
            missing.push(display_name);
        }
    }

    if !missing.is_empty() {
        panic!(
            "missing Linux GUI system libraries: {}\n\
             Install your distro's development packages for: libxcb, libxkbcommon, libxkbcommon-x11.\n\
             Example package names:\n\
             - Debian/Ubuntu: libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev\n\
             - Fedora: libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel\n\
             - Arch: libxcb libxkbcommon xorg-xkbcommon\n\
             Then rerun cargo build.",
            missing.join(", ")
        );
    }
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn main() {}
