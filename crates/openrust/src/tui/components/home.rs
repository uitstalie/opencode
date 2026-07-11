//! Home view — the welcome/startup screen shown before a session starts.

use ratatui::{
    Frame,
    layout::Alignment,
    style::Modifier,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::tui::components::{input::InputPanel, modal::ModalLayer, status::StatusBar, toast::Toast};
use crate::tui::layout::home_layout;
use crate::tui::SessionView;

pub struct HomeView<'a> {
    view: &'a SessionView,
}

impl<'a> HomeView<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    pub fn render(&self, frame: &mut Frame) {
        let view = self.view;
        let layout = home_layout(frame.area());

        let header = Paragraph::new(vec![
            Line::from(Span::styled(
                "OpenRust",
                view.theme.brand_style().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "AI coding agent · Rust native runtime",
                view.theme.muted_style(),
            )),
            Line::from(Span::styled(
                format!(
                    "model {}/{} · agent {}",
                    view.provider_name,
                    view.model,
                    view.current_session_agent().unwrap_or_else(|| "default".to_string())
                ),
                view.theme.muted_style(),
            )),
        ])
        .alignment(Alignment::Center)
        .style(view.theme.panel_style());
        frame.render_widget(header, layout.header);

        InputPanel::new(view).render(frame, layout.input, "Prompt");

        let hint = Paragraph::new(
            "输入消息后 Enter 开始 · /connect 配置 provider · /models 选择模型 · Esc 退出",
        )
        .style(view.theme.muted_style())
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });
        frame.render_widget(hint, layout.hint);

        let status = Paragraph::new(view.home_status_message())
            .style(view.theme.muted_style())
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(status, layout.status_message);

        StatusBar::new(view).render(frame, layout.info, layout.status);
        Toast::new(view).render(frame, frame.area());
        ModalLayer::new(view).render(frame, frame.area());
    }
}
