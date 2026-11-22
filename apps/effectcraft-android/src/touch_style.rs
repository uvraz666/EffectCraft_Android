//! Finger-sized grab areas for the desktop interface.
//!
//! The desktop theme sizes panel-resize handles and scroll bars for a mouse pointer. On a phone the
//! layout is zoomed down to a desktop width (see `scale.rs`), which makes them a few pixels wide.
//! This only widens those hit areas; it changes no layout, command or setting. The theme resets
//! the style whenever settings change, so [`apply`] runs every frame after the app's own tick and
//! does nothing while the values are already large enough.

/// Grab radius of a panel edge, in points.
const RESIZE_SIDE: f32 = 14.0;
/// Grab radius of a panel corner, in points.
const RESIZE_CORNER: f32 = 22.0;
/// Width of a scroll bar, in points.
const SCROLL_BAR_WIDTH: f32 = 14.0;
/// Shortest scroll handle, in points, so a long list still leaves something to grab.
const SCROLL_HANDLE_MIN: f32 = 48.0;

pub fn apply(ctx: &egui::Context) {
    let style = ctx.style();
    let done = style.interaction.resize_grab_radius_side >= RESIZE_SIDE
        && style.interaction.resize_grab_radius_corner >= RESIZE_CORNER
        && style.spacing.scroll.bar_width >= SCROLL_BAR_WIDTH
        && style.spacing.scroll.handle_min_length >= SCROLL_HANDLE_MIN;
    if done {
        return;
    }
    ctx.all_styles_mut(|s| {
        let i = &mut s.interaction;
        i.resize_grab_radius_side = i.resize_grab_radius_side.max(RESIZE_SIDE);
        i.resize_grab_radius_corner = i.resize_grab_radius_corner.max(RESIZE_CORNER);
        let scroll = &mut s.spacing.scroll;
        scroll.bar_width = scroll.bar_width.max(SCROLL_BAR_WIDTH);
        scroll.handle_min_length = scroll.handle_min_length.max(SCROLL_HANDLE_MIN);
    });
}
