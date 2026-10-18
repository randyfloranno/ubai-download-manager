//! System tray icon (Windows): show the window, pause/resume everything, quit.
//!
//! The tray only reports [`TrayAction`]s; the app turns them into commands.
//! Other platforms get a stub until a tray backend without copyleft
//! dependencies is approved (see `docs/en/gaps.md`).

use std::sync::Arc;

use udm_i18n::Localizer;

use crate::bridge::Bridge;

/// What the user picked in the tray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    PauseAll,
    ResumeAll,
    Quit,
}

/// Menu entries in display order with their i18n keys.
const ITEMS: [(TrayAction, &str); 4] = [
    (TrayAction::Show, "ui-tray-show"),
    (TrayAction::PauseAll, "cmd-download-pause-all"),
    (TrayAction::ResumeAll, "cmd-download-resume-all"),
    (TrayAction::Quit, "ui-tray-quit"),
];

#[cfg(windows)]
mod imp {
    use std::sync::{Arc, mpsc};

    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };
    use udm_i18n::Localizer;

    use super::{ITEMS, TrayAction};
    use crate::bridge::Bridge;

    /// Tray icon edge in pixels; Windows scales it for the notification area.
    const ICON_PX: u16 = 32;

    pub struct Tray {
        icon: TrayIcon,
        items: Vec<(MenuItem, &'static str)>,
        rx: mpsc::Receiver<TrayAction>,
        tooltip: String,
    }

    impl Tray {
        pub fn new(loc: &Localizer, bridge: Arc<Bridge>) -> Option<Self> {
            let rgba = udm_core::logo::rgba(ICON_PX)?;
            let icon = Icon::from_rgba(rgba, u32::from(ICON_PX), u32::from(ICON_PX)).ok()?;
            let menu = Menu::new();
            let mut items = Vec::new();
            let mut ids: Vec<(MenuId, TrayAction)> = Vec::new();
            for (action, key) in ITEMS {
                if action == TrayAction::Quit {
                    menu.append(&PredefinedMenuItem::separator()).ok()?;
                }
                let item = MenuItem::new(loc.tr(key), true, None);
                menu.append(&item).ok()?;
                ids.push((item.id().clone(), action));
                items.push((item, key));
            }
            let title = loc.tr("app-title");
            let icon = TrayIconBuilder::new()
                .with_icon(icon)
                .with_tooltip(&title)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .ok()?;

            let (tx, rx) = mpsc::channel();
            let menu_tx = tx.clone();
            let menu_wake = Arc::clone(&bridge);
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                if let Some((_, a)) = ids.iter().find(|(id, _)| *id == e.id) {
                    let _ = menu_tx.send(*a);
                    menu_wake.wake();
                }
            }));
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                let show = matches!(
                    e,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick { .. }
                );
                if show {
                    let _ = tx.send(TrayAction::Show);
                    bridge.wake();
                }
            }));
            Some(Self {
                icon,
                items,
                rx,
                tooltip: title,
            })
        }

        pub fn actions(&self) -> Vec<TrayAction> {
            self.rx.try_iter().collect()
        }

        pub fn set_tooltip(&mut self, text: String) {
            if text != self.tooltip {
                if let Err(e) = self.icon.set_tooltip(Some(&text)) {
                    tracing::debug!("tray tooltip: {e}");
                }
                self.tooltip = text;
            }
        }

        pub fn relabel(&self, loc: &Localizer) {
            for (item, key) in &self.items {
                item.set_text(loc.tr(key));
            }
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            // The handlers hold channel senders; release them with the icon.
            MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
            TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::Arc;

    use udm_i18n::Localizer;

    use super::TrayAction;
    use crate::bridge::Bridge;

    pub struct Tray;

    impl Tray {
        pub fn new(_loc: &Localizer, _bridge: Arc<Bridge>) -> Option<Self> {
            None
        }

        pub fn actions(&self) -> Vec<TrayAction> {
            Vec::new()
        }

        pub fn set_tooltip(&mut self, _text: String) {}

        pub fn relabel(&self, _loc: &Localizer) {}
    }
}

pub use imp::Tray;

/// Creates the tray icon, or `None` where the platform has none or it fails.
pub fn create(loc: &Localizer, bridge: Arc<Bridge>) -> Option<Tray> {
    let tray = Tray::new(loc, bridge);
    if tray.is_none() && cfg!(windows) {
        tracing::warn!("tray icon unavailable");
    }
    tray
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_labels_exist_in_both_locales() {
        for l in [udm_i18n::Locale::Id, udm_i18n::Locale::En] {
            let loc = Localizer::new(l);
            for (_, key) in ITEMS {
                assert_ne!(loc.tr(key), key, "{key} missing");
            }
        }
    }
}
