//! Who draws the window frame on Linux.
//!
//! Wayland compositors that draw frames for their clients offer `zxdg_decoration_manager_v1`
//! (KDE, Sway, Hyprland, COSMIC). GNOME doesn't: it leaves the frame to the app, and winit then
//! drew an imitation of GNOME's. Without that protocol PdfCraft turns winit's frame off and draws
//! a header bar like GNOME's own apps (`pdfcraft_ui_egui::window_controls`), with the window
//! buttons GNOME's `button-layout` names. X11 window managers always draw the frame.
//!
//! `PDFCRAFT_TITLEBAR=app` or `=system` overrides the choice.

use pdfcraft_ui_egui::window_controls::Layout;

/// Decide whether the app draws window controls, given the environment.
///
/// This is the single source of truth for the window-frame decision. Both the
/// startup code in `main.rs` and the legacy `app_drawn()` wrapper call this.
pub fn decide_window_controls(
    titlebar_value: Option<&str>,
    is_linux: bool,
    wayland_display: Option<&str>,
    desktop: Option<&str>,
) -> Option<Layout> {
    // Explicit override wins.
    match titlebar_value {
        Some("system") => return None,
        Some("app") => return Some(button_layout()),
        _ => {}
    }

    // Muffin on X11 always gets server-side frame (tiling requires it).
    if is_linux && wayland_display.as_deref().unwrap_or("").is_empty() && x11_muffin_session(desktop) {
        return None;
    }

    // On Linux Wayland, check if the compositor leaves the frame to the app.
    if is_linux && wayland_display.is_some_and(|v| !v.is_empty()) {
        if compositor_leaves_frame_to_app() {
            return Some(button_layout());
        } else {
            return None;
        }
    }

    // Non-Linux Unix without override: no app-drawn controls.
    None
}

/// The window buttons to draw in the app, or `None` when the desktop draws the frame.
///
/// Kept for compatibility; reads the environment and delegates to
/// `decide_window_controls`.
pub(crate) fn app_drawn() -> Option<Layout> {
    let titlebar_value = std::env::var("PDFCRAFT_TITLEBAR").ok().map(|v| v.trim().to_string());
    let titlebar_value = titlebar_value.as_deref().filter(|v| *v == "app" || *v == "system");
    decide_window_controls(
        titlebar_value,
        cfg!(target_os = "linux"),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref().filter(|v| !v.is_empty()),
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
    )
}

/// GNOME's button order, or minimize, maximize and close on the right when it names none.
fn button_layout() -> Layout {
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    if let Some(value) = crate::text_scale::gsettings(&["get", "org.gnome.desktop.wm.preferences", "button-layout"]) {
        let layout = Layout::parse(&value);
        // A layout without a single button would leave no way to close the window by mouse.
        if !layout.left.is_empty() || !layout.right.is_empty() {
            return layout;
        }
    }
    Layout::default()
}

/// Whether the desktop named by `XDG_CURRENT_DESKTOP` is Cinnamon or its Muffin window manager.
/// Muffin tiles windows (#3467) only with the server-side frame, so PdfCraft leaves the frame to it.
pub fn x11_muffin_session(desktop: Option<&str>) -> bool {
    desktop
        .is_some_and(|value| value.split(':').any(|name| matches!(name.trim().to_ascii_lowercase().as_str(), "cinnamon" | "x-cinnamon" | "muffin")))
}

/// Whether this is a Wayland session whose compositor draws no frames.
#[cfg(target_os = "linux")]
fn compositor_leaves_frame_to_app() -> bool {
    use wayland_client::Connection;
    use wayland_client::globals::registry_queue_init;
    // No Wayland compositor: X11, whose window manager draws the frame (winit falls back to it).
    let Ok(conn) = Connection::connect_to_env() else { return false };
    let Ok((globals, _queue)) = registry_queue_init::<Registry>(&conn) else { return false };
    let offered = globals.contents().with_list(|list| list.iter().any(|g| g.interface == "zxdg_decoration_manager_v1"));
    if !offered {
        log::info!("the Wayland compositor draws no window frames; PdfCraft draws its own title bar");
    }
    !offered
}

#[cfg(not(target_os = "linux"))]
fn compositor_leaves_frame_to_app() -> bool {
    false
}

#[cfg(target_os = "linux")]
struct Registry;

#[cfg(target_os = "linux")]
impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for Registry {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_registry::WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}
