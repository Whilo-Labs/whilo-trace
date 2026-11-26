//! The icon in the notification area: with the window closed the app keeps probing. A click on the
//! icon opens the window; its menu opens it or quits. While a target has an alert, the icon has a
//! red dot and its tooltip says at how many targets.

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
#[cfg(windows)]
use tauri::webview::Webview;
use tauri::{AppHandle, Manager, Runtime};

const ID: &str = "main";

/// Brings the main window up: shown, not minimized, in front.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    page_shown(app, true);
}

/// Whether the page of the main window is hidden from WebView2 now.
#[cfg(windows)]
static PAGE_HIDDEN: AtomicBool = AtomicBool::new(false);

/// Tells the page of the main window whether it can be seen. WebView2 does not find out by itself
/// that its window went to the notification area or was minimized (the page stays `visible`), so
/// the screen kept asking the core for data every second and the route map kept drawing. A hidden
/// page (`document.hidden`) asks for nothing until it is seen again, and its timers slow down; the
/// core keeps probing and alerting. Windows only: elsewhere it was not tried in a real window.
pub fn page_shown<R: Runtime>(app: &AppHandle<R>, shown: bool) {
    #[cfg(windows)]
    {
        if PAGE_HIDDEN.swap(!shown, Ordering::SeqCst) == !shown {
            return;
        }
        if let Some(w) = app.get_webview_window("main") {
            let page: &Webview<R> = w.as_ref();
            let _ = if shown { page.show() } else { page.hide() };
        }
    }
    #[cfg(not(windows))]
    let _ = (app, shown);
}

pub fn build<R: Runtime>(app: &tauri::App<R>, lang: &str) -> tauri::Result<()> {
    let en = lang == "en";
    let open = MenuItem::with_id(
        app,
        "open",
        if en {
            "Open Whilo Trace"
        } else {
            "Открыть Whilo Trace"
        },
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        if en { "Quit" } else { "Выйти" },
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;
    let mut tray = TrayIconBuilder::with_id(ID)
        .tooltip("Whilo Trace")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// The icon with a red dot (the color of loss) in its lower right corner, ringed in white.
pub fn with_dot(icon: &Image<'_>) -> Image<'static> {
    let (w, h) = (icon.width(), icon.height());
    let mut px = icon.rgba().to_vec();
    let r = (w.min(h) as f32 * 0.24).max(2.0);
    let (cx, cy) = (w as f32 - r - 1.0, h as f32 - r - 1.0);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let i = ((y * w + x) * 4) as usize;
            if d <= r {
                px[i..i + 4].copy_from_slice(&[0xFF, 0x4F, 0x47, 0xFF]);
            } else if d <= r + 1.5 {
                px[i..i + 4].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
            }
        }
    }
    Image::new_owned(px, w, h)
}

/// "цели" after "1 из" and the like, "целей" otherwise.
fn of_targets(n: usize) -> &'static str {
    if n % 10 == 1 && n % 100 != 11 {
        "цели"
    } else {
        "целей"
    }
}

/// The tooltip of the icon: how many targets, and at how many of them there is trouble.
pub fn tooltip(alerts: usize, targets: usize, lang: &str) -> String {
    match (alerts, lang == "en") {
        (0, true) => format!("Whilo Trace: {targets} targets"),
        (0, false) => format!("Whilo Trace, целей: {targets}"),
        (a, true) => format!("Whilo Trace: trouble at {a} of {targets} targets"),
        (a, false) => format!(
            "Whilo Trace: сбой у {a} из {targets} {}",
            of_targets(targets)
        ),
    }
}

/// Shows on the icon whether any target has an alert.
pub fn set_alerts<R: Runtime>(app: &AppHandle<R>, alerts: usize, targets: usize, lang: &str) {
    let Some(tray) = app.tray_by_id(ID) else {
        return;
    };
    let _ = tray.set_tooltip(Some(tooltip(alerts, targets, lang)));
    if let Some(icon) = app.default_window_icon() {
        let _ = tray.set_icon(Some(if alerts > 0 {
            with_dot(icon)
        } else {
            icon.clone()
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_sits_in_the_corner() {
        let icon = Image::new_owned(vec![0; 32 * 32 * 4], 32, 32);
        let dot = with_dot(&icon);
        let px = |x: u32, y: u32| {
            let i = ((y * 32 + x) * 4) as usize;
            dot.rgba()[i..i + 4].to_vec()
        };
        assert_eq!(
            px(26, 26),
            vec![0xFF, 0x4F, 0x47, 0xFF],
            "red in the corner"
        );
        assert_eq!(px(4, 4), vec![0, 0, 0, 0], "the rest is the icon");
    }

    #[test]
    fn the_tooltip_counts_targets() {
        assert_eq!(tooltip(0, 3, "ru"), "Whilo Trace, целей: 3");
        assert_eq!(tooltip(1, 21, "ru"), "Whilo Trace: сбой у 1 из 21 цели");
        assert_eq!(tooltip(2, 12, "ru"), "Whilo Trace: сбой у 2 из 12 целей");
        assert_eq!(
            tooltip(2, 5, "en"),
            "Whilo Trace: trouble at 2 of 5 targets"
        );
    }
}
