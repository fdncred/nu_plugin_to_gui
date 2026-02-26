# nu_plugin_to_gui

`nu_plugin_to_gui` is a small Rust utility that bridges Nushell plugins and a graphical user interface. It pulls in necessary Nushell crates (plugin, protocol, color-config, utils) and exposes a simple GUI built with `gpui` components.

The intent is to show the output of Nushell commands in a GUI.

## Getting Started

1. Clone the repository and ensure you have Rust installed (edition 2024).
2. Run `cargo build` to compile the project or `cargo install --path .`. Dependencies are fetched from the Nushell GitHub repo.
3. Register the plugin with `plugin add /path/to/nu_plugin_to_gui`.
4. Restart or use the plugin with `plugin use /path/to/nu_plugin_to_gui`
5. Try it out `ls | to-gui`

_Note:_ the project is in early development and primarily intended for internal tooling or experimentation.


Feel free to open issues or contribute enhancements.
