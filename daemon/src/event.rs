use std::fmt;

/// What the dial can say, whatever the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialEvent {
    /// Positive = clockwise.
    Turn(i32),
    Press,
    DoublePress,
}

impl fmt::Display for DialEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialEvent::Turn(n) => write!(f, "turn({n:+})"),
            DialEvent::Press => write!(f, "press"),
            DialEvent::DoublePress => write!(f, "doublePress"),
        }
    }
}

/// The nine segments, clockwise from the gap at 12 o'clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeId {
    Volume,
    Brightness,
    Apps,
    Windows,
    Desktops,
    Tabs,
    Scroll,
    UndoRedo,
    Slides,
}

impl ModeId {
    pub const ALL: [ModeId; 9] = [
        ModeId::Volume,
        ModeId::Brightness,
        ModeId::Apps,
        ModeId::Windows,
        ModeId::Desktops,
        ModeId::Tabs,
        ModeId::Scroll,
        ModeId::UndoRedo,
        ModeId::Slides,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ModeId::Volume => "Volume",
            ModeId::Brightness => "Brightness",
            ModeId::Apps => "Apps",
            ModeId::Windows => "Windows",
            ModeId::Desktops => "Workspaces",
            ModeId::Tabs => "Tabs",
            ModeId::Scroll => "Scroll",
            ModeId::UndoRedo => "Undo / Redo",
            ModeId::Slides => "Slides",
        }
    }
}

/// Timestamped log line, same shape as the Phase 0 probe: `HH:MM:SS.mmm [tag   ] message`.
pub fn log(tag: &str, msg: impl fmt::Display) {
    let now = chrono::Local::now();
    println!("{} [{tag:6}] {msg}", now.format("%H:%M:%S%.3f"));
}
