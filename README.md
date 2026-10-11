# Wayde

Tiling Wayland compositor in a single Rust file (`wayde.rs`) built on smithay 0.7.

Current backend: winit (runs as a window inside an existing Wayland/X11 session). A DRM/KMS + libinput backend for bare-metal sessions is not implemented yet.

## Build

Dependencies (build and runtime): Rust >= 1.80, libxkbcommon, EGL/GLES (Mesa), Wayland libs, pkg-config.

Arch:

    sudo pacman -S --needed rust libxkbcommon mesa wayland pkgconf

Fedora:

    sudo dnf install cargo libxkbcommon-devel mesa-libEGL-devel mesa-libGLES-devel wayland-devel pkgconf-pkg-config

Ubuntu/Debian:

    sudo apt install cargo libxkbcommon-dev libegl1-mesa-dev libgles2-mesa-dev libwayland-dev pkg-config

Then:

    cargo build --release
    install -Dm755 target/release/wayde ~/.local/bin/wayde
    install -Dm644 wayde.conf ~/.config/wayde/wayde.conf
    install -Dm644 wayde.service ~/.config/systemd/user/wayde.service

Run with `wayde` or `wayde --config path/to/wayde.conf`. Release binary is about 3.4 MB.

Optional service: `systemctl --user enable --now wayde.service`.

## Default keybinds (Mod = Super)

| Key | Action |
| --- | --- |
| Mod+Return | spawn terminal |
| Mod+Shift+Q | close window |
| Mod+H/J/K/L | focus left/down/up/right |
| Mod+Shift+H/J/K/L | move window (swap tiled, nudge floating) |
| Mod+1..9, Mod+0 | workspace 1-10 |
| Mod+Shift+1..9, 0 | move window to workspace |
| Mod+Minus / Mod+Equal | shrink / grow floating window (master ratio when tiled) |
| Mod+Space | cycle layout (master-stack, horizontal, vertical) |
| Mod+Shift+Space | toggle floating |
| Mod+Left drag / Mod+Right drag | move / resize (makes window floating) |
| Mod+R | reload config |
| Mod+Shift+E | quit |

Config `[[keybinds]]` are added on top of the defaults and take priority on conflict. Use `key` (xkb keysym name) or `keycode` (xkb keycode), with `mods` from `super`, `shift`, `ctrl`, `alt`. Actions: `spawn_terminal`, `spawn` (arg = command), `close`, `focus_*`, `move_*`, `workspace` / `move_to_workspace` (arg = 1-10), `reload`, `resize_grow`, `resize_shrink`, `toggle_floating`, `cycle_layout`, `quit`.

## Notes

- `font` is parsed but unused: the compositor draws no text.
- `RUST_LOG` overrides `log_level`.
- SIGINT/SIGTERM shut down cleanly; a crashing client is dropped without affecting the compositor.
