//! TUI display components.
//!
//! Each component is a lightweight struct borrowing `&SessionView`
//! that renders into a given `Rect`. The layer system in [`crate::tui::view`]
//! composes them via z-order.

pub(super) mod home;
pub(super) mod input;
pub(super) mod modal;
pub(super) mod session;
pub(super) mod sidebar;
pub(super) mod slash_help;
pub(super) mod status;
pub(super) mod toast;

pub(super) use home::HomeContent;
pub(super) use input::InputPanel;
pub(super) use modal::ModalLayer;
pub(super) use session::SessionPanel;
pub(super) use sidebar::SidebarPanel;
pub(super) use slash_help::SlashHelpPopup;
pub(super) use status::StatusBar;
pub(super) use toast::Toast;
