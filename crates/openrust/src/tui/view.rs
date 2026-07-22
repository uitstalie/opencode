//! View layer system: z-order levels, key-flow control, and layer dispatch.
//!
//! Each frame the app builds a `Vec<Layer>` from current state.
//! Layers are sorted by [`Z`] before rendering (lowest first) and
//! again before key routing (highest first). This replaces the
//! hardcoded call order that previously lived in `render_session_frame`.

use ratatui::{Frame, layout::Rect};

use super::SessionView;
use super::components;

// ── public(super) types ──────────────────────────────────────────

/// Result of key handling — controls propagation through the z-order stack.
#[derive(Clone, Copy, Debug)]
pub(super) enum KeyFlow {
    /// Key consumed; stop propagation and render.
    Consumed,
    /// Key not handled; let the next lower layer try.
    Propagate,
    /// Request to exit the application loop.
    Exit,
}

/// Explicit z-order level.
///
/// Higher layers render on top and receive key events first.
#[derive(Clone, Copy, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub(super) enum Z {
    /// z=0 — main panels: session / sidebar / input / status / home-content.
    Base,
    /// z=1 — floating popups: toast / slash-help.
    Float,
    /// z=2 — modal overlays: dialog / question / permission / text-input / diff.
    Modal,
}

// ── Layer enum ───────────────────────────────────────────────────

/// A single layer to render.
///
/// Built per-frame from current app state. Carries enough geometry
/// (`Rect`) for its component to draw without re-computing layout.
pub(super) enum Layer {
    // ── Z::Base ──
    /// Home screen header, hint, and status message (not input/status-bar).
    HomeContent {
        header: Rect,
        hint: Rect,
        status_msg: Rect,
    },
    /// Workspace path + TODO panel.
    Sidebar {
        workspace: Option<Rect>,
        todo: Option<Rect>,
    },
    /// Conversation history panel.
    Session(Rect),
    /// Input textarea + cursor placement.
    Input {
        area: Rect,
        title: &'static str,
    },
    /// Info line + bottom status footer.
    Status {
        info: Rect,
        footer: Rect,
    },

    // ── Z::Float ──
    /// Slash-command help popup floating above the input area.
    SlashHelp {
        input_area: Rect,
    },
    /// Transient toast notification.
    Toast,

    // ── Z::Modal ──
    /// Dialog / question / permission / text-input / diff overlay.
    Overlay,
}

impl Layer {
    pub(super) fn z(&self) -> Z {
        match self {
            Self::HomeContent { .. }
            | Self::Sidebar { .. }
            | Self::Session(_)
            | Self::Input { .. }
            | Self::Status { .. } => Z::Base,
            Self::SlashHelp { .. } | Self::Toast => Z::Float,
            Self::Overlay => Z::Modal,
        }
    }

    /// Dispatch rendering to the appropriate component.
    pub(super) fn render(&self, view: &SessionView, frame: &mut Frame) {
        match self {
            Self::HomeContent {
                header,
                hint,
                status_msg,
            } => {
                components::HomeContent::new(view).render(frame, *header, *hint, *status_msg);
            }
            Self::Sidebar { workspace, todo } => {
                components::SidebarPanel::new(view).render(frame, *workspace, *todo);
            }
            Self::Session(area) => {
                components::SessionPanel::new(view).render(frame, *area);
            }
            Self::Input { area, title } => {
                components::InputPanel::new(view).render(frame, *area, title);
            }
            Self::Status { info, footer } => {
                components::StatusBar::new(view).render(frame, *info, *footer);
            }
            Self::SlashHelp { input_area } => {
                components::SlashHelpPopup::new(view).render(frame, *input_area);
            }
            Self::Toast => {
                components::Toast::new(view).render(frame, frame.area());
            }
            Self::Overlay => {
                components::ModalLayer::new(view).render(frame, frame.area());
            }
        }
    }
}

// ── Layer list builder ───────────────────────────────────────────

impl SessionView {
    /// Build the active layer list for this frame.
    ///
    /// Base layers differ between Home and Session; Float and Modal
    /// layers are shared. The returned vector is in insertion order;
    /// the caller sorts by [`Layer::z`] before rendering.
    pub(super) fn build_layers(&self, area: Rect) -> Vec<Layer> {
        let mut layers = if self.view_mode == super::ViewMode::Home {
            self.build_home_base_layers(area)
        } else {
            self.build_session_base_layers(area)
        };

        // Float layers
        if self.slash_help_active() {
            layers.push(Layer::SlashHelp {
                input_area: self.render.input_area.get(),
            });
        }
        if self.ui.toast.is_some() {
            layers.push(Layer::Toast);
        }

        // Modal layer
        if self.overlay_active() || self.diff_visible {
            layers.push(Layer::Overlay);
        }

        layers
    }

    fn build_home_base_layers(&self, area: Rect) -> Vec<Layer> {
        let layout = super::layout::home_layout(area);
        self.render.input_area.set(layout.input);
        vec![
            Layer::HomeContent {
                header: layout.header,
                hint: layout.hint,
                status_msg: layout.status_message,
            },
            Layer::Input {
                area: layout.input,
                title: "Prompt",
            },
            Layer::Status {
                info: layout.info,
                footer: layout.status,
            },
        ]
    }

    fn build_session_base_layers(&self, area: Rect) -> Vec<Layer> {
        let layout = super::layout::session_layout(
            area,
            self.sidebar_visible,
            self.task_count.get(),
        );
        self.render.input_area.set(layout.input);
        vec![
            Layer::Sidebar {
                workspace: layout.sidebar_workspace,
                todo: layout.sidebar_todo,
            },
            Layer::Session(layout.session),
            Layer::Input {
                area: layout.input,
                title: "Input",
            },
            Layer::Status {
                info: layout.info,
                footer: layout.status,
            },
        ]
    }
}
