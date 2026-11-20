//! The floating on-screen keyboard.
//!
//! A touch screen has no Ctrl, Shift or Alt, and the desktop interface (which this app hosts
//! unchanged) is full of shortcuts that need them. This module is a small egui overlay, owned by the
//! Android wrapper ([`crate::Mobile`]), that taps out real keyboard events:
//!
//! * [`VirtualKeyboard::show`] draws the overlay after the editor's own UI (a compact *control pad*,
//!   a full PC keyboard, or a draggable bubble that docks to a screen edge) and queues the events of
//!   every tap;
//! * [`VirtualKeyboard::feed_raw_input`] hands one queued tap per frame to eframe's raw-input hook,
//!   right after the app's own hook, so the events enter egui exactly like a hardware keyboard's.
//!
//! Nothing here resolves a shortcut: `Ctrl`+`Z` is only a `Key::Z` press with `ctrl` and `command`
//! set, as `menus::parse_shortcut` builds it on every platform but macOS, and the app's own shortcut
//! dispatcher decides what it means. Printable keys also send the `Event::Text` a hardware keyboard
//! would (never with Ctrl, Alt or Win held), so text fields type.
//!
//! The state lives in memory for the session; nothing is saved.
use std::collections::VecDeque;

use egui::{Align2, Color32, CornerRadius, Event, FontId, Id, Key, Modifiers, Order, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

const PANEL_ID: &str = "effectcraft-virtual-keyboard";
const BUBBLE_ID: &str = "effectcraft-virtual-keyboard-bubble";
/// Space kept between the overlay and the screen edge, in points.
const MARGIN: f32 = 8.0;
/// A bubble drag shorter than this (in points, times the keyboard scale) still counts as a tap.
const TAP_SLOP: f32 = 8.0;
/// Taps waiting for a frame; a hook that never runs cannot grow this without bound.
const QUEUE_LIMIT: usize = 32;
const FIRST_STATUS: &str = "Tap a modifier, then a key. Drag the top bar to move.";

// Palette (approved design: solid charcoal blue).
const PANEL: Color32 = Color32::from_rgb(0x20, 0x28, 0x34);
const PANEL_BORDER: Color32 = Color32::from_rgb(0x46, 0x53, 0x63);
const BOX_BG: Color32 = Color32::from_rgb(0x1b, 0x24, 0x30);
const BOX_BORDER: Color32 = Color32::from_rgb(0x35, 0x43, 0x54);
const BRAND: Color32 = Color32::from_rgb(0xc3, 0xd1, 0xe2);
const GRIP: Color32 = Color32::from_rgb(0x78, 0x90, 0xad);
const PILL_BG: Color32 = Color32::from_rgb(0x1a, 0x22, 0x2d);
const PILL_BORDER: Color32 = Color32::from_rgb(0x3a, 0x48, 0x59);
const PILL_TEXT: Color32 = Color32::from_rgb(0x9f, 0xb2, 0xca);
const ICON_BG: Color32 = Color32::from_rgb(0x30, 0x3b, 0x4a);
const ICON_DOWN: Color32 = Color32::from_rgb(0x47, 0x5a, 0x70);
const ICON_BORDER: Color32 = Color32::from_rgb(0x45, 0x53, 0x65);
const ICON_FG: Color32 = Color32::from_rgb(0xe2, 0xea, 0xf4);
const KEY: Color32 = Color32::from_rgb(0x35, 0x42, 0x53);
const KEY_BORDER: Color32 = Color32::from_rgb(0x46, 0x54, 0x66);
const KEY_DOWN: Color32 = Color32::from_rgb(0x52, 0x66, 0x7f);
const TEXT: Color32 = Color32::from_rgb(0xed, 0xf3, 0xfb);
const UTILITY: Color32 = Color32::from_rgb(0x2b, 0x35, 0x43);
const UTILITY_TEXT: Color32 = Color32::from_rgb(0xc8, 0xd4, 0xe3);
const MODIFIER: Color32 = Color32::from_rgb(0x26, 0x3c, 0x54);
const MODIFIER_BORDER: Color32 = Color32::from_rgb(0x53, 0x77, 0x9d);
const MODIFIER_TEXT: Color32 = Color32::from_rgb(0xdc, 0xec, 0xff);
const ACTIVE: Color32 = Color32::from_rgb(0x2f, 0x75, 0xbf);
const ACTIVE_BORDER: Color32 = Color32::from_rgb(0x91, 0xc4, 0xff);
const ACCENT: Color32 = Color32::from_rgb(0x31, 0x5c, 0x88);
const ACCENT_BORDER: Color32 = Color32::from_rgb(0x5b, 0x8f, 0xc4);
const TAB: Color32 = Color32::from_rgb(0x1b, 0x25, 0x31);
const TAB_BORDER: Color32 = Color32::from_rgb(0x3b, 0x4a, 0x5b);
const TAB_TEXT: Color32 = Color32::from_rgb(0x9e, 0xaf, 0xc4);
const TAB_ON: Color32 = Color32::from_rgb(0x31, 0x5d, 0x89);
const TAB_ON_BORDER: Color32 = Color32::from_rgb(0x65, 0x99, 0xcc);
const SECTION: Color32 = Color32::from_rgb(0x83, 0x94, 0xa9);
const STATUS: Color32 = Color32::from_rgb(0x8e, 0x9e, 0xb2);
const BUBBLE: Color32 = Color32::from_rgb(0x28, 0x5f, 0x99);
const BUBBLE_DOWN: Color32 = Color32::from_rgb(0x1f, 0x4c, 0x7d);
const BUBBLE_BORDER: Color32 = Color32::from_rgb(0xa5, 0xce, 0xfa);

// ---------------------------------------------------------------------------------------------
// Key codes and events
// ---------------------------------------------------------------------------------------------

const LETTERS: [Key; 26] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
];
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: [Key; 10] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
const DIGIT_LABELS: [&str; 10] = ["0 )", "1 !", "2 @", "3 #", "4 $", "5 %", "6 ^", "7 &", "8 *", "9 ("];
const DIGIT_SHIFTED: [char; 10] = [')', '!', '@', '#', '$', '%', '^', '&', '*', '('];
const DIGIT_PLAIN: &str = "0123456789";
/// (key, plain, shifted, label)
const PUNCTUATION: [(Key, char, char, &str); 11] = [
    (Key::Backtick, '`', '~', "` ~"),
    (Key::Minus, '-', '_', "- _"),
    (Key::Equals, '=', '+', "= +"),
    (Key::OpenBracket, '[', '{', "[ {"),
    (Key::CloseBracket, ']', '}', "] }"),
    (Key::Backslash, '\\', '|', "\\ |"),
    (Key::Semicolon, ';', ':', "; :"),
    (Key::Quote, '\'', '"', "' \""),
    (Key::Comma, ',', '<', ", <"),
    (Key::Period, '.', '>', ". >"),
    (Key::Slash, '/', '?', "/ ?"),
];
const F_KEYS: [Key; 24] = [
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
    Key::F13,
    Key::F14,
    Key::F15,
    Key::F16,
    Key::F17,
    Key::F18,
    Key::F19,
    Key::F20,
    Key::F21,
    Key::F22,
    Key::F23,
    Key::F24,
];
const F_LABELS: [&str; 24] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];

/// The sticky modifiers. `Meta` is the Win key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mod {
    Ctrl,
    Shift,
    Alt,
    Meta,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Mods {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

impl Mods {
    fn get(self, m: Mod) -> bool {
        match m {
            Mod::Ctrl => self.ctrl,
            Mod::Shift => self.shift,
            Mod::Alt => self.alt,
            Mod::Meta => self.meta,
        }
    }

    fn toggle(&mut self, m: Mod) {
        let flag = match m {
            Mod::Ctrl => &mut self.ctrl,
            Mod::Shift => &mut self.shift,
            Mod::Alt => &mut self.alt,
            Mod::Meta => &mut self.meta,
        };
        *flag = !*flag;
    }

    /// The modifiers as the app's shortcut parser builds them (`menus::parse_shortcut`): off macOS
    /// `Cmd` is `command` plus `ctrl`, so Ctrl sets both. egui has no field for the Win key outside
    /// macOS; `mac_cmd` is where winit puts Super, and the shortcut matcher ignores it, which is
    /// what a hardware keyboard's Win key does in this app.
    fn to_egui(self) -> Modifiers {
        let mut m = Modifiers::NONE;
        m.ctrl = self.ctrl;
        m.command = self.ctrl;
        m.shift = self.shift;
        m.alt = self.alt;
        m.mac_cmd = self.meta;
        m
    }

    /// "Ctrl + Shift", or `None` when nothing is held.
    fn describe(self) -> Option<String> {
        let names: Vec<&str> = [(self.ctrl, "Ctrl"), (self.shift, "Shift"), (self.alt, "Alt"), (self.meta, "Win")]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect();
        (!names.is_empty()).then(|| names.join(" + "))
    }
}

/// The text a printable key types.
#[derive(Clone, Copy)]
enum Typed {
    Nothing,
    Plain(char),
    /// (plain, with Shift)
    Shifted(char, char),
    /// A letter (lowercase): Shift and Caps Lock each flip its case.
    Letter(char),
}

impl Typed {
    /// What the key types with `mods` held and Caps Lock `caps`; nothing under Ctrl, Alt or Win, as
    /// on a hardware keyboard (the shortcut gets the key, the text field does not).
    fn resolve(self, mods: Mods, caps: bool) -> Option<String> {
        if mods.ctrl || mods.alt || mods.meta {
            return None;
        }
        let c = match self {
            Typed::Nothing => return None,
            Typed::Plain(c) => c,
            Typed::Shifted(plain, shifted) => {
                if mods.shift {
                    shifted
                } else {
                    plain
                }
            }
            Typed::Letter(lower) => {
                if mods.shift != caps {
                    lower.to_ascii_uppercase()
                } else {
                    lower
                }
            }
        };
        Some(c.to_string())
    }
}

/// One physical key of the virtual keyboard.
#[derive(Clone, Copy)]
struct Code {
    /// For the status line.
    name: &'static str,
    /// `None` when egui has no such key (Print Screen, Num Lock...) or the key only types text.
    key: Option<Key>,
    typed: Typed,
    /// Whether the event names a physical key (the numpad has none in egui: its keys are the main
    /// block's, and claiming that position would trigger `Physical+` bindings of the number row).
    physical: bool,
}

impl Code {
    fn named(key: Key) -> Self {
        Self { name: key.name(), key: Some(key), typed: Typed::Nothing, physical: true }
    }

    fn printable(key: Key, plain: char, shifted: char) -> Self {
        Self { name: key.name(), key: Some(key), typed: Typed::Shifted(plain, shifted), physical: true }
    }

    fn letter(key: Key, lower: char) -> Self {
        Self { name: key.name(), key: Some(key), typed: Typed::Letter(lower), physical: true }
    }

    fn numpad(key: Key, c: char) -> Self {
        Self { name: key.name(), key: Some(key), typed: Typed::Plain(c), physical: false }
    }

    fn text_only(name: &'static str, c: char) -> Self {
        Self { name, key: None, typed: Typed::Plain(c), physical: false }
    }

    fn unsupported(name: &'static str) -> Self {
        Self { name, key: None, typed: Typed::Nothing, physical: false }
    }
}

/// The events of one tap, in the order a hardware keyboard produces them: press, text, release.
/// `None` when the key has neither an egui key nor text, or Ctrl/Alt/Win blocks the text of a key
/// that only types text.
fn tap_events(code: Code, mods: Mods, caps: bool) -> Option<Vec<Event>> {
    let text = code.typed.resolve(mods, caps);
    let Some(key) = code.key else {
        return text.map(|t| vec![Event::Text(t)]);
    };
    let modifiers = mods.to_egui();
    // The windowing layer turns Ctrl+C / Ctrl+X into clipboard events instead of key presses, and
    // the shortcut dispatcher maps them back (`collect_key_events`): do the same.
    if modifiers.command && !modifiers.alt && !modifiers.shift && !mods.meta {
        match key {
            Key::C => return Some(vec![Event::Copy]),
            Key::X => return Some(vec![Event::Cut]),
            _ => {}
        }
    }
    let physical_key = code.physical.then_some(key);
    let mut events = vec![Event::Key { key, physical_key, pressed: true, repeat: false, modifiers }];
    if let Some(t) = text {
        events.push(Event::Text(t));
    }
    events.push(Event::Key { key, physical_key, pressed: false, repeat: false, modifiers });
    Some(events)
}

// ---------------------------------------------------------------------------------------------
// Layout vocabulary
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Compact,
    Full,
    Bubble,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Main,
    Navigation,
    Numpad,
    Extra,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dock {
    Free,
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum Kind {
    Normal,
    Utility,
    Modifier,
    Accent,
    Tab,
}

/// What tapping a button does.
#[derive(Clone, Copy)]
enum Act {
    /// A spacer.
    Nothing,
    Press(Code),
    /// Press `key` with Ctrl added to the held modifiers (the quick shortcuts).
    Ctrl(Key),
    Toggle(Mod),
    CapsLock,
    ClearModifiers,
    Switch(Mode),
    Show(Section),
}

/// A button: label, weight in its row, look and action.
#[derive(Clone, Copy)]
struct K {
    label: &'static str,
    sub: Option<&'static str>,
    w: f32,
    kind: Kind,
    act: Act,
    arrow: Option<Dir>,
}

impl K {
    fn new(label: &'static str, w: f32, kind: Kind, act: Act) -> Self {
        Self { label, sub: None, w, kind, act, arrow: None }
    }

    fn gap(w: f32) -> Self {
        Self::new("", w, Kind::Utility, Act::Nothing)
    }

    fn code(label: &'static str, w: f32, kind: Kind, code: Code) -> Self {
        Self::new(label, w, kind, Act::Press(code))
    }

    fn util(label: &'static str, w: f32, key: Key) -> Self {
        Self::code(label, w, Kind::Utility, Code::named(key))
    }

    fn missing(label: &'static str, w: f32, name: &'static str) -> Self {
        Self::code(label, w, Kind::Utility, Code::unsupported(name))
    }

    fn modifier(label: &'static str, w: f32, m: Mod) -> Self {
        Self::new(label, w, Kind::Modifier, Act::Toggle(m))
    }

    fn arrow(dir: Dir, key: Key) -> Self {
        Self { arrow: Some(dir), ..Self::util("", 1.0, key) }
    }

    fn quick(label: &'static str, sub: &'static str, key: Key) -> Self {
        Self { sub: Some(sub), ..Self::new(label, 1.0, Kind::Accent, Act::Ctrl(key)) }
    }

    fn pad(label: &'static str, key: Key, c: char) -> Self {
        Self::code(label, 1.0, Kind::Normal, Code::numpad(key, c))
    }
}

fn letter(c: char) -> K {
    let i = (c as usize).saturating_sub('a' as usize);
    match (LETTERS.get(i), UPPER.get(i..i + 1)) {
        (Some(&key), Some(label)) => K::code(label, 1.0, Kind::Normal, Code::letter(key, c)),
        _ => K::gap(1.0),
    }
}

fn digit(d: usize) -> K {
    match (DIGITS.get(d), DIGIT_LABELS.get(d), DIGIT_SHIFTED.get(d), DIGIT_PLAIN.chars().nth(d)) {
        (Some(&key), Some(&label), Some(&shifted), Some(plain)) => K::code(label, 1.0, Kind::Normal, Code::printable(key, plain, shifted)),
        _ => K::gap(1.0),
    }
}

fn punct(key: Key, w: f32) -> K {
    match PUNCTUATION.iter().find(|p| p.0 == key) {
        Some(&(key, plain, shifted, label)) => K::code(label, w, Kind::Normal, Code::printable(key, plain, shifted)),
        None => K::gap(w),
    }
}

fn fkey(n: usize) -> K {
    let i = n.saturating_sub(1);
    match (F_KEYS.get(i), F_LABELS.get(i)) {
        (Some(&key), Some(&label)) => K::util(label, 1.0, key),
        _ => K::gap(1.0),
    }
}

fn letters(row: &str) -> Vec<K> {
    row.chars().map(letter).collect()
}

fn space(w: f32) -> K {
    K::code("Space", w, Kind::Normal, Code::printable(Key::Space, ' ', ' '))
}

/// Cell `rect` of a `cols` x `rows` grid inside `area`, spanning `cs` x `rs` cells.
fn cell(area: Rect, cols: usize, rows: usize, gap: f32, origin: (usize, usize), span: (usize, usize)) -> Rect {
    let cw = (area.width() - gap * cols.saturating_sub(1) as f32) / cols.max(1) as f32;
    let ch = (area.height() - gap * rows.saturating_sub(1) as f32) / rows.max(1) as f32;
    let (c, r) = (origin.0 as f32, origin.1 as f32);
    let (cs, rs) = (span.0 as f32, span.1 as f32);
    Rect::from_min_size(
        pos2(area.min.x + c * (cw + gap), area.min.y + r * (ch + gap)),
        vec2(cs * cw + (cs - 1.0) * gap, rs * ch + (rs - 1.0) * gap),
    )
}

/// A grid item: column, row, columns spanned, rows spanned, button.
type Item = (usize, usize, usize, usize, K);

fn at(col: usize, row: usize, k: K) -> Item {
    (col, row, 1, 1, k)
}

fn clamp_pos(pos: Pos2, size: Vec2, vp: Rect) -> Pos2 {
    let lo = vp.min + vec2(MARGIN, MARGIN);
    let hi = pos2((vp.max.x - size.x - MARGIN).max(lo.x), (vp.max.y - size.y - MARGIN).max(lo.y));
    pos2(pos.x.clamp(lo.x, hi.x), pos.y.clamp(lo.y, hi.y))
}

/// Keys grow with the zoom the screen governor applied (it shrinks everything to fit a 1280 point
/// desktop width), so a finger still finds them; never below 1, so wide screens keep the design's
/// sizes.
fn touch_scale(ctx: &egui::Context) -> f32 {
    (1.0 / ctx.zoom_factor().max(0.3)).sqrt().clamp(1.0, 1.3)
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

pub(crate) struct VirtualKeyboard {
    mode: Mode,
    /// The expanded mode a tap on the bubble returns to.
    expanded: Mode,
    section: Section,
    mods: Mods,
    caps: bool,
    /// 55 to 100.
    opacity_pct: f32,
    status: String,
    scale: f32,
    panel_pos: Pos2,
    /// Still in its initial lower-left place (follows the screen until the first drag).
    anchored: bool,
    panel_size: Vec2,
    bubble_pos: Pos2,
    dock: Dock,
    bubble_travel: f32,
    /// One entry per tap: its press, text and release.
    queue: VecDeque<Vec<Event>>,
    /// A batch has been fed to a pass that has not run yet.
    in_flight: bool,
    /// The widget that had keyboard focus before the pointer went down (see [`Self::tap`]).
    focus: Option<Id>,
}

impl Default for VirtualKeyboard {
    fn default() -> Self {
        Self {
            mode: Mode::Compact,
            expanded: Mode::Compact,
            section: Section::Main,
            mods: Mods::default(),
            caps: false,
            opacity_pct: 100.0,
            status: FIRST_STATUS.to_owned(),
            scale: 1.0,
            panel_pos: Pos2::ZERO,
            anchored: true,
            panel_size: Vec2::ZERO,
            bubble_pos: Pos2::ZERO,
            dock: Dock::Left,
            bubble_travel: 0.0,
            queue: VecDeque::new(),
            in_flight: false,
            focus: None,
        }
    }
}

impl VirtualKeyboard {
    /// Add the next queued tap to this frame's raw input. Call it after the app's own hook. Like the
    /// app's synthetic input, one batch goes out at a time and the next waits until a UI pass has
    /// applied it ([`Self::show`] notes that), so a press and its release never split across passes
    /// and eframe cannot apply two taps as one burst.
    pub(crate) fn feed_raw_input(&mut self, raw_input: &mut egui::RawInput) {
        if self.in_flight {
            return;
        }
        if let Some(batch) = self.queue.pop_front() {
            raw_input.events.extend(batch);
            self.in_flight = true;
        }
    }

    /// Draw the overlay (call it after the editor's UI) and queue the events of this frame's taps.
    pub(crate) fn show(&mut self, ctx: &egui::Context) {
        // The pass that is running now applied any batch fed to it.
        self.in_flight = false;
        let vp = ctx.content_rect();
        if vp.width() < 80.0 || vp.height() < 80.0 {
            return;
        }
        // Remember which widget has keyboard focus while the pointer is idle: a tap on a virtual key
        // is a press on something else, which makes egui drop a text field's focus (see `tap`).
        if ctx.input(|i| !i.pointer.any_down() && !i.pointer.any_released()) {
            self.focus = ctx.memory(|m| m.focused());
        }
        self.scale = touch_scale(ctx);
        if self.mode == Mode::Bubble {
            self.show_bubble(ctx, vp);
        } else {
            self.show_panel(ctx, vp);
        }
        if !self.queue.is_empty() {
            ctx.request_repaint();
        }
    }

    fn s(&self) -> f32 {
        self.scale
    }

    fn panel_width(&self, mode: Mode, vp: Rect) -> f32 {
        let target = if mode == Mode::Full { 660.0 } else { 352.0 } * self.s();
        target.min(vp.width() - 2.0 * MARGIN).max(1.0)
    }

    // ----- taps ---------------------------------------------------------------------------------

    fn tap(&mut self, ctx: &egui::Context, act: Act) {
        match act {
            Act::Nothing => {}
            Act::Press(code) => self.send(ctx, code, false),
            Act::Ctrl(key) => self.send(ctx, Code::named(key), true),
            Act::Toggle(m) => {
                self.mods.toggle(m);
                self.status = self.mods.describe().map_or_else(|| "Modifiers cleared".to_owned(), |d| format!("Holding {d}"));
            }
            Act::CapsLock => {
                self.caps = !self.caps;
                self.status = if self.caps { "Caps Lock on" } else { "Caps Lock off" }.to_owned();
            }
            Act::ClearModifiers => {
                self.mods = Mods::default();
                self.status = "All modifier keys cleared".to_owned();
            }
            Act::Switch(mode) => {
                self.mode = mode;
                self.expanded = mode;
                self.status = if mode == Mode::Full { "Full keyboard" } else { "Control pad" }.to_owned();
            }
            Act::Show(section) => self.section = section,
        }
    }

    fn send(&mut self, ctx: &egui::Context, code: Code, with_ctrl: bool) {
        let mut mods = self.mods;
        mods.ctrl |= with_ctrl;
        let combo = mods.describe().map_or_else(|| code.name.to_owned(), |m| format!("{m} + {}", code.name));
        let Some(events) = tap_events(code, mods, self.caps) else {
            self.status = format!("{combo}: egui has no such key, not sent");
            return;
        };
        if self.queue.len() >= QUEUE_LIMIT {
            self.queue.pop_front();
        }
        self.queue.push_back(events);
        self.status = format!("{combo} sent");
        // Modifiers apply to the next key only; Caps Lock stays latched.
        self.mods = Mods::default();
        // The press on this button made egui drop the focus of the text field that was being typed
        // in. Give it back so the key reaches it, as a hardware key would.
        if let Some(id) = self.focus {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        ctx.request_repaint();
    }

    // ----- bubble -------------------------------------------------------------------------------

    fn show_bubble(&mut self, ctx: &egui::Context, vp: Rect) {
        let s = self.s();
        let d = 50.0 * s;
        let top = vp.min.y + MARGIN;
        let bottom = (vp.max.y - d - MARGIN).max(top);
        let x = match self.dock {
            Dock::Left => vp.min.x,
            Dock::Right => vp.max.x - d,
            Dock::Free => self.bubble_pos.x.clamp(vp.min.x, (vp.max.x - d).max(vp.min.x)),
        };
        self.bubble_pos = pos2(x, self.bubble_pos.y.clamp(top, bottom));
        let dock = self.dock;
        let area = egui::Area::new(Id::new(BUBBLE_ID)).order(Order::Foreground).fixed_pos(self.bubble_pos).show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(vec2(d, d), Sense::click_and_drag());
            paint_bubble(ui, rect, &resp, dock, s);
            resp
        });
        ctx.move_to_top(area.response.layer_id);
        let resp = area.inner;
        if resp.drag_started() {
            self.bubble_travel = 0.0;
        }
        if resp.dragged() {
            let delta = resp.drag_delta();
            self.bubble_travel += delta.length();
            self.bubble_pos += delta;
            self.dock = Dock::Free;
            ctx.request_repaint();
        }
        if resp.drag_stopped() {
            if self.bubble_travel < TAP_SLOP * s {
                self.open_from_bubble(vp);
            } else {
                self.dock = if self.bubble_pos.x + d / 2.0 < vp.center().x { Dock::Left } else { Dock::Right };
                self.status = if self.dock == Dock::Left { "Bubble docked to left edge" } else { "Bubble docked to right edge" }.to_owned();
                ctx.request_repaint();
            }
        } else if resp.clicked() {
            self.open_from_bubble(vp);
        }
    }

    /// Open the panel beside the bubble (its bottom edge level with the bubble's), clamped to the screen.
    fn open_from_bubble(&mut self, vp: Rect) {
        let s = self.s();
        let (w, h) = (self.panel_width(self.expanded, vp), if self.panel_size.y > 0.0 { self.panel_size.y } else { 380.0 * s });
        let x = if self.dock == Dock::Right { vp.max.x - w - MARGIN } else { self.bubble_pos.x + MARGIN };
        self.panel_pos = clamp_pos(pos2(x, self.bubble_pos.y + 50.0 * s - h), vec2(w, h), vp);
        self.anchored = false;
        self.mode = self.expanded;
        self.status = "Keyboard open".to_owned();
    }

    /// Collapse the panel into a bubble on the nearer screen edge, at the panel's height.
    fn minimize(&mut self, panel: Rect, vp: Rect) {
        let d = 50.0 * self.s();
        self.dock = if panel.center().x < vp.center().x { Dock::Left } else { Dock::Right };
        self.bubble_pos = pos2(if self.dock == Dock::Left { vp.min.x } else { vp.max.x - d }, panel.center().y - d / 2.0);
        self.mode = Mode::Bubble;
        self.status = if self.dock == Dock::Left { "Bubble docked to left edge" } else { "Bubble docked to right edge" }.to_owned();
    }

    // ----- panel --------------------------------------------------------------------------------

    fn show_panel(&mut self, ctx: &egui::Context, vp: Rect) {
        let s = self.s();
        let full = self.mode == Mode::Full;
        let outer_w = self.panel_width(self.mode, vp);
        let pad = (9.0 * s).round();
        let inner_w = (outer_w - 2.0 * pad - 2.0).max(1.0);
        let guess = vec2(outer_w, if self.panel_size.y > 0.0 { self.panel_size.y } else { 380.0 * s });
        let want = if self.anchored { pos2(vp.min.x + MARGIN, vp.max.y - MARGIN - guess.y) } else { self.panel_pos };
        self.panel_pos = clamp_pos(want, guess, vp);
        // The body scrolls when the screen is too short for it; the header never does.
        let body_max = (vp.height() - 2.0 * MARGIN - 2.0 * pad - 2.0 - 29.0 * s - 6.0 * s).max(60.0);
        let mut minimize = false;
        let area = egui::Area::new(Id::new(PANEL_ID)).order(Order::Foreground).fixed_pos(self.panel_pos).show(ctx, |ui| {
            ui.set_opacity(self.opacity_pct / 100.0);
            egui::Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, PANEL_BORDER))
                .corner_radius(15.0 * s)
                .inner_margin(egui::Margin::same(pad as i8))
                .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(110) })
                .show(ui, |ui| {
                    ui.set_width(inner_w);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    minimize = self.header(ui, inner_w, vp, full);
                    ui.add_space(6.0 * s);
                    ui.push_id(full, |ui| {
                        egui::ScrollArea::vertical().max_height(body_max).auto_shrink([false, true]).show(ui, |ui| {
                            if full {
                                self.full_body(ui, inner_w);
                            } else {
                                self.compact_body(ui, inner_w);
                            }
                        });
                    });
                });
        });
        let rect = area.response.rect;
        self.panel_size = rect.size();
        ctx.move_to_top(area.response.layer_id);
        if minimize {
            self.minimize(rect, vp);
        }
    }

    fn header(&mut self, ui: &mut Ui, w: f32, vp: Rect, full: bool) -> bool {
        let s = self.s();
        let bar = alloc(ui, w, 29.0 * s);
        let (bw, bh, gap) = (27.0 * s, 25.0 * s, 5.0 * s);
        let y = bar.min.y + (bar.height() - bh) / 2.0 - 2.0 * s;
        let button = |i: usize| Rect::from_min_size(pos2(bar.max.x - (i + 1) as f32 * bw - i as f32 * gap, y), vec2(bw, bh));
        let (close, mini, expand) = (button(0), button(1), button(2));
        // Only the part left of the buttons drags the panel, so the buttons stay their own targets.
        let grab = Rect::from_min_max(bar.min, pos2(expand.min.x - gap, bar.max.y));
        let drag = ui.interact(grab, ui.id().with("header-drag"), Sense::click_and_drag());
        if drag.dragged() {
            self.panel_pos = clamp_pos(self.panel_pos + drag.drag_delta(), self.panel_size, vp);
            self.anchored = false;
            ui.ctx().request_repaint();
        }

        let p = ui.painter().clone();
        let cy = bar.center().y - 2.0 * s;
        for col in 0..2 {
            for row in 0..3 {
                p.circle_filled(pos2(bar.min.x + (5.0 + col as f32 * 4.5) * s, cy + (row as f32 - 1.0) * 4.5 * s), 1.2 * s, GRIP);
            }
        }
        let brand = p.text(pos2(bar.min.x + 17.0 * s, cy), Align2::LEFT_CENTER, "EFFECTCRAFT KEYS", FontId::proportional(10.0 * s), BRAND);
        let label = if full { "FULL KEYBOARD" } else { "CONTROL PAD" };
        let galley = p.layout_no_wrap(label.to_owned(), FontId::proportional(8.5 * s), PILL_TEXT);
        let pill = Rect::from_min_size(pos2(brand.max.x + 8.0 * s, cy - galley.size().y / 2.0 - 3.0 * s), galley.size() + vec2(12.0 * s, 6.0 * s));
        if pill.max.x <= grab.max.x {
            p.rect_filled(pill, 5.0 * s, PILL_BG);
            p.rect_stroke(pill, 5.0 * s, Stroke::new(1.0, PILL_BORDER), StrokeKind::Inside);
            p.galley(pill.min + vec2(6.0 * s, 3.0 * s), galley, PILL_TEXT);
        }

        let id = ui.id().with("header");
        if icon_button(ui, expand, id.with("expand"), if full { Icon::Shrink } else { Icon::Expand }, s) {
            let mode = if full { Mode::Compact } else { Mode::Full };
            self.tap(ui.ctx(), Act::Switch(mode));
        }
        let minimize = icon_button(ui, mini, id.with("minimize"), Icon::Minimize, s);
        let hide = icon_button(ui, close, id.with("close"), Icon::Close, s);
        minimize || hide
    }

    // ----- compact control pad ------------------------------------------------------------------

    fn compact_body(&mut self, ui: &mut Ui, w: f32) {
        let s = self.s();
        let gap = 4.0 * s;
        self.key_row(ui, w, 33.0 * s, gap, &modifier_row(false), 0);

        // SYSTEM / EDIT and NAVIGATION boxes side by side.
        ui.add_space(7.0 * s);
        let box_gap = 7.0 * s;
        let box_h = 94.0 * s;
        let boxes = alloc(ui, w, box_h);
        let bw = (w - box_gap) / 2.0;
        let left = Rect::from_min_size(boxes.min, vec2(bw, box_h));
        let right = Rect::from_min_size(pos2(boxes.min.x + bw + box_gap, boxes.min.y), vec2(bw, box_h));
        let system = vec![
            at(0, 0, K::util("Esc", 1.0, Key::Escape)),
            at(1, 0, K::util("Tab", 1.0, Key::Tab)),
            at(2, 0, K::util("Enter", 1.0, Key::Enter)),
            at(0, 1, K::util("Bksp", 1.0, Key::Backspace)),
            at(1, 1, K::util("Del", 1.0, Key::Delete)),
            at(2, 1, K::code("Space", 1.0, Kind::Utility, Code::printable(Key::Space, ' ', ' '))),
        ];
        let arrows = vec![
            at(1, 0, K::arrow(Dir::Up, Key::ArrowUp)),
            at(0, 1, K::arrow(Dir::Left, Key::ArrowLeft)),
            at(1, 1, K::arrow(Dir::Down, Key::ArrowDown)),
            at(2, 1, K::arrow(Dir::Right, Key::ArrowRight)),
        ];
        for (i, (rect, title, items)) in [(left, "SYSTEM / EDIT", system), (right, "NAVIGATION", arrows)].into_iter().enumerate() {
            let p = ui.painter();
            p.rect_filled(rect, 10.0 * s, BOX_BG);
            p.rect_stroke(rect, 10.0 * s, Stroke::new(1.0, BOX_BORDER), StrokeKind::Inside);
            let inner = rect.shrink(7.0 * s);
            p.text(inner.min + vec2(2.0 * s, 0.0), Align2::LEFT_TOP, title, FontId::proportional(8.0 * s), SECTION);
            let grid = Rect::from_min_max(pos2(inner.min.x, inner.min.y + 14.0 * s), inner.max);
            self.key_grid(ui, grid, 3, 2, gap, &items, 10 + i as u32);
        }

        self.section_label(ui, w, "FUNCTION KEYS");
        let fns = alloc(ui, w, 2.0 * 27.0 * s + gap);
        let items: Vec<Item> = (0..12).map(|i| at(i % 6, i / 6, fkey(i + 1))).collect();
        self.key_grid(ui, fns, 6, 2, gap, &items, 20);

        self.section_label(ui, w, "QUICK SHORTCUTS");
        let quick = [
            K::quick("Undo", "Ctrl Z", Key::Z),
            K::quick("Redo", "Ctrl Y", Key::Y),
            K::quick("Save", "Ctrl S", Key::S),
            K::quick("Select", "Ctrl A", Key::A),
        ];
        self.key_row(ui, w, 33.0 * s, gap, &quick, 30);

        self.bottom_bar(ui, w, false);
    }

    // ----- full keyboard ------------------------------------------------------------------------

    fn full_body(&mut self, ui: &mut Ui, w: f32) {
        let s = self.s();
        self.key_row(ui, w, 33.0 * s, 4.0 * s, &modifier_row(true), 0);
        ui.add_space(6.0 * s);
        let tab = |label: &'static str, section: Section| K::new(label, 1.0, Kind::Tab, Act::Show(section));
        let tabs = [
            tab("MAIN KEYBOARD", Section::Main),
            tab("NAVIGATION", Section::Navigation),
            tab("NUMPAD", Section::Numpad),
            tab("EXTRA KEYS", Section::Extra),
        ];
        self.key_row(ui, w, 28.0 * s, 5.0 * s, &tabs, 1);
        ui.add_space(8.0 * s);
        let section = self.section;
        ui.push_id(section as u8, |ui| match section {
            Section::Main => self.main_section(ui, w),
            Section::Navigation => self.navigation_section(ui, w),
            Section::Numpad => self.numpad_section(ui, w),
            Section::Extra => self.extra_section(ui, w),
        });
        self.bottom_bar(ui, w, true);
    }

    fn main_section(&mut self, ui: &mut Ui, w: f32) {
        let s = self.s();
        let gap = 3.5 * s;
        let (tall, short) = (34.0 * s, 27.0 * s);

        let mut row = vec![K::util("Esc", 1.2, Key::Escape), K::gap(0.3)];
        for (n, group) in [(1, 4), (5, 4), (9, 4)] {
            row.extend((n..n + group).map(fkey));
            if n < 9 {
                row.push(K::gap(0.3));
            }
        }
        self.key_row(ui, w, short, gap, &row, 0);

        ui.add_space(gap);
        let mut row = vec![punct(Key::Backtick, 1.0)];
        row.extend((1..=9).chain(std::iter::once(0)).map(digit));
        row.extend([punct(Key::Minus, 1.0), punct(Key::Equals, 1.0), K::util("Backspace", 2.0, Key::Backspace)]);
        self.key_row(ui, w, tall, gap, &row, 1);

        ui.add_space(gap);
        let mut row = vec![K::util("Tab", 1.5, Key::Tab)];
        row.extend(letters("qwertyuiop"));
        row.extend([punct(Key::OpenBracket, 1.0), punct(Key::CloseBracket, 1.0), punct(Key::Backslash, 1.5)]);
        self.key_row(ui, w, tall, gap, &row, 2);

        ui.add_space(gap);
        let mut row = vec![K::new("Caps", 1.8, Kind::Utility, Act::CapsLock)];
        row.extend(letters("asdfghjkl"));
        row.extend([punct(Key::Semicolon, 1.0), punct(Key::Quote, 1.0), K::util("Enter", 2.2, Key::Enter)]);
        self.key_row(ui, w, tall, gap, &row, 3);

        ui.add_space(gap);
        let mut row = vec![K::modifier("Shift", 2.3, Mod::Shift)];
        row.extend(letters("zxcvbnm"));
        row.extend([punct(Key::Comma, 1.0), punct(Key::Period, 1.0), punct(Key::Slash, 1.0), K::modifier("Shift", 2.3, Mod::Shift)]);
        self.key_row(ui, w, tall, gap, &row, 4);

        ui.add_space(gap);
        let row = [
            K::modifier("Ctrl", 1.3, Mod::Ctrl),
            K::modifier("Win", 1.1, Mod::Meta),
            K::modifier("Alt", 1.1, Mod::Alt),
            space(6.0),
            K::modifier("Alt", 1.1, Mod::Alt),
            K::missing("Menu", 1.1, "Context Menu"),
            K::modifier("Ctrl", 1.3, Mod::Ctrl),
        ];
        self.key_row(ui, w, tall, gap, &row, 5);
    }

    fn navigation_section(&mut self, ui: &mut Ui, w: f32) {
        let s = self.s();
        self.section_label(ui, w, "EDITING & NAVIGATION");
        let keys = [
            K::util("Insert", 1.0, Key::Insert),
            K::util("Home", 1.0, Key::Home),
            K::util("Page Up", 1.0, Key::PageUp),
            K::util("Delete", 1.0, Key::Delete),
            K::util("End", 1.0, Key::End),
            K::util("Page Down", 1.0, Key::PageDown),
            K::missing("Print Scr", 1.0, "Print Screen"),
            K::missing("Pause", 1.0, "Pause"),
            K::missing("Scroll Lock", 1.0, "Scroll Lock"),
            K::missing("Num Lock", 1.0, "Num Lock"),
            K::util("Escape", 1.0, Key::Escape),
            K::util("Tab", 1.0, Key::Tab),
        ];
        let items: Vec<Item> = keys.iter().enumerate().map(|(i, k)| at(i % 4, i / 4, *k)).collect();
        let area = alloc(ui, w, 3.0 * 36.0 * s + 2.0 * 5.0 * s);
        self.key_grid(ui, area, 4, 3, 5.0 * s, &items, 0);

        self.section_label(ui, w, "ARROW KEYS");
        let pad_w = (190.0 * s).min(w);
        let row = alloc(ui, w, 2.0 * 36.0 * s + 4.0 * s);
        let area = Rect::from_min_size(pos2(row.center().x - pad_w / 2.0, row.min.y), vec2(pad_w, row.height()));
        let items = vec![
            at(1, 0, K::arrow(Dir::Up, Key::ArrowUp)),
            at(0, 1, K::arrow(Dir::Left, Key::ArrowLeft)),
            at(1, 1, K::arrow(Dir::Down, Key::ArrowDown)),
            at(2, 1, K::arrow(Dir::Right, Key::ArrowRight)),
        ];
        self.key_grid(ui, area, 3, 2, 4.0 * s, &items, 1);
    }

    fn numpad_section(&mut self, ui: &mut Ui, w: f32) {
        let s = self.s();
        self.section_label(ui, w, "NUMERIC KEYPAD");
        let pad_w = (320.0 * s).min(w);
        let row = alloc(ui, w, 5.0 * 43.0 * s + 4.0 * 5.0 * s);
        let area = Rect::from_min_size(pos2(row.center().x - pad_w / 2.0, row.min.y), vec2(pad_w, row.height()));
        // egui has no numpad keys: each key is the main block's key without a physical position,
        // as winit reports the numpad to egui. `*` has no key at all and arrives as text, which
        // is how the app's Add Marker shortcut expects it.
        let num = |label: &'static str, key: Key, c: char| K::pad(label, key, c);
        let items: Vec<Item> = vec![
            at(0, 0, K::missing("Num Lock", 1.0, "Num Lock")),
            at(1, 0, K::pad("/", Key::Slash, '/')),
            at(2, 0, K::code("*", 1.0, Kind::Normal, Code::text_only("*", '*'))),
            at(3, 0, K::pad("-", Key::Minus, '-')),
            at(0, 1, num("7", Key::Num7, '7')),
            at(1, 1, num("8", Key::Num8, '8')),
            at(2, 1, num("9", Key::Num9, '9')),
            at(3, 1, K::pad("+", Key::Plus, '+')),
            at(0, 2, num("4", Key::Num4, '4')),
            at(1, 2, num("5", Key::Num5, '5')),
            at(2, 2, num("6", Key::Num6, '6')),
            (3, 2, 1, 2, K::code("Enter", 1.0, Kind::Utility, Code { physical: false, ..Code::named(Key::Enter) })),
            at(0, 3, num("1", Key::Num1, '1')),
            at(1, 3, num("2", Key::Num2, '2')),
            at(2, 3, num("3", Key::Num3, '3')),
            (0, 4, 2, 1, num("0", Key::Num0, '0')),
            at(2, 4, K::pad(".", Key::Period, '.')),
            at(3, 4, K::pad("=", Key::Equals, '=')),
        ];
        self.key_grid(ui, area, 4, 5, 5.0 * s, &items, 0);
    }

    fn extra_section(&mut self, ui: &mut Ui, w: f32) {
        let s = self.s();
        self.section_label(ui, w, "EXTENDED FUNCTION KEYS");
        let mut keys: Vec<K> = (13..=24).map(fkey).collect();
        keys.extend([
            K::missing("Print Screen", 1.0, "Print Screen"),
            K::missing("Pause", 1.0, "Pause"),
            K::missing("Scroll Lock", 1.0, "Scroll Lock"),
            K::missing("Menu", 1.0, "Context Menu"),
        ]);
        let items: Vec<Item> = keys.iter().enumerate().map(|(i, k)| at(i % 4, i / 4, *k)).collect();
        let area = alloc(ui, w, 4.0 * 36.0 * s + 3.0 * 5.0 * s);
        self.key_grid(ui, area, 4, 4, 5.0 * s, &items, 0);
    }

    // ----- shared pieces ------------------------------------------------------------------------

    fn section_label(&self, ui: &mut Ui, w: f32, text: &str) {
        let s = self.s();
        ui.add_space(9.0 * s);
        let r = alloc(ui, w, 10.0 * s);
        ui.painter().text(r.min + vec2(2.0 * s, 0.0), Align2::LEFT_TOP, text, FontId::proportional(8.5 * s), SECTION);
        ui.add_space(4.0 * s);
    }

    /// Opacity slider, Clear Modifiers (and Control Pad in full mode) and the status line.
    fn bottom_bar(&mut self, ui: &mut Ui, w: f32, full: bool) {
        let s = self.s();
        ui.add_space(8.0 * s);
        let row = alloc(ui, w, 27.0 * s);
        let gap = 6.0 * s;
        let mut buttons = vec![K::new("Clear modifiers", 1.0, Kind::Utility, Act::ClearModifiers)];
        let mut widths = vec![(w * 0.3).clamp(78.0 * s, 104.0 * s)];
        if full {
            buttons.push(K::new("Control Pad", 1.0, Kind::Accent, Act::Switch(Mode::Compact)));
            widths.push(86.0 * s);
        }
        let mut right = row.max.x;
        for (i, (k, bw)) in buttons.iter().zip(&widths).enumerate() {
            let rect = Rect::from_min_size(pos2(right - bw, row.min.y), vec2(*bw, row.height()));
            let id = ui.id().with(("bottom", i));
            self.key(ui, rect, id, k);
            right = rect.min.x - gap;
        }
        let label_w = 46.0 * s;
        ui.painter().text(pos2(row.min.x + 2.0 * s, row.center().y), Align2::LEFT_CENTER, "Opacity", FontId::proportional(9.0 * s), STATUS);
        let slider = Rect::from_min_max(pos2(row.min.x + label_w, row.min.y), pos2(right.max(row.min.x + label_w + 20.0), row.max.y));
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = (slider.width() - 8.0 * s).max(20.0);
            ui.put(slider, egui::Slider::new(&mut self.opacity_pct, 55.0..=100.0).show_value(false));
        });

        ui.add_space(4.0 * s);
        let line = alloc(ui, w, 16.0 * s);
        let clipped = ui.painter().with_clip_rect(line);
        clipped.text(pos2(line.min.x + 2.0 * s, line.center().y), Align2::LEFT_CENTER, &self.status, FontId::proportional(9.0 * s), STATUS);
    }

    /// A row of buttons sharing `w` by weight.
    fn key_row(&mut self, ui: &mut Ui, w: f32, h: f32, gap: f32, keys: &[K], salt: u32) {
        let row = alloc(ui, w, h);
        let total: f32 = keys.iter().map(|k| k.w).sum();
        let unit = (row.width() - gap * keys.len().saturating_sub(1) as f32).max(0.0) / total.max(0.1);
        let mut x = row.min.x;
        for (i, k) in keys.iter().enumerate() {
            let rect = Rect::from_min_size(pos2(x, row.min.y), vec2(k.w * unit, h));
            x += k.w * unit + gap;
            let id = ui.id().with((salt, i));
            self.key(ui, rect, id, k);
        }
    }

    fn key_grid(&mut self, ui: &mut Ui, area: Rect, cols: usize, rows: usize, gap: f32, items: &[Item], salt: u32) {
        for (i, (col, row, cs, rs, k)) in items.iter().enumerate() {
            let rect = cell(area, cols, rows, gap, (*col, *row), (*cs, *rs));
            let id = ui.id().with((salt, i));
            self.key(ui, rect, id, k);
        }
    }

    /// Draw one button and run its action when it is tapped.
    fn key(&mut self, ui: &mut Ui, rect: Rect, id: Id, k: &K) {
        if matches!(k.act, Act::Nothing) {
            return;
        }
        let resp = ui.interact(rect, id, Sense::click());
        let active = match k.act {
            Act::Toggle(m) => self.mods.get(m),
            Act::CapsLock => self.caps,
            Act::Show(section) => self.section == section,
            _ => false,
        };
        paint_key(ui, rect, &resp, k, active, self.s());
        if resp.clicked() {
            self.tap(ui.ctx(), k.act);
        }
    }
}

fn modifier_row(full: bool) -> [K; 4] {
    [
        K::modifier("Ctrl", 1.0, Mod::Ctrl),
        K::modifier("Shift", 1.0, Mod::Shift),
        K::modifier("Alt", 1.0, Mod::Alt),
        K::modifier(if full { "Win / Meta" } else { "Win" }, 1.0, Mod::Meta),
    ]
}

/// A `w` wide, `h` tall slot in the layout (the panel places its own rows).
fn alloc(ui: &mut Ui, w: f32, h: f32) -> Rect {
    ui.allocate_space(vec2(w, h)).1
}

// ---------------------------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------------------------

fn paint_key(ui: &Ui, rect: Rect, resp: &egui::Response, k: &K, active: bool, s: f32) {
    let (mut fill, border, text) = match (k.kind, active) {
        (Kind::Tab, true) => (TAB_ON, TAB_ON_BORDER, Color32::WHITE),
        (_, true) => (ACTIVE, ACTIVE_BORDER, Color32::WHITE),
        (Kind::Normal, false) => (KEY, KEY_BORDER, TEXT),
        (Kind::Utility, false) => (UTILITY, KEY_BORDER, UTILITY_TEXT),
        (Kind::Modifier, false) => (MODIFIER, MODIFIER_BORDER, MODIFIER_TEXT),
        (Kind::Accent, false) => (ACCENT, ACCENT_BORDER, TEXT),
        (Kind::Tab, false) => (TAB, TAB_BORDER, TAB_TEXT),
    };
    let down = resp.is_pointer_button_down_on();
    if down && !active {
        fill = KEY_DOWN;
    }
    let rect = if down { rect.translate(vec2(0.0, s)) } else { rect };
    let radius = 7.0 * s;
    let p = ui.painter();
    p.rect_filled(rect, radius, fill);
    p.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    if let Some(dir) = k.arrow {
        paint_arrow(p, rect.center(), dir, rect.height().min(rect.width()) * 0.2, text);
        return;
    }
    let size = if rect.height() >= 30.0 * s { 10.5 } else { 9.5 } * s;
    match k.sub {
        Some(sub) => {
            p.text(rect.center() - vec2(0.0, 6.0 * s), Align2::CENTER_CENTER, k.label, FontId::proportional(size), text);
            p.text(rect.center() + vec2(0.0, 7.0 * s), Align2::CENTER_CENTER, sub, FontId::proportional(8.0 * s), text.gamma_multiply(0.7));
        }
        None => {
            let avail = (rect.width() - 4.0).max(1.0);
            let mut galley = p.layout_no_wrap(k.label.to_owned(), FontId::proportional(size), text);
            if galley.size().x > avail {
                let shrunk = (size * avail / galley.size().x).max(5.5);
                galley = p.layout_no_wrap(k.label.to_owned(), FontId::proportional(shrunk), text);
            }
            let pos = rect.center() - galley.size() / 2.0;
            p.galley(pos, galley, text);
        }
    }
}

fn paint_arrow(p: &egui::Painter, c: Pos2, dir: Dir, r: f32, color: Color32) {
    let points = match dir {
        Dir::Up => [pos2(c.x, c.y - r), pos2(c.x - r, c.y + r * 0.7), pos2(c.x + r, c.y + r * 0.7)],
        Dir::Down => [pos2(c.x, c.y + r), pos2(c.x - r, c.y - r * 0.7), pos2(c.x + r, c.y - r * 0.7)],
        Dir::Left => [pos2(c.x - r, c.y), pos2(c.x + r * 0.7, c.y - r), pos2(c.x + r * 0.7, c.y + r)],
        Dir::Right => [pos2(c.x + r, c.y), pos2(c.x - r * 0.7, c.y - r), pos2(c.x - r * 0.7, c.y + r)],
    };
    p.add(Shape::convex_polygon(points.to_vec(), color, Stroke::NONE));
}

#[derive(Clone, Copy)]
enum Icon {
    Expand,
    Shrink,
    Minimize,
    Close,
}

fn icon_button(ui: &Ui, rect: Rect, id: Id, icon: Icon, s: f32) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 7.0 * s, if resp.is_pointer_button_down_on() { ICON_DOWN } else { ICON_BG });
    p.rect_stroke(rect, 7.0 * s, Stroke::new(1.0, ICON_BORDER), StrokeKind::Inside);
    let c = rect.center();
    let stroke = Stroke::new(1.5, ICON_FG);
    let line = |a: (f32, f32), b: (f32, f32)| p.line_segment([c + vec2(a.0, a.1) * s, c + vec2(b.0, b.1) * s], stroke);
    match icon {
        Icon::Minimize => line((-5.0, 0.0), (5.0, 0.0)),
        Icon::Close => {
            line((-4.0, -4.0), (4.0, 4.0));
            line((-4.0, 4.0), (4.0, -4.0));
        }
        Icon::Expand => {
            line((-5.0, 5.0), (5.0, -5.0));
            line((5.0, -5.0), (1.0, -5.0));
            line((5.0, -5.0), (5.0, -1.0));
            line((-5.0, 5.0), (-1.0, 5.0));
            line((-5.0, 5.0), (-5.0, 1.0));
        }
        Icon::Shrink => {
            line((-5.0, 5.0), (-1.0, 1.0));
            line((5.0, -5.0), (1.0, -1.0));
            line((1.0, -1.0), (5.0, -1.0));
            line((1.0, -1.0), (1.0, -5.0));
            line((-1.0, 1.0), (-5.0, 1.0));
            line((-1.0, 1.0), (-1.0, 5.0));
        }
    }
    resp.clicked()
}

fn paint_bubble(ui: &Ui, rect: Rect, resp: &egui::Response, dock: Dock, s: f32) {
    // A circle, with the corners on the docked edge flattened.
    let round = (rect.height() / 2.0).round() as u8;
    let flat = (6.0 * s).round() as u8;
    let (l, r) = match dock {
        Dock::Left => (flat, round),
        Dock::Right => (round, flat),
        Dock::Free => (round, round),
    };
    let corners = CornerRadius { nw: l, sw: l, ne: r, se: r };
    let p = ui.painter();
    p.rect_filled(rect.translate(vec2(0.0, 3.0 * s)), corners, Color32::from_black_alpha(90));
    p.rect_filled(rect, corners, if resp.is_pointer_button_down_on() || resp.dragged() { BUBBLE_DOWN } else { BUBBLE });
    p.rect_stroke(rect, corners, Stroke::new(1.0, BUBBLE_BORDER), StrokeKind::Inside);
    // A keyboard glyph drawn from shapes (no font has to carry one).
    let (kw, kh) = (24.0 * s, 15.0 * s);
    let kb = Rect::from_center_size(rect.center(), vec2(kw, kh));
    p.rect_stroke(kb, 3.0 * s, Stroke::new(1.6, Color32::WHITE), StrokeKind::Inside);
    for row in 0..2 {
        for col in 0..4 {
            p.circle_filled(pos2(kb.min.x + kw * (col + 1) as f32 / 5.0, kb.min.y + kh * (row + 1) as f32 / 4.0), 1.2 * s, Color32::WHITE);
        }
    }
    let bar_y = kb.min.y + kh * 0.75;
    p.line_segment([pos2(kb.center().x - 5.0 * s, bar_y), pos2(kb.center().x + 5.0 * s, bar_y)], Stroke::new(1.4, Color32::WHITE));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool) -> Mods {
        Mods { ctrl, shift, ..Mods::default() }
    }

    fn typed(events: &[Event]) -> Vec<&str> {
        events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect()
    }

    #[test]
    fn ctrl_z_is_a_shortcut_press_and_release_without_text() {
        let events = tap_events(Code::letter(Key::Z, 'z'), mods(true, false), false).unwrap();
        assert_eq!(events.len(), 2);
        let m = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
        assert!(matches!(&events[0], Event::Key { key: Key::Z, pressed: true, repeat: false, modifiers, .. } if *modifiers == m));
        assert!(matches!(&events[1], Event::Key { key: Key::Z, pressed: false, .. }));
        assert!(typed(&events).is_empty());
    }

    #[test]
    fn letters_follow_shift_and_caps_lock() {
        let a = Code::letter(Key::A, 'a');
        assert_eq!(typed(&tap_events(a, mods(false, false), false).unwrap()), ["a"]);
        assert_eq!(typed(&tap_events(a, mods(false, true), false).unwrap()), ["A"]);
        assert_eq!(typed(&tap_events(a, mods(false, false), true).unwrap()), ["A"]);
        assert_eq!(typed(&tap_events(a, mods(false, true), true).unwrap()), ["a"]);
    }

    #[test]
    fn symbols_use_shift_not_caps_lock() {
        let one = Code::printable(Key::Num1, '1', '!');
        assert_eq!(typed(&tap_events(one, mods(false, true), false).unwrap()), ["!"]);
        assert_eq!(typed(&tap_events(one, mods(false, false), true).unwrap()), ["1"]);
    }

    #[test]
    fn text_comes_between_press_and_release() {
        let events = tap_events(Code::letter(Key::Q, 'q'), Mods::default(), false).unwrap();
        assert!(matches!(&events[..], [Event::Key { pressed: true, .. }, Event::Text(t), Event::Key { pressed: false, .. }] if t == "q"));
    }

    #[test]
    fn keys_without_text_send_only_the_key() {
        let events = tap_events(Code::named(Key::Backspace), Mods::default(), false).unwrap();
        assert_eq!(events.len(), 2);
        assert!(typed(&events).is_empty());
    }

    #[test]
    fn ctrl_c_and_x_are_clipboard_events_as_on_a_hardware_keyboard() {
        assert!(matches!(tap_events(Code::letter(Key::C, 'c'), mods(true, false), false).as_deref(), Some([Event::Copy])));
        assert!(matches!(tap_events(Code::letter(Key::X, 'x'), mods(true, false), false).as_deref(), Some([Event::Cut])));
        // With Shift it stays a key press, so the shortcut dispatcher sees Ctrl+Shift+C.
        assert_eq!(tap_events(Code::letter(Key::C, 'c'), mods(true, true), false).map(|e| e.len()), Some(2));
    }

    #[test]
    fn the_numpad_has_no_physical_key_and_star_is_text() {
        let events = tap_events(Code::numpad(Key::Num7, '7'), Mods::default(), false).unwrap();
        assert!(matches!(&events[0], Event::Key { key: Key::Num7, physical_key: None, .. }));
        assert_eq!(typed(&events), ["7"]);
        assert!(matches!(tap_events(Code::text_only("*", '*'), Mods::default(), false).as_deref(), Some([Event::Text(t)]) if t == "*"));
    }

    #[test]
    fn keys_egui_cannot_represent_send_nothing() {
        assert!(tap_events(Code::unsupported("Print Screen"), Mods::default(), false).is_none());
    }

    #[test]
    fn win_sets_only_the_mac_cmd_flag() {
        let m = Mods { meta: true, ..Mods::default() }.to_egui();
        assert!(m.mac_cmd && !m.command && !m.ctrl && !m.alt && !m.shift);
    }

    #[test]
    fn sticky_modifiers_report_their_names() {
        assert_eq!(Mods::default().describe(), None);
        assert_eq!(Mods { ctrl: true, alt: true, ..Mods::default() }.describe().as_deref(), Some("Ctrl + Alt"));
    }

    #[test]
    fn panel_stays_inside_the_screen() {
        let vp = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 400.0));
        let p = clamp_pos(pos2(900.0, 900.0), vec2(352.0, 380.0), vp);
        assert!(p.x + 352.0 <= 800.0 && p.y + 380.0 <= 400.0);
        // Larger than the screen: pinned to the top-left margin, no panic.
        let p = clamp_pos(pos2(50.0, 50.0), vec2(2000.0, 2000.0), vp);
        assert_eq!((p.x, p.y), (MARGIN, MARGIN));
    }
}
