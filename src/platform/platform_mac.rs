//! macOS: force the window to be opaque.
//!
//! Slint's winit backend creates the NSWindow with a transparent layer, so the
//! native titlebar has nothing behind it and whatever sits under the app shows
//! through. Slint's `background` property only paints the client area, which
//! does not extend under the titlebar — so the fix has to happen on the NSWindow
//! itself: mark it opaque and give it a background colour.
//!
//! Runs at runtime (not at window construction) because Slint owns the window;
//! main.rs retries on a timer until the window exists, same as the Windows icon.

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSApplication, NSColor,
};

/// Background colour of the window chrome. Must match `base_tlo` (#242532) in
/// ui/colors.slint, or the titlebar reads as a slightly different shade than
/// the app body.
const BG_R: f64 = 0x24 as f64 / 255.0;
const BG_G: f64 = 0x25 as f64 / 255.0;
const BG_B: f64 = 0x32 as f64 / 255.0;

/// Make every application window opaque. Returns true once at least one window
/// was found (i.e. there is no point retrying).
pub fn try_make_windows_opaque() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };

    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    if windows.is_empty() {
        return false; // window not created yet — caller retries
    }

    let background = NSColor::colorWithSRGBRed_green_blue_alpha(BG_R, BG_G, BG_B, 1.0);
    let dark = unsafe { NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua) };

    for window in windows.iter() {
        // Opaque + a background colour: stops the desktop showing through.
        window.setOpaque(true);
        window.setBackgroundColor(Some(&background));

        // ...but the titlebar draws its own NSVisualEffectView material on top,
        // so it would still read as system grey rather than the app's colour.
        // Making it "transparent" lets the window background show through it —
        // which is how you tint a native titlebar to match the UI.
        window.setTitlebarAppearsTransparent(true);

        // Dark appearance so the title text and the traffic-light glyphs stay
        // light against #242532 instead of rendering for a light theme.
        window.setAppearance(dark.as_deref());
    }

    true
}
