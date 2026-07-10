//! Dialog openers, connect wizard, and modal rendering.

use super::dialog::{Dialog, DialogKind, DialogOption};
use super::util::{now_micros, single_line_textarea};
use super::{ConnectDraft, PendingTextInput, SessionView, ThinkingModeCommand};
use crate::core::{config::{ModelConfig, ProviderConfig}, vault::Vault};

impl SessionView {
    pub(super) fn open_thinking_dialog(&mut self, mode: ThinkingModeCommand) {
        self.ui.dialog = Some(Dialog::new(
            DialogKind::Thinking,
            "Thinking",
            "选择 thinking 内容是否显示在会话流中。",
            vec![
                DialogOption::new(
                    "show",
                    "Show",
                    "显示 thinking 内容，使用独立 thinking theme。",
                ),
                DialogOption::new("hide", "Hide", "隐藏 thinking 内容，只保留普通输出。"),
                DialogOption::new("toggle", "Toggle", "在 show / hide 之间切换。"),
            ],
            match mode {
                ThinkingModeCommand::Show => 0,
                ThinkingModeCommand::Hide => 1,
                ThinkingModeCommand::Toggle => 2,
            },
        ));
        self.status = "dialog: thinking".to_string();
    }

    pub(super) fn open_session_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("new") => {
                self.create_session();
            }
            Some("switch") | Some("use") => {
                if let Some(target) = args.get(1) {
                    self.switch_session(target);
                } else {
                    self.note("usage: /session switch <id|number>".to_string());
                }
            }
            Some("delete") | Some("rm") => {
                if let Some(target) = args.get(1) {
                    self.delete_session(target);
                } else {
                    self.open_session_delete_dialog();
                }
            }
            _ => {
                let Some(store) = self.store.clone() else {
                    tracing::warn!("open_session_dialog: store is None");
                    self.note("session store unavailable".to_string());
                    return;
                };
                let Ok(sessions) = store.list_sessions() else {
                    tracing::error!("open_session_dialog: list_sessions failed");
                    self.note("failed to list sessions".to_string());
                    return;
                };
                tracing::info!(count = sessions.len(), "open_session_dialog");
                let mut options = vec![DialogOption::new(
                    "__new__",
                    "New session",
                    "创建一个新的空会话。",
                )];
                options.extend(self.session_list_options(&sessions, false));
                options.push(DialogOption::new(
                    "__delete__",
                    "Delete session",
                    "选择一个会话删除（不可恢复）。",
                ));
                self.ui.dialog = Some(Dialog::new(
                    DialogKind::Session,
                    "Sessions",
                    "选择要切换的会话。",
                    options,
                    0,
                ));
                self.status = "dialog: sessions".to_string();
            }
        }
    }

    fn open_session_delete_dialog(&mut self) {
        let Some(store) = self.store.clone() else {
            self.note("session store unavailable".to_string());
            return;
        };
        if let Ok(n) = store.cleanup_old_sessions(30 * 24 * 60 * 60)
            && n > 0 {
                self.note(format!("cleaned {n} sessions older than 30 days"));
            }
        let Ok(sessions) = store.list_sessions() else {
            self.note("failed to list sessions".to_string());
            return;
        };
        let options = self.session_list_options(&sessions, true);
        if options.is_empty() {
            self.note("no deletable sessions".to_string());
            return;
        }
        self.ui.dialog = Some(Dialog::new(
            DialogKind::SessionDelete,
            "Delete Session",
            "选择要删除的会话（Enter 确认 · Esc 取消）。",
            options,
            0,
        ));
        self.status = "dialog: delete session".to_string();
    }

    fn session_list_options(
        &self,
        sessions: &[crate::core::session::SessionSummary],
        delete_mode: bool,
    ) -> Vec<DialogOption> {
        sessions
            .iter()
            .take(20)
            .enumerate()
            .filter_map(|(index, session)| {
                if delete_mode && session.id == self.session_id {
                    return None;
                }
                let title = session.title.as_deref().unwrap_or(&session.id);
                let agent = session.agent.as_deref().unwrap_or("(no agent)");
                let marker = if session.id == self.session_id { "● " } else { "" };
                let summary_preview = session
                    .summary
                    .as_deref()
                    .map(|s| {
                        let short = &s[..s.len().min(30)];
                        format!(" · {}", short)
                    })
                    .unwrap_or_default();
                let mut desc = format!(
                    "{} msg · {}{}",
                    session.message_count, agent, summary_preview
                );
                if desc.len() > 60 {
                    desc.truncate(57);
                    desc.push_str("...");
                }
                Some(DialogOption::new(
                    session.id.clone(),
                    format!("{}{}. {}", marker, index + 1, title),
                    desc,
                ))
            })
            .collect()
    }

    pub(super) fn open_agent_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("use") => {
                if let Some(agent_id) = args.get(1) {
                    self.switch_agent(agent_id);
                } else {
                    self.note("usage: /agent use <id|number>".to_string());
                }
            }
            _ => {
                let mut options = vec![DialogOption::new(
                    "__default__",
                    "Default agent",
                    "Clear the session agent and use the default workflow.",
                )];
                options.extend(self.agents.iter().enumerate().map(|(index, info)| {
                    let active =
                        if self.current_session_agent().as_deref() == Some(info.id.as_str()) {
                            "● "
                        } else {
                            ""
                        };
                    DialogOption::new(
                        info.id.clone(),
                        format!("{}{}. {}", active, index + 1, info.title),
                        info.description.clone(),
                    )
                }));
                self.ui.dialog = Some(Dialog::new(
                    DialogKind::Agent,
                    "Agents",
                    "选择当前会话使用的 agent。",
                    options,
                    0,
                ));
                self.status = "dialog: agents".to_string();
            }
        }
    }

    pub(super) fn open_task_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("add") => {
                let Some(title) = args.get(1) else {
                    self.note("usage: /task add <title> [status]".to_string());
                    return;
                };
                let status = args
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| "pending".to_string());
                let Some(store) = &self.store else {
                    self.note("session store unavailable".to_string());
                    return;
                };
                let id = format!("task-{}", now_micros());
                let agent = self.current_session_agent();
                match store.upsert_task(&self.session_id, &id, agent, title.clone(), status.clone(), None)
                {
                    Ok(task) => self.note(format!("task added: {} [{}]", task.title, task.status)),
                    Err(err) => self.note(format!("failed to add task: {}", err)),
                }
            }
            Some("done") => {
                let Some(id) = args.get(1) else {
                    self.note("usage: /task done <id>".to_string());
                    return;
                };
                let Some(store) = &self.store else {
                    self.note("session store unavailable".to_string());
                    return;
                };
                match store.update_task_status(&self.session_id, id, "completed") {
                    Ok(Some(task)) => self.note(format!("task completed: {}", task.id)),
                    Ok(None) => self.note(format!("task not found: {}", id)),
                    Err(err) => self.note(format!("failed to update task: {}", err)),
                }
            }
            _ => {
                let Some(store) = &self.store else {
                    self.note("session store unavailable".to_string());
                    return;
                };
                let Ok(tasks) = store.list_tasks(&self.session_id) else {
                    self.note("failed to list tasks".to_string());
                    return;
                };
                let mut options = vec![DialogOption::new(
                    "__new__",
                    "New task",
                    "Create a new task for the current session.",
                )];
                options.extend(tasks.iter().enumerate().map(|(index, task)| {
                    let agent = task.agent.as_deref().unwrap_or("(no agent)");
                    DialogOption::new(
                        task.id.clone(),
                        format!("{}. {}", index + 1, task.title),
                        format!("{} · agent: {}", task.status, agent),
                    )
                }));
                self.ui.dialog = Some(Dialog::new(
                    DialogKind::Task,
                    "Tasks",
                    "查看或更新当前会话的任务。",
                    options,
                    0,
                ));
                self.status = "dialog: tasks".to_string();
            }
        }
    }

    pub(super) fn open_connect_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("use") => {
                if let Some(provider) = args.get(1) {
                    self.switch_provider(provider);
                } else {
                    self.note("usage: /connect use <provider>".to_string());
                }
            }
            Some("add") if args.len() >= 4 => self.add_provider(&args),
            Some("add") => self.start_connect_wizard(),
            Some("key") => {
                let (Some(provider), Some(api_key)) = (args.get(1), args.get(2)) else {
                    self.note("usage: /connect key <provider> <api-key>".to_string());
                    return;
                };
                match Vault::save(provider, api_key) {
                    Ok(()) => self.note(format!(
                        "saved API key for {}; run /connect verify {}",
                        provider, provider
                    )),
                    Err(err) => self.note(format!("failed to save API key: {}", err)),
                }
                self.reload_config();
            }
            Some("verify") => {
                let provider = args
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| self.provider_name.clone());
                self.verify_provider(&provider);
            }
            _ => {
                let mut providers = self.config.provider.keys().cloned().collect::<Vec<_>>();
                providers.sort();
                let mut options = providers
                    .iter()
                    .map(|provider| {
                        let active = if provider == &self.provider_name {
                            "● "
                        } else {
                            ""
                        };
                        DialogOption::new(
                            provider.clone(),
                            format!("{}{}", active, provider),
                            "切换到这个 provider。",
                        )
                    })
                    .collect::<Vec<_>>();
                options.push(DialogOption::new(
                    "__add__",
                    "Add custom provider",
                    "交互式输入 provider、base URL、model 和 API key。",
                ));
                options.push(DialogOption::new(
                    "__verify__",
                    "Verify current provider",
                    "验证当前 provider、API key 和模型通路。",
                ));
                self.ui.dialog = Some(Dialog::new(
                    DialogKind::Provider,
                    "Connect",
                    "选择 provider，或验证当前通路。自定义 provider 使用 /connect add。",
                    options,
                    0,
                ));
                self.status = "dialog: connect".to_string();
            }
        }
    }

    pub(super) fn open_models_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("use") => {
                if let Some(spec) = args.get(1) {
                    self.switch_model(spec);
                } else {
                    self.note("usage: /models use <provider/model>".to_string());
                }
            }
            Some("thinking") | Some("effort") => {
                self.open_reasoning_dialog(args.get(1).map(String::as_str))
            }
            Some(spec) if spec.contains('/') => self.switch_model(spec),
            _ => {
                let current = self.config.model.as_deref().unwrap_or("");
                let mut options = self
                    .config
                    .provider
                    .iter()
                    .flat_map(|(provider, cfg)| {
                        let mut models = cfg.models.keys().cloned().collect::<Vec<_>>();
                        models.sort();
                        models
                            .into_iter()
                            .map(|model| {
                                let spec = format!("{}/{}", provider, model);
                                let active = if spec == current { "● " } else { "" };
                                DialogOption::new(
                                    spec.clone(),
                                    format!("{}{}", active, spec),
                                    "切换到这个已注册模型。",
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                options.push(DialogOption::new(
                    "__reasoning__",
                    "Thinking effort",
                    "设置模型 reasoning_effort：low / medium / high / off。",
                ));
                self.ui.dialog = Some(Dialog::new(
                    DialogKind::Model,
                    "Models",
                    "选择已注册模型，或进入 thinking effort 设置。",
                    options,
                    0,
                ));
                self.status = "dialog: models".to_string();
            }
        }
    }

    fn open_reasoning_dialog(&mut self, selected: Option<&str>) {
        let selected = match selected.or(self.reasoning_effort.as_deref()) {
            Some("low") => 0,
            Some("medium") => 1,
            Some("high") => 2,
            _ => 3,
        };
        self.ui.dialog = Some(Dialog::new(
            DialogKind::ReasoningEffort,
            "Thinking Effort",
            "选择发送给模型的 reasoning_effort。",
            vec![
                DialogOption::new("low", "Low", "较低思考强度。"),
                DialogOption::new("medium", "Medium", "默认思考强度。"),
                DialogOption::new("high", "High", "更强思考强度。"),
                DialogOption::new("off", "Off", "不发送 reasoning_effort 参数。"),
            ],
            selected,
        ));
        self.status = "dialog: thinking effort".to_string();
    }

    pub(super) fn submit_dialog_selection(&mut self) {
        let Some(dialog) = self.ui.dialog.take() else {
            return;
        };
        match dialog.kind {
            DialogKind::Thinking => {
                let mode = match dialog.selected_value() {
                    Some("show") => ThinkingModeCommand::Show,
                    Some("hide") => ThinkingModeCommand::Hide,
                    _ => ThinkingModeCommand::Toggle,
                };
                self.thinking_mode = mode.apply(self.thinking_mode);
                self.note(format!("thinking: {}", self.thinking_mode.label()));
            }
            DialogKind::Session => match dialog.selected_value() {
                Some("__new__") => self.create_session(),
                Some("__delete__") => self.open_session_delete_dialog(),
                Some(value) => self.switch_session(value),
                None => {}
            },
            DialogKind::SessionDelete => if let Some(value) = dialog.selected_value() {
                self.delete_session(value);
                self.open_session_delete_dialog();
            },
            DialogKind::Agent => match dialog.selected_value() {
                Some("__default__") => self.set_session_agent(None),
                Some(value) => self.set_session_agent(Some(value.to_string())),
                None => {}
            },
            DialogKind::Task => match dialog.selected_value() {
                Some("__new__") => {
                    self.note("use /task add <title> [status] to create a task".to_string())
                }
                Some(value) => self.note(format!("task selected: {}", value)),
                None => {}
            },
            DialogKind::Provider => match dialog.selected_value() {
                Some("__add__") => self.start_connect_wizard(),
                Some("__verify__") => self.verify_provider(&self.provider_name.clone()),
                Some(value) => self.switch_provider(value),
                None => {}
            },
            DialogKind::Model => match dialog.selected_value() {
                Some("__reasoning__") => self.open_reasoning_dialog(None),
                Some(value) => self.switch_model(value),
                None => {}
            },
            DialogKind::ReasoningEffort => {
                if let Some(value) = dialog.selected_value() {
                    self.set_reasoning_effort(value);
                }
            }
            DialogKind::SlashHelp => {
                if let Some(value) = dialog.selected_value() {
                    self.set_input_text(value);
                    self.ui.dialog = None;
                }
            }
        }
    }
    fn add_provider(&mut self, args: &[String]) {
        let (Some(provider), Some(base_url), Some(model)) = (args.get(1), args.get(2), args.get(3))
        else {
            self.note("usage: /connect add <provider> <base-url> <model> [wire-model]".to_string());
            return;
        };
        let wire = args.get(4).cloned().unwrap_or_else(|| model.clone());
        self.config.provider.insert(
            provider.clone(),
            ProviderConfig {
                api_key: None,
                base_url: Some(base_url.clone()),
                models: std::collections::HashMap::from([(
                    model.clone(),
                    ModelConfig {
                        name: Some(wire),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        );
        self.config.model = Some(format!("{}/{}", provider, model));
        match self.save_global_config() {
            Ok(()) => {
                self.reload_config();
                self.note(format!(
                    "provider added: {}; run /connect key {} <api-key>",
                    provider, provider
                ));
            }
            Err(err) => self.note(format!("failed to save provider: {}", err)),
        }
    }

    fn start_connect_wizard(&mut self) {
        self.ui.connect_draft = Some(ConnectDraft::default());
        self.ui.pending_text_input = Some(PendingTextInput {
            title: "Connect · provider".to_string(),
            description: "输入 provider 名称，例如 deepseek、openai、one_route。".to_string(),
            value: String::new(),
            editor: single_line_textarea("", false),
            submit: SessionView::save_connect_provider_name,
        });
        self.status = "connect: provider name".to_string();
    }

    fn save_connect_provider_name(&mut self, value: &str) {
        let provider = value.trim();
        if provider.is_empty() {
            self.note("provider name cannot be empty".to_string());
            return;
        }
        let draft = self.ui.connect_draft.get_or_insert_with(ConnectDraft::default);
        draft.provider = provider.to_string();
        self.ui.pending_text_input = Some(PendingTextInput {
            title: format!("Connect · {} base URL", provider),
            description: "输入 OpenAI-compatible base URL，例如 https://api.deepseek.com/v1 。"
                .to_string(),
            value: String::new(),
            editor: single_line_textarea("", false),
            submit: SessionView::save_connect_base_url,
        });
        self.status = format!("connect: base URL for {}", provider);
    }

    fn save_connect_base_url(&mut self, value: &str) {
        let base_url = value.trim();
        if base_url.is_empty() {
            self.note("base URL cannot be empty".to_string());
            return;
        }
        let Some(draft) = self.ui.connect_draft.as_mut() else {
            self.note("connect wizard state missing".to_string());
            return;
        };
        draft.base_url = base_url.to_string();
        self.ui.pending_text_input = Some(PendingTextInput {
            title: format!("Connect · {} model", draft.provider),
            description: "输入配置中的 model 名称，例如 deepseek-chat。".to_string(),
            value: String::new(),
            editor: single_line_textarea("", false),
            submit: SessionView::save_connect_model,
        });
        self.status = format!("connect: model for {}", draft.provider);
    }

    fn save_connect_model(&mut self, value: &str) {
        let model = value.trim();
        if model.is_empty() {
            self.note("model cannot be empty".to_string());
            return;
        }
        let Some(draft) = self.ui.connect_draft.as_mut() else {
            self.note("connect wizard state missing".to_string());
            return;
        };
        draft.model = model.to_string();
        self.ui.pending_text_input = Some(PendingTextInput {
            title: format!("Connect · {} wire model", draft.provider),
            description: "输入实际发给 API 的模型名；若与上一步相同可直接回车留空。".to_string(),
            value: String::new(),
            editor: single_line_textarea("", false),
            submit: SessionView::save_connect_wire_model,
        });
        self.status = format!("connect: wire model for {}", draft.provider);
    }

    fn save_connect_wire_model(&mut self, value: &str) {
        let Some(draft) = self.ui.connect_draft.as_mut() else {
            self.note("connect wizard state missing".to_string());
            return;
        };
        let wire_model = value.trim();
        draft.wire_model = if wire_model.is_empty() {
            None
        } else {
            Some(wire_model.to_string())
        };
        self.ui.pending_text_input = Some(PendingTextInput {
            title: format!("Connect · {} API key", draft.provider),
            description:
                "输入 API key；若暂时没有可直接回车跳过，之后再用 /connect key <provider> <api-key>。"
                    .to_string(),
            value: String::new(),
            editor: single_line_textarea("", true),
            submit: SessionView::save_connect_api_key,
        });
        self.status = format!("connect: API key for {}", draft.provider);
    }

    fn save_connect_api_key(&mut self, value: &str) {
        let Some(draft) = self.ui.connect_draft.take() else {
            self.note("connect wizard state missing".to_string());
            return;
        };
        self.config.provider.insert(
            draft.provider.clone(),
            ProviderConfig {
                api_key: None,
                base_url: Some(draft.base_url.clone()),
                models: std::collections::HashMap::from([(
                    draft.model.clone(),
                    ModelConfig {
                        name: draft.wire_model.clone(),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            },
        );
        self.config.model = Some(format!("{}/{}", draft.provider, draft.model));

        match self.save_global_config() {
            Ok(()) => {
                let api_key = value.trim();
                if !api_key.is_empty()
                    && let Err(err) = Vault::save(&draft.provider, api_key) {
                        self.reload_config();
                        self.note(format!(
                            "provider added: {}, but failed to save API key: {}",
                            draft.provider, err
                        ));
                        return;
                    }
                self.reload_config();
                self.note(format!(
                    "provider configured: {} · model: {}/{}",
                    draft.provider, draft.provider, draft.model
                ));
            }
            Err(err) => self.note(format!("failed to save provider: {}", err)),
        }
    }
}
