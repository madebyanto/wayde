# wayde

tiling wayland compositor in one single rust file (`wayde.rs`), built on smithay 0.7. yes, one file. no, im not sorry

right now it only has the winit backend, so it runs as a window inside ur existing wayland/x11 session. DRM/KMS + libinput for real bare-metal sessions? not implemented yet, patches welcome lol

## build

deps (build + runtime): rust >= 1.80, libxkbcommon, EGL/GLES (mesa), wayland libs, pkg-config

arch (btw):

    sudo pacman -S --needed rust libxkbcommon mesa wayland pkgconf

fedora:

    sudo dnf install cargo libxkbcommon-devel mesa-libEGL-devel mesa-libGLES-devel wayland-devel pkgconf-pkg-config

ubuntu/debian:

    sudo apt install cargo libxkbcommon-dev libegl1-mesa-dev libgles2-mesa-dev libwayland-dev pkg-config

then just:

    cargo build --release
    install -Dm755 target/release/wayde ~/.local/bin/wayde
    install -Dm644 wayde.conf ~/.config/wayde/wayde.conf
    install -Dm644 wayde.service ~/.config/systemd/user/wayde.service

run it w/ `wayde` or `wayde --config path/to/wayde.conf`. release binary is ~3.4 MB, tiny af

optional systemd service if u want it: `systemctl --user enable --now wayde.service`

## default keybinds (Mod = Super)

| Key | Action |
| --- | --- |
| Mod+Return | spawn terminal |
| Mod+Shift+Q | close window |
| Mod+H/J/K/L | focus left/down/up/right (vim gang) |
| Mod+Shift+H/J/K/L | move window (swaps if tiled, nudges if floating) |
| Mod+1..9, Mod+0 | workspace 1-10 |
| Mod+Shift+1..9, 0 | send window to workspace |
| Mod+Minus / Mod+Equal | shrink / grow floating window (master ratio when tiled) |
| Mod+Space | cycle layout (master-stack, horizontal, vertical) |
| Mod+Shift+Space | toggle floating |
| Mod+Left drag / Mod+Right drag | move / resize (window goes floating) |
| Mod+R | reload config |
| Mod+Shift+E | quit |

`[[keybinds]]` in ur config get added on top of the defaults and win if theres a conflict. use `key` (xkb keysym name) or `keycode` (xkb keycode), `mods` can be `super`, `shift`, `ctrl`, `alt`. available actions: `spawn_terminal`, `spawn` (arg = the command), `close`, `focus_*`, `move_*`, `workspace` / `move_to_workspace` (arg = 1-10), `reload`, `resize_grow`, `resize_shrink`, `toggle_floating`, `cycle_layout`, `quit`

## notes

- `font` gets parsed but does nothing, the compositor doesnt draw any text (yet?)
- `RUST_LOG` overrides `log_level`
- SIGINT/SIGTERM shut it down clean, and if a client crashes it just gets dropped, compositor keeps going