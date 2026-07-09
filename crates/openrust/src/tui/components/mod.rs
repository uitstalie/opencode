//! TUI display components.
//!
//! Each component is a lightweight struct borrowing `&SessionView`
//! that renders into a given `Rect`. The render entry point in
//! `session_render.rs` composes them via the layout system.

pub(super) mod home;
pub(super) mod input;
pub(super) mod modal;
pub(super) mod session;
pub(super) mod sidebar;
pub(super) mod status;
pub(super) mod toast;

pub(super) use home::HomeView;
pub(super) use input::InputPanel;
pub(super) use modal::ModalLayer;
pub(super) use session::SessionPanel;
pub(super) use sidebar::SidebarPanel;
pub(super) use status::StatusBar;
pub(super) use toast::Toast;
