use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod error {
    #[derive(Debug, thiserror::Error)]
    pub enum WaydeError {
        #[error("io error: {0}")]
        Io(#[from] std::io::Error),
        #[error("config parse error: {0}")]
        Parse(#[from] toml::de::Error),
        #[error("invalid config: {0}")]
        Config(String),
        #[error("backend error: {0}")]
        Backend(String),
    }

    pub type Result<T> = std::result::Result<T, WaydeError>;
}

mod config {
    use super::error::{Result, WaydeError};
    use serde::Deserialize;
    use std::path::{Path, PathBuf};

    #[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
    #[serde(rename_all = "snake_case")]
    pub enum FocusMode {
        FollowsMouse,
        Click,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Direction {
        Left,
        Down,
        Up,
        Right,
    }

    #[derive(Clone, PartialEq, Debug)]
    pub enum Action {
        SpawnTerminal,
        Spawn(String),
        Close,
        Focus(Direction),
        Move(Direction),
        Workspace(usize),
        MoveToWorkspace(usize),
        Reload,
        ResizeGrow,
        ResizeShrink,
        ToggleFloating,
        CycleLayout,
        Quit,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct Mods {
        pub logo: bool,
        pub shift: bool,
        pub ctrl: bool,
        pub alt: bool,
    }

    #[derive(Clone, PartialEq, Debug)]
    pub enum KeyMatch {
        Name(String),
        Code(u32),
    }

    #[derive(Clone, Debug)]
    pub struct Binding {
        pub mods: Mods,
        pub key: KeyMatch,
        pub action: Action,
    }

    #[derive(Deserialize, Clone, Debug)]
    #[serde(untagged)]
    pub enum Arg {
        Int(i64),
        Text(String),
    }

    #[derive(Deserialize, Clone, Debug)]
    pub struct KeybindSpec {
        #[serde(default)]
        pub mods: Vec<String>,
        pub key: Option<String>,
        pub keycode: Option<u32>,
        pub action: String,
        pub arg: Option<Arg>,
    }

    #[derive(Deserialize, Clone, Debug)]
    #[serde(default)]
    pub struct Theme {
        pub border_color_focused: String,
        pub border_color_unfocused: String,
        pub border_width: i32,
        pub gap: i32,
        pub font: String,
    }

    impl Default for Theme {
        fn default() -> Self {
            Self {
                border_color_focused: "#5294e2".into(),
                border_color_unfocused: "#3a3f4b".into(),
                border_width: 2,
                gap: 6,
                font: "monospace 10".into(),
            }
        }
    }

    #[derive(Deserialize, Clone, Debug)]
    #[serde(default)]
    pub struct OutputConfig {
        pub width: i32,
        pub height: i32,
        pub x: i32,
        pub y: i32,
        pub refresh: i32,
    }

    impl Default for OutputConfig {
        fn default() -> Self {
            Self {
                width: 1280,
                height: 800,
                x: 0,
                y: 0,
                refresh: 60,
            }
        }
    }

    #[derive(Deserialize, Clone, Debug)]
    #[serde(default)]
    pub struct Config {
        pub terminal: String,
        pub log_level: String,
        pub focus_mode: FocusMode,
        pub workspace_count: usize,
        pub theme: Theme,
        pub output: OutputConfig,
        pub keybinds: Vec<KeybindSpec>,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                terminal: "foot".into(),
                log_level: "info".into(),
                focus_mode: FocusMode::FollowsMouse,
                workspace_count: 10,
                theme: Theme::default(),
                output: OutputConfig::default(),
                keybinds: Vec::new(),
            }
        }
    }

    pub fn parse_color(text: &str) -> Option<[f32; 4]> {
        let hex = text.strip_prefix('#')?;
        if hex.len() != 6 && hex.len() != 8 {
            return None;
        }
        let channel = |i: usize| -> Option<f32> {
            u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16)
                .ok()
                .map(|v| f32::from(v) / 255.0)
        };
        let alpha = if hex.len() == 8 { channel(3)? } else { 1.0 };
        Some([channel(0)?, channel(1)?, channel(2)?, alpha])
    }

    fn parse_mods(names: &[String]) -> Result<Mods> {
        let mut mods = Mods::default();
        for name in names {
            match name.to_lowercase().as_str() {
                "super" | "mod" | "logo" | "win" => mods.logo = true,
                "shift" => mods.shift = true,
                "ctrl" | "control" => mods.ctrl = true,
                "alt" => mods.alt = true,
                other => return Err(WaydeError::Config(format!("unknown modifier '{other}'"))),
            }
        }
        Ok(mods)
    }

    fn workspace_arg(arg: &Option<Arg>) -> Result<usize> {
        let value = match arg {
            Some(Arg::Int(n)) => *n,
            Some(Arg::Text(t)) => t
                .parse::<i64>()
                .map_err(|_| WaydeError::Config(format!("bad workspace '{t}'")))?,
            None => return Err(WaydeError::Config("workspace action needs arg".into())),
        };
        if (1..=10).contains(&value) {
            Ok(value as usize - 1)
        } else {
            Err(WaydeError::Config(format!("workspace {value} out of range 1-10")))
        }
    }

    fn parse_action(name: &str, arg: &Option<Arg>) -> Result<Action> {
        Ok(match name {
            "spawn_terminal" => Action::SpawnTerminal,
            "spawn" => match arg {
                Some(Arg::Text(cmd)) => Action::Spawn(cmd.clone()),
                _ => return Err(WaydeError::Config("spawn needs a text arg".into())),
            },
            "close" => Action::Close,
            "focus_left" => Action::Focus(Direction::Left),
            "focus_down" => Action::Focus(Direction::Down),
            "focus_up" => Action::Focus(Direction::Up),
            "focus_right" => Action::Focus(Direction::Right),
            "move_left" => Action::Move(Direction::Left),
            "move_down" => Action::Move(Direction::Down),
            "move_up" => Action::Move(Direction::Up),
            "move_right" => Action::Move(Direction::Right),
            "workspace" => Action::Workspace(workspace_arg(arg)?),
            "move_to_workspace" => Action::MoveToWorkspace(workspace_arg(arg)?),
            "reload" => Action::Reload,
            "resize_grow" => Action::ResizeGrow,
            "resize_shrink" => Action::ResizeShrink,
            "toggle_floating" => Action::ToggleFloating,
            "cycle_layout" => Action::CycleLayout,
            "quit" => Action::Quit,
            other => return Err(WaydeError::Config(format!("unknown action '{other}'"))),
        })
    }

    fn default_keybinds() -> Vec<KeybindSpec> {
        let spec = |mods: &[&str], key: &str, action: &str, arg: Option<Arg>| KeybindSpec {
            mods: mods.iter().map(|m| (*m).to_string()).collect(),
            key: Some(key.to_string()),
            keycode: None,
            action: action.to_string(),
            arg,
        };
        let mut list = vec![
            spec(&["super"], "Return", "spawn_terminal", None),
            spec(&["super", "shift"], "q", "close", None),
            spec(&["super"], "h", "focus_left", None),
            spec(&["super"], "j", "focus_down", None),
            spec(&["super"], "k", "focus_up", None),
            spec(&["super"], "l", "focus_right", None),
            spec(&["super", "shift"], "h", "move_left", None),
            spec(&["super", "shift"], "j", "move_down", None),
            spec(&["super", "shift"], "k", "move_up", None),
            spec(&["super", "shift"], "l", "move_right", None),
            spec(&["super"], "r", "reload", None),
            spec(&["super"], "minus", "resize_shrink", None),
            spec(&["super"], "equal", "resize_grow", None),
            spec(&["super"], "space", "cycle_layout", None),
            spec(&["super", "shift"], "space", "toggle_floating", None),
            spec(&["super", "shift"], "e", "quit", None),
        ];
        for n in 1..=9 {
            let key = n.to_string();
            list.push(spec(&["super"], &key, "workspace", Some(Arg::Int(n))));
            list.push(spec(&["super", "shift"], &key, "move_to_workspace", Some(Arg::Int(n))));
        }
        list.push(spec(&["super"], "0", "workspace", Some(Arg::Int(10))));
        list.push(spec(&["super", "shift"], "0", "move_to_workspace", Some(Arg::Int(10))));
        list
    }

    impl Config {
        pub fn default_path() -> PathBuf {
            if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
                return PathBuf::from(dir).join("wayde/wayde.conf");
            }
            let home = std::env::var_os("HOME").unwrap_or_default();
            PathBuf::from(home).join(".config/wayde/wayde.conf")
        }

        pub fn load(path: &Path) -> Result<Self> {
            let text = std::fs::read_to_string(path)?;
            let config: Config = toml::from_str(&text)?;
            config.validate()?;
            Ok(config)
        }

        pub fn load_or_default(path: &Path) -> (Self, Option<String>) {
            if !path.exists() {
                return (Self::default(), Some(format!("no config at {}, using defaults", path.display())));
            }
            match Self::load(path) {
                Ok(config) => (config, None),
                Err(err) => (Self::default(), Some(format!("{err}; using defaults"))),
            }
        }

        pub fn validate(&self) -> Result<()> {
            if parse_color(&self.theme.border_color_focused).is_none()
                || parse_color(&self.theme.border_color_unfocused).is_none()
            {
                return Err(WaydeError::Config("border colors must be #rrggbb or #rrggbbaa".into()));
            }
            if self.theme.border_width < 0 || self.theme.gap < 0 {
                return Err(WaydeError::Config("border_width and gap must be >= 0".into()));
            }
            if !(1..=10).contains(&self.workspace_count) {
                return Err(WaydeError::Config("workspace_count must be 1-10".into()));
            }
            if self.output.width < 1 || self.output.height < 1 || self.output.refresh < 1 {
                return Err(WaydeError::Config("output size and refresh must be positive".into()));
            }
            if self.terminal.trim().is_empty() {
                return Err(WaydeError::Config("terminal must not be empty".into()));
            }
            self.log_level
                .parse::<log::LevelFilter>()
                .map_err(|_| WaydeError::Config(format!("bad log_level '{}'", self.log_level)))?;
            self.bindings()?;
            Ok(())
        }

        pub fn bindings(&self) -> Result<Vec<Binding>> {
            let mut specs = self.keybinds.clone();
            specs.extend(default_keybinds());
            specs
                .iter()
                .map(|spec| {
                    let key = match (&spec.key, spec.keycode) {
                        (Some(name), None) => KeyMatch::Name(name.to_lowercase()),
                        (None, Some(code)) => KeyMatch::Code(code),
                        _ => {
                            return Err(WaydeError::Config(
                                "keybind needs exactly one of key or keycode".into(),
                            ))
                        }
                    };
                    Ok(Binding {
                        mods: parse_mods(&spec.mods)?,
                        key,
                        action: parse_action(&spec.action, &spec.arg)?,
                    })
                })
                .collect()
        }

        pub fn focused_color(&self) -> [f32; 4] {
            parse_color(&self.theme.border_color_focused).unwrap_or([0.32, 0.58, 0.89, 1.0])
        }

        pub fn unfocused_color(&self) -> [f32; 4] {
            parse_color(&self.theme.border_color_unfocused).unwrap_or([0.23, 0.25, 0.29, 1.0])
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_colors() {
            assert_eq!(parse_color("#ff0000"), Some([1.0, 0.0, 0.0, 1.0]));
            assert!(parse_color("ff0000").is_none());
            assert!(parse_color("#ff00").is_none());
        }

        #[test]
        fn default_config_is_valid() {
            let config = Config::default();
            assert!(config.validate().is_ok());
            assert!(config.bindings().unwrap().len() > 30);
        }

        #[test]
        fn rejects_bad_workspace() {
            let config: Config = toml::from_str(
                "[[keybinds]]\nmods=[\"super\"]\nkey=\"x\"\naction=\"workspace\"\narg=11\n",
            )
            .unwrap();
            assert!(config.validate().is_err());
        }
    }
}

mod layout {
    use smithay::utils::{Logical, Point, Rectangle, Size};

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum LayoutKind {
        MasterStack,
        Horizontal,
        Vertical,
    }

    impl LayoutKind {
        pub fn next(self) -> Self {
            match self {
                Self::MasterStack => Self::Horizontal,
                Self::Horizontal => Self::Vertical,
                Self::Vertical => Self::MasterStack,
            }
        }
    }

    fn split(area: Rectangle<i32, Logical>, count: usize, gap: i32, horizontal: bool) -> Vec<Rectangle<i32, Logical>> {
        if count == 0 {
            return Vec::new();
        }
        let n = count as i32;
        let total = if horizontal { area.size.w } else { area.size.h };
        let each = ((total - gap * (n - 1)) / n).max(1);
        (0..n)
            .map(|i| {
                let offset = i * (each + gap);
                let length = if i == n - 1 { (total - offset).max(1) } else { each };
                if horizontal {
                    Rectangle::new(
                        Point::from((area.loc.x + offset, area.loc.y)),
                        Size::from((length, area.size.h)),
                    )
                } else {
                    Rectangle::new(
                        Point::from((area.loc.x, area.loc.y + offset)),
                        Size::from((area.size.w, length)),
                    )
                }
            })
            .collect()
    }

    pub fn compute(
        kind: LayoutKind,
        area: Rectangle<i32, Logical>,
        count: usize,
        gap: i32,
        ratio: f32,
    ) -> Vec<Rectangle<i32, Logical>> {
        match (kind, count) {
            (_, 0) => Vec::new(),
            (_, 1) => vec![area],
            (LayoutKind::MasterStack, n) => {
                let master_width = (((area.size.w - gap) as f32) * ratio) as i32;
                let master_width = master_width.clamp(1, (area.size.w - gap - 1).max(1));
                let master = Rectangle::new(area.loc, Size::from((master_width, area.size.h)));
                let stack_area = Rectangle::new(
                    Point::from((area.loc.x + master_width + gap, area.loc.y)),
                    Size::from(((area.size.w - master_width - gap).max(1), area.size.h)),
                );
                let mut rects = vec![master];
                rects.extend(split(stack_area, n - 1, gap, false));
                rects
            }
            (LayoutKind::Horizontal, n) => split(area, n, gap, true),
            (LayoutKind::Vertical, n) => split(area, n, gap, false),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn area() -> Rectangle<i32, Logical> {
            Rectangle::new(Point::from((10, 10)), Size::from((1000, 600)))
        }

        #[test]
        fn single_window_fills_area() {
            assert_eq!(compute(LayoutKind::MasterStack, area(), 1, 8, 0.5), vec![area()]);
        }

        #[test]
        fn master_stack_splits_stack_vertically() {
            let rects = compute(LayoutKind::MasterStack, area(), 3, 10, 0.5);
            assert_eq!(rects.len(), 3);
            assert_eq!(rects[0].size.h, 600);
            assert_eq!(rects[1].loc.x, rects[2].loc.x);
            assert_eq!(rects[1].loc.y + rects[1].size.h + 10, rects[2].loc.y);
            assert_eq!(rects[2].loc.y + rects[2].size.h, 610);
        }

        #[test]
        fn horizontal_covers_full_width() {
            let rects = compute(LayoutKind::Horizontal, area(), 3, 10, 0.5);
            assert_eq!(rects.last().map(|r| r.loc.x + r.size.w), Some(1010));
        }

        #[test]
        fn empty_is_empty() {
            assert!(compute(LayoutKind::Vertical, area(), 0, 5, 0.5).is_empty());
        }
    }
}

mod compositor {
    use super::config::{Action, Binding, Config, Direction, FocusMode, KeyMatch};
    use super::error::{Result, WaydeError};
    use super::layout::{self, LayoutKind};
    use super::{Arc, Duration, HashSet, Instant, OsString};
    use smithay::backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TouchEvent,
    };
    use smithay::backend::renderer::element::Id;
    use smithay::backend::renderer::utils::on_commit_buffer_handler;
    use smithay::delegate_compositor;
    use smithay::delegate_data_device;
    use smithay::delegate_output;
    use smithay::delegate_seat;
    use smithay::delegate_shm;
    use smithay::delegate_xdg_shell;
    use smithay::desktop::{PopupKind, PopupManager, Window, WindowSurfaceType};
    use smithay::input::keyboard::{xkb, FilterResult};
    use smithay::input::pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent};
    use smithay::input::touch::{DownEvent, MotionEvent as TouchMotionEvent, UpEvent};
    use smithay::input::{Seat, SeatHandler, SeatState};
    use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
    use smithay::reexports::calloop::generic::Generic;
    use smithay::reexports::calloop::{Interest, LoopHandle, LoopSignal, Mode as PollMode, PostAction};
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as ToplevelState;
    use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
    use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
    use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
    use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    use smithay::reexports::wayland_server::{Client, Display, DisplayHandle, Resource};
    use smithay::utils::{Logical, Point, Rectangle, Serial, Size, Transform, SERIAL_COUNTER};
    use smithay::wayland::buffer::BufferHandler;
    use smithay::wayland::compositor::{
        get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler, CompositorState,
    };
    use smithay::wayland::output::{OutputHandler, OutputManagerState};
    use smithay::wayland::selection::data_device::{
        set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
    };
    use smithay::wayland::selection::SelectionHandler;
    use smithay::wayland::shell::xdg::{
        PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    };
    use smithay::wayland::seat::WaylandFocus;
    use smithay::wayland::shm::{ShmHandler, ShmState};
    use smithay::wayland::socket::ListeningSocketSource;
    use std::process::{Command, Stdio};

    const MIN_WIDTH: i32 = 120;
    const MIN_HEIGHT: i32 = 80;
    const STEP: i32 = 40;
    const RATIO_STEP: f32 = 0.05;
    const BTN_LEFT: u32 = 0x110;
    const BTN_RIGHT: u32 = 0x111;

    pub struct Win {
        pub window: Window,
        pub floating: bool,
        pub rect: Rectangle<i32, Logical>,
        pub float_rect: Rectangle<i32, Logical>,
        pub z: u64,
        pub id_focused: Id,
        pub id_unfocused: Id,
    }

    pub struct Workspace {
        pub windows: Vec<Win>,
        pub focus: Option<Window>,
        pub layout: LayoutKind,
        pub ratio: f32,
    }

    impl Workspace {
        fn new() -> Self {
            Self {
                windows: Vec::new(),
                focus: None,
                layout: LayoutKind::MasterStack,
                ratio: 0.55,
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum DragKind {
        Move,
        Resize,
    }

    struct Drag {
        window: Window,
        kind: DragKind,
        origin: Point<f64, Logical>,
        start: Rectangle<i32, Logical>,
    }

    #[derive(Default)]
    pub struct ClientState {
        pub compositor_state: CompositorClientState,
    }

    impl ClientData for ClientState {
        fn initialized(&self, client_id: ClientId) {
            log::debug!("client connected: {client_id:?}");
        }

        fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
            log::info!("client disconnected: {client_id:?} ({reason:?})");
        }
    }

    pub struct Wayde {
        pub start_time: Instant,
        pub socket_name: OsString,
        pub display_handle: DisplayHandle,
        pub loop_signal: LoopSignal,
        pub config: Config,
        pub config_path: std::path::PathBuf,
        pub bindings: Vec<Binding>,
        pub compositor_state: CompositorState,
        pub xdg_shell_state: XdgShellState,
        pub shm_state: ShmState,
        #[allow(dead_code)]
        pub output_manager_state: OutputManagerState,
        pub seat_state: SeatState<Wayde>,
        pub data_device_state: DataDeviceState,
        pub popups: PopupManager,
        pub seat: Seat<Wayde>,
        pub output: Output,
        pub output_size: Size<i32, Logical>,
        pub workspaces: Vec<Workspace>,
        pub current: usize,
        pub pointer_location: Point<f64, Logical>,
        drag: Option<Drag>,
        swallowed: HashSet<u32>,
        z_counter: u64,
    }

    impl Wayde {
        pub fn new(
            handle: &LoopHandle<'static, Wayde>,
            display: Display<Wayde>,
            loop_signal: LoopSignal,
            config: Config,
            config_path: std::path::PathBuf,
        ) -> Result<Self> {
            let bindings = config.bindings()?;
            let dh = display.handle();

            let compositor_state = CompositorState::new::<Self>(&dh);
            let xdg_shell_state = XdgShellState::new::<Self>(&dh);
            let shm_state = ShmState::new::<Self>(&dh, vec![]);
            let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
            let data_device_state = DataDeviceState::new::<Self>(&dh);
            let mut seat_state = SeatState::new();
            let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "seat0");
            seat.add_keyboard(Default::default(), 200, 25)
                .map_err(|e| WaydeError::Backend(format!("keyboard: {e}")))?;
            seat.add_pointer();
            seat.add_touch();

            let out = &config.output;
            let mode = Mode {
                size: (out.width, out.height).into(),
                refresh: out.refresh * 1000,
            };
            let output = Output::new(
                "WAYDE-1".to_string(),
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Wayde".into(),
                    model: "Virtual".into(),
                },
            );
            output.create_global::<Self>(&dh);
            output.change_current_state(
                Some(mode),
                Some(Transform::Flipped180),
                None,
                Some((out.x, out.y).into()),
            );
            output.set_preferred(mode);

            let socket = ListeningSocketSource::new_auto().map_err(|e| WaydeError::Backend(e.to_string()))?;
            let socket_name = socket.socket_name().to_os_string();
            handle
                .insert_source(socket, |stream, _, state: &mut Wayde| {
                    if let Err(err) = state
                        .display_handle
                        .insert_client(stream, Arc::new(ClientState::default()))
                    {
                        log::warn!("failed to accept client: {err}");
                    }
                })
                .map_err(|e| WaydeError::Backend(e.to_string()))?;
            handle
                .insert_source(
                    Generic::new(display, Interest::READ, PollMode::Level),
                    |_, display, state| {
                        // SAFETY: the display is never dropped while the event loop owns it.
                        let result = unsafe { display.get_mut().dispatch_clients(state) };
                        if let Err(err) = result {
                            log::warn!("dispatch error: {err}");
                        }
                        Ok(PostAction::Continue)
                    },
                )
                .map_err(|e| WaydeError::Backend(e.to_string()))?;

            let count = config.workspace_count;
            Ok(Self {
                start_time: Instant::now(),
                socket_name,
                display_handle: dh,
                loop_signal,
                config_path,
                bindings,
                compositor_state,
                xdg_shell_state,
                shm_state,
                output_manager_state,
                seat_state,
                data_device_state,
                popups: PopupManager::default(),
                seat,
                output,
                output_size: (out.width, out.height).into(),
                workspaces: (0..count).map(|_| Workspace::new()).collect(),
                current: 0,
                pointer_location: (0.0, 0.0).into(),
                drag: None,
                swallowed: HashSet::new(),
                z_counter: 0,
                config,
            })
        }

        pub fn set_output_size(&mut self, size: Size<i32, Logical>) {
            self.output_size = size;
            for idx in 0..self.workspaces.len() {
                self.arrange(idx);
            }
        }

        fn border(&self) -> i32 {
            self.config.theme.border_width
        }

        pub fn render_location(&self, win: &Win) -> Point<i32, Logical> {
            let bw = self.border();
            win.rect.loc + Point::from((bw, bw)) - win.window.geometry().loc
        }

        pub fn stacking(&self) -> Vec<&Win> {
            let ws = &self.workspaces[self.current];
            let mut ordered: Vec<&Win> = ws.windows.iter().filter(|w| !w.floating).collect();
            let mut floating: Vec<&Win> = ws.windows.iter().filter(|w| w.floating).collect();
            floating.sort_by_key(|w| w.z);
            ordered.extend(floating);
            ordered
        }

        fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(Window, WlSurface, Point<f64, Logical>)> {
            for win in self.stacking().into_iter().rev() {
                let loc = self.render_location(win);
                if let Some((surface, offset)) = win
                    .window
                    .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                {
                    return Some((win.window.clone(), surface, (offset + loc).to_f64()));
                }
            }
            None
        }

        fn find_window(&self, root: &WlSurface) -> Option<Window> {
            self.workspaces
                .iter()
                .flat_map(|ws| ws.windows.iter())
                .find(|w| w.window.toplevel().is_some_and(|t| t.wl_surface() == root))
                .map(|w| w.window.clone())
        }

        fn win_mut(&mut self, window: &Window) -> Option<&mut Win> {
            self.workspaces[self.current]
                .windows
                .iter_mut()
                .find(|w| &w.window == window)
        }

        fn configure(&self, win: &Win) {
            let bw = self.border();
            if let Some(toplevel) = win.window.toplevel() {
                let size: Size<i32, Logical> = (
                    (win.rect.size.w - 2 * bw).max(1),
                    (win.rect.size.h - 2 * bw).max(1),
                )
                    .into();
                toplevel.with_pending_state(|state| {
                    state.size = Some(size);
                    for edge in [
                        ToplevelState::TiledLeft,
                        ToplevelState::TiledRight,
                        ToplevelState::TiledTop,
                        ToplevelState::TiledBottom,
                    ] {
                        if win.floating {
                            state.states.unset(edge);
                        } else {
                            state.states.set(edge);
                        }
                    }
                });
                toplevel.send_pending_configure();
            }
        }

        pub fn arrange(&mut self, idx: usize) {
            let gap = self.config.theme.gap;
            let size = self.output_size;
            let area = Rectangle::new(
                Point::from((gap, gap)),
                Size::from(((size.w - 2 * gap).max(1), (size.h - 2 * gap).max(1))),
            );
            let ws = &mut self.workspaces[idx];
            let tiled: Vec<usize> = (0..ws.windows.len()).filter(|&i| !ws.windows[i].floating).collect();
            let rects = layout::compute(ws.layout, area, tiled.len(), gap, ws.ratio);
            for (i, rect) in tiled.into_iter().zip(rects) {
                ws.windows[i].rect = rect;
            }
            for win in ws.windows.iter_mut().filter(|w| w.floating) {
                win.rect = win.float_rect;
            }
            for win in &self.workspaces[idx].windows {
                self.configure(win);
            }
        }

        pub fn set_focus(&mut self, target: Option<Window>) {
            self.z_counter += 1;
            let z = self.z_counter;
            let ws = &mut self.workspaces[self.current];
            for win in ws.windows.iter_mut() {
                let active = target.as_ref() == Some(&win.window);
                if win.window.set_activated(active) {
                    if let Some(toplevel) = win.window.toplevel() {
                        toplevel.send_pending_configure();
                    }
                }
                if active {
                    win.z = z;
                }
            }
            ws.focus = target.clone();
            let surface = target.and_then(|w| w.wl_surface().map(|s| s.into_owned()));
            if let Some(keyboard) = self.seat.get_keyboard() {
                keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
            }
        }

        fn refocus(&mut self) {
            let ws = &self.workspaces[self.current];
            let still_present = ws
                .focus
                .as_ref()
                .is_some_and(|f| ws.windows.iter().any(|w| &w.window == f));
            if still_present {
                return;
            }
            let next = self.stacking().last().map(|w| w.window.clone());
            self.set_focus(next);
        }

        fn centered_rect(&self) -> Rectangle<i32, Logical> {
            let size = self.output_size;
            let (w, h) = (size.w * 6 / 10, size.h * 6 / 10);
            Rectangle::new(Point::from(((size.w - w) / 2, (size.h - h) / 2)), Size::from((w, h)))
        }

        pub fn add_window(&mut self, toplevel: ToplevelSurface) {
            let floating = toplevel.parent().is_some();
            let window = Window::new_wayland_window(toplevel);
            let rect = self.centered_rect();
            let win = Win {
                window: window.clone(),
                floating,
                rect,
                float_rect: rect,
                z: 0,
                id_focused: Id::new(),
                id_unfocused: Id::new(),
            };
            self.workspaces[self.current].windows.push(win);
            self.arrange(self.current);
            self.set_focus(Some(window));
            self.refresh_pointer();
        }

        pub fn remove_dead(&mut self) {
            let mut changed = false;
            for idx in 0..self.workspaces.len() {
                let ws = &mut self.workspaces[idx];
                let before = ws.windows.len();
                ws.windows
                    .retain(|w| w.window.toplevel().is_some_and(|t| t.alive()));
                if ws.windows.len() != before {
                    changed = true;
                    if let Some(focus) = &ws.focus {
                        if !ws.windows.iter().any(|w| &w.window == focus) {
                            ws.focus = None;
                        }
                    }
                    self.arrange(idx);
                }
            }
            if let Some(drag) = &self.drag {
                if !drag.window.toplevel().is_some_and(|t| t.alive()) {
                    self.drag = None;
                }
            }
            if changed {
                self.refocus();
                self.refresh_pointer();
            }
        }

        pub fn remove_toplevel(&mut self, surface: &ToplevelSurface) {
            for ws in &mut self.workspaces {
                ws.windows
                    .retain(|w| w.window.toplevel().is_none_or(|t| t != surface));
            }
            self.remove_dead();
        }

        fn refresh_pointer(&mut self) {
            if let Some(pointer) = self.seat.get_pointer() {
                let location = self.pointer_location;
                let focus = self.surface_under(location).map(|(_, s, p)| (s, p));
                pointer.motion(
                    self,
                    focus,
                    &MotionEvent {
                        location,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: self.start_time.elapsed().as_millis() as u32,
                    },
                );
                pointer.frame(self);
            }
        }

        fn neighbor(&self, dir: Direction) -> Option<Window> {
            let ws = &self.workspaces[self.current];
            let focus = ws.focus.as_ref()?;
            let from = ws.windows.iter().find(|w| &w.window == focus)?;
            let center = |r: Rectangle<i32, Logical>| (r.loc.x + r.size.w / 2, r.loc.y + r.size.h / 2);
            let (cx, cy) = center(from.rect);
            ws.windows
                .iter()
                .filter(|w| &w.window != focus)
                .filter_map(|w| {
                    let (x, y) = center(w.rect);
                    let (dx, dy) = (x - cx, y - cy);
                    let (primary, secondary) = match dir {
                        Direction::Left => (-dx, dy.abs()),
                        Direction::Right => (dx, dy.abs()),
                        Direction::Up => (-dy, dx.abs()),
                        Direction::Down => (dy, dx.abs()),
                    };
                    (primary > 0).then(|| (primary + 2 * secondary, w.window.clone()))
                })
                .min_by_key(|(score, _)| *score)
                .map(|(_, window)| window)
        }

        fn focused(&self) -> Option<Window> {
            self.workspaces[self.current].focus.clone()
        }

        fn spawn(&self, command: &str) {
            let mut parts = command.split_whitespace();
            let Some(program) = parts.next() else { return };
            let mut cmd = Command::new(program);
            cmd.args(parts)
                .env("WAYLAND_DISPLAY", &self.socket_name)
                .env("XDG_SESSION_TYPE", "wayland")
                .stdin(Stdio::null());
            // SAFETY: only async-signal-safe libc calls run between fork and exec.
            unsafe {
                use std::os::unix::process::CommandExt;
                cmd.pre_exec(|| {
                    let mut set: libc::sigset_t = std::mem::zeroed();
                    libc::sigemptyset(&mut set);
                    libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                    Ok(())
                });
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    log::info!("spawned '{command}' (pid {})", child.id());
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                Err(err) => log::error!("failed to spawn '{command}': {err}"),
            }
        }

        fn switch_workspace(&mut self, idx: usize) {
            if idx >= self.workspaces.len() || idx == self.current {
                return;
            }
            self.drag = None;
            self.current = idx;
            self.arrange(idx);
            let focus = self.workspaces[idx].focus.clone();
            self.set_focus(focus);
            self.refocus();
            self.refresh_pointer();
            log::debug!("workspace {}", idx + 1);
        }

        fn move_to_workspace(&mut self, idx: usize) {
            if idx >= self.workspaces.len() || idx == self.current {
                return;
            }
            let Some(focus) = self.focused() else { return };
            let source = &mut self.workspaces[self.current];
            let Some(pos) = source.windows.iter().position(|w| w.window == focus) else {
                return;
            };
            let win = source.windows.remove(pos);
            source.focus = None;
            let target = &mut self.workspaces[idx];
            target.focus = Some(win.window.clone());
            target.windows.push(win);
            self.arrange(self.current);
            self.arrange(idx);
            self.refocus();
            self.refresh_pointer();
        }

        fn toggle_floating(&mut self) {
            let Some(focus) = self.focused() else { return };
            let fallback = self.centered_rect();
            if let Some(win) = self.win_mut(&focus) {
                win.floating = !win.floating;
                if win.floating {
                    win.float_rect = if win.float_rect.size.w > 0 { win.float_rect } else { fallback };
                }
            }
            self.arrange(self.current);
            self.set_focus(Some(focus));
        }

        fn resize_focused(&mut self, grow: bool) {
            let Some(focus) = self.focused() else { return };
            let delta = if grow { STEP } else { -STEP };
            let current = self.current;
            let Some(win) = self.win_mut(&focus) else { return };
            if win.floating {
                let size = win.float_rect.size;
                win.float_rect.size = Size::from(((size.w + delta).max(MIN_WIDTH), (size.h + delta).max(MIN_HEIGHT)));
            } else {
                let ws = &mut self.workspaces[current];
                let step = if grow { RATIO_STEP } else { -RATIO_STEP };
                ws.ratio = (ws.ratio + step).clamp(0.15, 0.85);
            }
            self.arrange(current);
        }

        fn move_focused(&mut self, dir: Direction) {
            let Some(focus) = self.focused() else { return };
            let floating = self.workspaces[self.current]
                .windows
                .iter()
                .find(|w| w.window == focus)
                .is_some_and(|w| w.floating);
            if floating {
                let (dx, dy) = match dir {
                    Direction::Left => (-STEP, 0),
                    Direction::Right => (STEP, 0),
                    Direction::Up => (0, -STEP),
                    Direction::Down => (0, STEP),
                };
                if let Some(win) = self.win_mut(&focus) {
                    win.float_rect.loc += Point::from((dx, dy));
                }
            } else if let Some(other) = self.neighbor(dir) {
                let ws = &mut self.workspaces[self.current];
                let a = ws.windows.iter().position(|w| w.window == focus);
                let b = ws.windows.iter().position(|w| w.window == other);
                if let (Some(a), Some(b)) = (a, b) {
                    ws.windows.swap(a, b);
                }
            }
            self.arrange(self.current);
        }

        fn reload(&mut self) {
            let loaded = Config::load(&self.config_path).and_then(|c| c.bindings().map(|b| (c, b)));
            match loaded {
                Ok((config, bindings)) => {
                    if let Ok(level) = config.log_level.parse::<log::LevelFilter>() {
                        log::set_max_level(level);
                    }
                    self.config = config;
                    self.bindings = bindings;
                    for ws in &mut self.workspaces {
                        for win in &mut ws.windows {
                            win.id_focused = Id::new();
                            win.id_unfocused = Id::new();
                        }
                    }
                    while self.workspaces.len() < self.config.workspace_count {
                        self.workspaces.push(Workspace::new());
                    }
                    for idx in 0..self.workspaces.len() {
                        self.arrange(idx);
                    }
                    log::info!("configuration reloaded");
                }
                Err(err) => log::error!("reload failed, keeping current configuration: {err}"),
            }
        }

        pub fn run_action(&mut self, action: Action) {
            match action {
                Action::SpawnTerminal => {
                    let terminal = self.config.terminal.clone();
                    self.spawn(&terminal);
                }
                Action::Spawn(cmd) => self.spawn(&cmd),
                Action::Close => {
                    if let Some(toplevel) = self.focused().and_then(|w| w.toplevel().cloned()) {
                        toplevel.send_close();
                    }
                }
                Action::Focus(dir) => {
                    if let Some(window) = self.neighbor(dir) {
                        self.set_focus(Some(window));
                    }
                }
                Action::Move(dir) => self.move_focused(dir),
                Action::Workspace(idx) => self.switch_workspace(idx),
                Action::MoveToWorkspace(idx) => self.move_to_workspace(idx),
                Action::Reload => self.reload(),
                Action::ResizeGrow => self.resize_focused(true),
                Action::ResizeShrink => self.resize_focused(false),
                Action::ToggleFloating => self.toggle_floating(),
                Action::CycleLayout => {
                    let ws = &mut self.workspaces[self.current];
                    ws.layout = ws.layout.next();
                    self.arrange(self.current);
                }
                Action::Quit => self.loop_signal.stop(),
            }
            self.refresh_pointer();
        }

        fn clamp_pointer(&mut self, point: Point<f64, Logical>) -> Point<f64, Logical> {
            let size = self.output_size;
            (
                point.x.clamp(0.0, f64::from(size.w - 1).max(0.0)),
                point.y.clamp(0.0, f64::from(size.h - 1).max(0.0)),
            )
                .into()
        }

        fn start_drag(&mut self, window: Window, kind: DragKind) {
            let current = self.current;
            let Some(win) = self.win_mut(&window) else { return };
            if !win.floating {
                win.floating = true;
                win.float_rect = win.rect;
            }
            let start = win.float_rect;
            self.arrange(current);
            self.set_focus(Some(window.clone()));
            self.drag = Some(Drag {
                window,
                kind,
                origin: self.pointer_location,
                start,
            });
        }

        fn update_drag(&mut self) {
            let Some(drag) = &self.drag else { return };
            let delta = self.pointer_location - drag.origin;
            let (dx, dy) = (delta.x.round() as i32, delta.y.round() as i32);
            let (window, kind, start) = (drag.window.clone(), drag.kind, drag.start);
            let current = self.current;
            if let Some(win) = self.win_mut(&window) {
                match kind {
                    DragKind::Move => win.float_rect.loc = start.loc + Point::from((dx, dy)),
                    DragKind::Resize => {
                        win.float_rect.size =
                            Size::from(((start.size.w + dx).max(MIN_WIDTH), (start.size.h + dy).max(MIN_HEIGHT)));
                    }
                }
                win.rect = win.float_rect;
            }
            let _ = current;
            if let Some(win) = self.workspaces[self.current].windows.iter().find(|w| w.window == window) {
                self.configure(win);
            }
        }

        fn pointer_moved(&mut self, location: Point<f64, Logical>, time: u32) {
            self.pointer_location = location;
            if self.drag.is_some() {
                self.update_drag();
                return;
            }
            let under = self.surface_under(location);
            if self.config.focus_mode == FocusMode::FollowsMouse {
                if let Some((window, _, _)) = &under {
                    if self.focused().as_ref() != Some(window) {
                        self.set_focus(Some(window.clone()));
                    }
                }
            }
            if let Some(pointer) = self.seat.get_pointer() {
                pointer.motion(
                    self,
                    under.map(|(_, s, p)| (s, p)),
                    &MotionEvent {
                        location,
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
                pointer.frame(self);
            }
        }

        fn match_binding(&self, mods: &smithay::input::keyboard::ModifiersState, name: &str, code: u32) -> Option<Action> {
            self.bindings
                .iter()
                .find(|b| {
                    b.mods.logo == mods.logo
                        && b.mods.shift == mods.shift
                        && b.mods.ctrl == mods.ctrl
                        && b.mods.alt == mods.alt
                        && match &b.key {
                            KeyMatch::Name(n) => n == name,
                            KeyMatch::Code(c) => *c == code,
                        }
                })
                .map(|b| b.action.clone())
        }

        pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
            match event {
                InputEvent::Keyboard { event } => {
                    let Some(keyboard) = self.seat.get_keyboard() else { return };
                    let serial = SERIAL_COUNTER.next_serial();
                    let time = Event::time_msec(&event);
                    let pressed = event.state() == KeyState::Pressed;
                    let code = event.key_code();
                    let raw = code.raw();
                    let action = keyboard.input::<Option<Action>, _>(
                        self,
                        code,
                        event.state(),
                        serial,
                        time,
                        |data, mods, handle| {
                            if !pressed {
                                return if data.swallowed.remove(&raw) {
                                    FilterResult::Intercept(None)
                                } else {
                                    FilterResult::Forward
                                };
                            }
                            let name = handle
                                .raw_latin_sym_or_raw_current_sym()
                                .map(|sym| xkb::keysym_get_name(sym).to_lowercase())
                                .unwrap_or_default();
                            match data.match_binding(mods, &name, raw) {
                                Some(action) => {
                                    data.swallowed.insert(raw);
                                    FilterResult::Intercept(Some(action))
                                }
                                None => FilterResult::Forward,
                            }
                        },
                    );
                    if let Some(Some(action)) = action {
                        self.run_action(action);
                    }
                }
                InputEvent::PointerMotionAbsolute { event } => {
                    let location = event.position_transformed(self.output_size);
                    let location = self.clamp_pointer(location);
                    self.pointer_moved(location, event.time_msec());
                }
                InputEvent::PointerMotion { event } => {
                    let location = self.pointer_location + Point::from((event.delta_x(), event.delta_y()));
                    let location = self.clamp_pointer(location);
                    self.pointer_moved(location, event.time_msec());
                }
                InputEvent::PointerButton { event } => {
                    let (Some(pointer), Some(keyboard)) = (self.seat.get_pointer(), self.seat.get_keyboard()) else {
                        return;
                    };
                    let serial = SERIAL_COUNTER.next_serial();
                    let button = event.button_code();
                    let state = event.state();
                    if state == ButtonState::Released {
                        if self.drag.take().is_some() {
                            return;
                        }
                    } else {
                        let under = self.surface_under(self.pointer_location);
                        let logo = keyboard.modifier_state().logo;
                        if let Some((window, _, _)) = under {
                            if logo && (button == BTN_LEFT || button == BTN_RIGHT) {
                                let kind = if button == BTN_LEFT { DragKind::Move } else { DragKind::Resize };
                                self.start_drag(window, kind);
                                return;
                            }
                            if self.focused().as_ref() != Some(&window) {
                                self.set_focus(Some(window));
                            }
                        }
                    }
                    pointer.button(
                        self,
                        &ButtonEvent {
                            button,
                            state,
                            serial,
                            time: event.time_msec(),
                        },
                    );
                    pointer.frame(self);
                }
                InputEvent::PointerAxis { event } => {
                    let Some(pointer) = self.seat.get_pointer() else { return };
                    let source = event.source();
                    let mut frame = AxisFrame::new(event.time_msec()).source(source);
                    for axis in [Axis::Horizontal, Axis::Vertical] {
                        let v120 = event.amount_v120(axis);
                        let amount = event.amount(axis).unwrap_or_else(|| v120.unwrap_or(0.0) * 15.0 / 120.0);
                        if amount != 0.0 {
                            frame = frame
                                .relative_direction(axis, event.relative_direction(axis))
                                .value(axis, amount);
                            if let Some(discrete) = v120 {
                                frame = frame.v120(axis, discrete as i32);
                            }
                        } else if source == AxisSource::Finger {
                            frame = frame.stop(axis);
                        }
                    }
                    pointer.axis(self, frame);
                    pointer.frame(self);
                }
                InputEvent::TouchDown { event } => {
                    let Some(touch) = self.seat.get_touch() else { return };
                    let location = event.position_transformed(self.output_size);
                    let under = self.surface_under(location);
                    if let Some((window, _, _)) = &under {
                        if self.focused().as_ref() != Some(window) {
                            self.set_focus(Some(window.clone()));
                        }
                    }
                    touch.down(
                        self,
                        under.map(|(_, s, p)| (s, p)),
                        &DownEvent {
                            slot: event.slot(),
                            location,
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time_msec(),
                        },
                    );
                    touch.frame(self);
                }
                InputEvent::TouchMotion { event } => {
                    let Some(touch) = self.seat.get_touch() else { return };
                    let location = event.position_transformed(self.output_size);
                    let under = self.surface_under(location);
                    touch.motion(
                        self,
                        under.map(|(_, s, p)| (s, p)),
                        &TouchMotionEvent {
                            slot: event.slot(),
                            location,
                            time: event.time_msec(),
                        },
                    );
                    touch.frame(self);
                }
                InputEvent::TouchUp { event } => {
                    let Some(touch) = self.seat.get_touch() else { return };
                    touch.up(
                        self,
                        &UpEvent {
                            slot: event.slot(),
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time_msec(),
                        },
                    );
                    touch.frame(self);
                }
                InputEvent::TouchCancel { .. } => {
                    if let Some(touch) = self.seat.get_touch() {
                        touch.cancel(self);
                    }
                }
                InputEvent::TouchFrame { .. } => {
                    if let Some(touch) = self.seat.get_touch() {
                        touch.frame(self);
                    }
                }
                _ => {}
            }
        }

        pub fn send_frames(&self) {
            let elapsed = self.start_time.elapsed();
            for win in &self.workspaces[self.current].windows {
                win.window
                    .send_frame(&self.output, elapsed, Some(Duration::ZERO), |_, _| Some(self.output.clone()));
            }
        }
    }

    impl CompositorHandler for Wayde {
        fn compositor_state(&mut self) -> &mut CompositorState {
            &mut self.compositor_state
        }

        fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
            match client.get_data::<ClientState>() {
                Some(data) => &data.compositor_state,
                None => unreachable!("every client is created with ClientState"),
            }
        }

        fn commit(&mut self, surface: &WlSurface) {
            on_commit_buffer_handler::<Self>(surface);
            if !is_sync_subsurface(surface) {
                let mut root = surface.clone();
                while let Some(parent) = get_parent(&root) {
                    root = parent;
                }
                if let Some(window) = self.find_window(&root) {
                    window.on_commit();
                    if let Some(toplevel) = window.toplevel() {
                        if !toplevel.is_initial_configure_sent() {
                            toplevel.send_configure();
                        }
                    }
                }
            }
            self.popups.commit(surface);
            if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
                if !popup.is_initial_configure_sent() {
                    if let Err(err) = popup.send_configure() {
                        log::debug!("popup configure failed: {err:?}");
                    }
                }
            }
        }
    }

    impl BufferHandler for Wayde {
        fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
    }

    impl ShmHandler for Wayde {
        fn shm_state(&self) -> &ShmState {
            &self.shm_state
        }
    }

    impl XdgShellHandler for Wayde {
        fn xdg_shell_state(&mut self) -> &mut XdgShellState {
            &mut self.xdg_shell_state
        }

        fn new_toplevel(&mut self, surface: ToplevelSurface) {
            self.add_window(surface);
        }

        fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
            surface.with_pending_state(|state| state.geometry = positioner.get_geometry());
            if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface)) {
                log::debug!("popup tracking failed: {err:?}");
            }
        }

        fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
            surface.with_pending_state(|state| {
                state.geometry = positioner.get_geometry();
                state.positioner = positioner;
            });
            surface.send_repositioned(token);
        }

        fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

        fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
            self.remove_toplevel(&surface);
        }
    }

    impl SeatHandler for Wayde {
        type KeyboardFocus = WlSurface;
        type PointerFocus = WlSurface;
        type TouchFocus = WlSurface;

        fn seat_state(&mut self) -> &mut SeatState<Self> {
            &mut self.seat_state
        }

        fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
            let dh = &self.display_handle;
            let client = focused.and_then(|s| dh.get_client(s.id()).ok());
            set_data_device_focus(dh, seat, client);
        }

        fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}
    }

    impl SelectionHandler for Wayde {
        type SelectionUserData = ();
    }

    impl DataDeviceHandler for Wayde {
        fn data_device_state(&self) -> &DataDeviceState {
            &self.data_device_state
        }
    }

    impl ClientDndGrabHandler for Wayde {}
    impl ServerDndGrabHandler for Wayde {}
    impl OutputHandler for Wayde {}

    delegate_compositor!(Wayde);
    delegate_shm!(Wayde);
    delegate_xdg_shell!(Wayde);
    delegate_output!(Wayde);
    delegate_seat!(Wayde);
    delegate_data_device!(Wayde);
}

mod backend {
    use super::compositor::Wayde;
    use super::config::parse_color;
    use super::error::WaydeError;
    use smithay::backend::renderer::damage::OutputDamageTracker;
    use smithay::backend::renderer::element::render_elements;
    use smithay::backend::renderer::element::solid::SolidColorRenderElement;
    use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
    use smithay::backend::renderer::element::{AsRenderElements, Kind};
    use smithay::backend::renderer::gles::GlesRenderer;
    use smithay::backend::renderer::utils::CommitCounter;
    use smithay::backend::renderer::{Color32F, ImportAll, ImportMem, Renderer, Texture};
    use smithay::backend::winit::{self, WinitEvent};
    use smithay::output::Mode;
    use smithay::reexports::calloop::EventLoop;
    use smithay::reexports::winit::dpi::LogicalSize;
    use smithay::reexports::winit::window::Window as WinitWindow;
    use smithay::utils::Scale;

    render_elements! {
        pub WaydeElement<R> where R: Renderer + ImportAll + ImportMem, R::TextureId: Texture + Clone + 'static;
        Surface=WaylandSurfaceRenderElement<R>,
        Solid=SolidColorRenderElement,
    }

    fn build_elements(state: &Wayde, renderer: &mut GlesRenderer) -> Vec<WaydeElement<GlesRenderer>> {
        let focus = state.workspaces[state.current].focus.clone();
        let bw = state.config.theme.border_width;
        let focused_color = state.config.focused_color();
        let unfocused_color = state.config.unfocused_color();
        let mut elements = Vec::new();
        for win in state.stacking().into_iter().rev() {
            let location = state.render_location(win).to_physical_precise_round(1.0);
            let surfaces: Vec<WaydeElement<GlesRenderer>> =
                win.window
                    .render_elements(renderer, location, Scale::from(1.0), 1.0);
            elements.extend(surfaces);
            if bw > 0 {
                let is_focused = focus.as_ref() == Some(&win.window);
                let (id, color) = if is_focused {
                    (win.id_focused.clone(), focused_color)
                } else {
                    (win.id_unfocused.clone(), unfocused_color)
                };
                elements.push(WaydeElement::Solid(SolidColorRenderElement::new(
                    id,
                    win.rect.to_physical_precise_round(1.0),
                    CommitCounter::default(),
                    Color32F::new(color[0], color[1], color[2], color[3]),
                    Kind::Unspecified,
                )));
            }
        }
        elements
    }

    pub fn init(event_loop: &mut EventLoop<'static, Wayde>, state: &mut Wayde) -> super::error::Result<()> {
        let out = state.config.output.clone();
        let attributes = WinitWindow::default_attributes()
            .with_inner_size(LogicalSize::new(f64::from(out.width), f64::from(out.height)))
            .with_title("Wayde")
            .with_visible(true);
        let (mut backend, winit_source) = winit::init_from_attributes::<GlesRenderer>(attributes)
            .map_err(|e| WaydeError::Backend(e.to_string()))?;

        let size = backend.window_size();
        state.set_output_size((size.w, size.h).into());
        let mode = Mode {
            size,
            refresh: out.refresh * 1000,
        };
        let output = state.output.clone();
        output.change_current_state(Some(mode), None, None, None);
        output.set_preferred(mode);
        let mut damage_tracker = OutputDamageTracker::from_output(&output);
        let background = parse_color("#14141a").unwrap_or([0.08, 0.08, 0.1, 1.0]);

        event_loop
            .handle()
            .insert_source(winit_source, move |event, _, state| match event {
                WinitEvent::Resized { size, .. } => {
                    let mode = Mode {
                        size,
                        refresh: state.config.output.refresh * 1000,
                    };
                    state.output.change_current_state(Some(mode), None, None, None);
                    state.output.set_preferred(mode);
                    state.set_output_size((size.w, size.h).into());
                }
                WinitEvent::Input(input) => state.process_input_event(input),
                WinitEvent::Redraw => {
                    state.remove_dead();
                    let age = backend.buffer_age().unwrap_or(0);
                    let rendered = match backend.bind() {
                        Ok((renderer, mut framebuffer)) => {
                            let elements = build_elements(state, renderer);
                            match damage_tracker.render_output(renderer, &mut framebuffer, age, &elements, background) {
                                Ok(result) => Some(result.damage.cloned()),
                                Err(err) => {
                                    log::error!("render failed: {err}");
                                    None
                                }
                            }
                        }
                        Err(err) => {
                            log::error!("bind failed: {err}");
                            None
                        }
                    };
                    if let Some(damage) = rendered {
                        if let Err(err) = backend.submit(damage.as_deref()) {
                            log::error!("submit failed: {err}");
                        }
                    }
                    state.send_frames();
                    state.popups.cleanup();
                    if let Err(err) = state.display_handle.flush_clients() {
                        log::warn!("flush failed: {err}");
                    }
                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => state.loop_signal.stop(),
                _ => {}
            })
            .map_err(|e| WaydeError::Backend(e.to_string()))?;
        Ok(())
    }

}

use compositor::Wayde;
use config::Config;
use error::{Result, WaydeError};
use calloop::signals::{Signal, Signals};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

fn config_path_from_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" || arg == "-c" {
            if let Some(path) = args.next() {
                return PathBuf::from(path);
            }
        }
    }
    Config::default_path()
}

fn run() -> Result<()> {
    let path = config_path_from_args();
    let (config, warning) = Config::load_or_default(&path);
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(config.log_level.clone())).init();
    if let Some(message) = warning {
        log::warn!("{message}");
    }

    let mut event_loop: EventLoop<'static, Wayde> =
        EventLoop::try_new().map_err(|e| WaydeError::Backend(e.to_string()))?;
    let display: Display<Wayde> = Display::new().map_err(|e| WaydeError::Backend(e.to_string()))?;
    let mut state = Wayde::new(&event_loop.handle(), display, event_loop.get_signal(), config, path)?;

    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM]).map_err(|e| WaydeError::Backend(e.to_string()))?;
    event_loop
        .handle()
        .insert_source(signals, |event, _, state| {
            log::info!("received {:?}, shutting down", event.signal());
            state.loop_signal.stop();
        })
        .map_err(|e| WaydeError::Backend(e.to_string()))?;

    backend::init(&mut event_loop, &mut state)?;
    log::info!("listening on WAYLAND_DISPLAY={}", state.socket_name.to_string_lossy());

    event_loop
        .run(None, &mut state, |state| {
            if let Err(err) = state.display_handle.flush_clients() {
                log::warn!("flush failed: {err}");
            }
        })
        .map_err(|e| WaydeError::Backend(e.to_string()))?;
    log::info!("wayde stopped");
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("wayde: {err}");
        std::process::exit(1);
    }
}
