//! The desktop layout on a phone screen.
//!
//! A phone reports a scale factor of 2.5 to 3.5, which would give the desktop interface a window
//! only 700 to 900 points wide: panels would not fit. Instead the zoom is chosen so the screen's
//! long edge is [`TARGET_POINTS`] points, the width of a normal desktop window, whatever the
//! density. Settings ▸ Appearance ▸ UI Scale (75 to 200 %) multiplies the result, so it is the
//! user's knob for bigger or smaller controls.

/// The screen's long edge in points at 100 % UI Scale (the desktop's default window is 1680
/// wide, its minimum 960).
pub const TARGET_POINTS: f32 = 1280.0;

/// Keeps egui's zoom factor at the value that gives [`TARGET_POINTS`].
#[derive(Default)]
pub struct Governor {
    last: Option<f32>,
}

impl Governor {
    /// Set the zoom for the current screen. Runs every frame after the app's own tick, which
    /// resets the zoom to `ui_scale / 100` whenever settings change; the screen's size in pixels
    /// does not depend on the zoom, so this settles at once and does not oscillate.
    pub fn apply(&mut self, ctx: &egui::Context, ui_scale_percent: u32) {
        let Some(native) = ctx.native_pixels_per_point().filter(|n| n.is_finite() && *n > 0.0) else {
            return;
        };
        let screen = ctx.content_rect().size();
        let long_edge_pixels = screen.x.max(screen.y) * ctx.pixels_per_point();
        let fit = zoom_for(long_edge_pixels, native);
        let want = fit * (ui_scale_percent as f32 / 100.0);
        if !want.is_finite() || want <= 0.0 {
            return;
        }
        if (ctx.zoom_factor() - want).abs() > 1e-3 || self.last != Some(want) {
            ctx.set_zoom_factor(want);
            self.last = Some(want);
        }
    }
}

/// The zoom that makes a screen of `long_edge_pixels` [`TARGET_POINTS`] points long, given the
/// native scale factor. Never zooms in past the native scale (a small screen keeps its density),
/// and never below 0.3.
pub fn zoom_for(long_edge_pixels: f32, native_ppp: f32) -> f32 {
    let wanted_ppp = long_edge_pixels / TARGET_POINTS;
    (wanted_ppp / native_ppp).clamp(0.3, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_gets_a_desktop_width() {
        // 2400 x 1080 at 2.75: 2400 / 1280 = 1.875 points per pixel, a zoom of about 0.68.
        let z = zoom_for(2400.0, 2.75);
        assert!((z - 0.6818).abs() < 1e-3, "{z}");
        assert!((2400.0 / (2.75 * z) - TARGET_POINTS).abs() < 0.5);
    }

    #[test]
    fn a_low_density_screen_is_left_alone() {
        assert_eq!(zoom_for(1280.0, 1.0), 1.0);
        assert_eq!(zoom_for(800.0, 1.0), 0.625);
    }

    #[test]
    fn the_zoom_stays_in_bounds() {
        assert_eq!(zoom_for(100.0, 4.0), 0.3);
        assert_eq!(zoom_for(6000.0, 1.0), 1.0);
    }
}
