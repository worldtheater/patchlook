use crate::strings::tr;
use anyhow::{Context as _, ensure};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    assets::IconName,
    base::{DeferredPopover, GlobalState, Popup, TestSupportExt},
    component::{
        ActiveTheme, Selectable, Sizable, ThemeStyled,
        button::{Button, ButtonVariants},
        h_flex,
        input::{self, Input, InputEvent, InputState},
        v_flex,
    },
    *,
};
use patchlook::{
    git::{RepositoryLocation, SshAuthentication, SshOptions},
    preferences::{SshHistoryField, SshInputHistory},
    ssh_auth::SshSecret,
};

type RemoveHistoryHandler = Box<dyn Fn(SshHistoryField, &str, &mut App)>;

pub struct RemoteRepositoryForm {
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub path: Entity<InputState>,
    pub config: Entity<InputState>,
    pub identity: Entity<InputState>,
    pub secret: Entity<InputState>,
    pub authentication: SshAuthentication,
    advanced: bool,
    pub error: Option<String>,
    history: SshInputHistory,
    history_field: Option<SshHistoryField>,
    history_selection: Option<usize>,
    history_scroll: ScrollHandle,
    history_popover: Option<DeferredPopover>,
    on_remove_history: RemoveHistoryHandler,
    _subscriptions: Vec<Subscription>,
}

impl RemoteRepositoryForm {
    pub fn new(
        history: SshInputHistory,
        on_remove_history: impl Fn(SshHistoryField, &str, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Self {
            host: cx.new(|cx| InputState::new(window, cx).placeholder(tr("ssh_host_placeholder"))),
            port: cx.new(|cx| InputState::new(window, cx).placeholder(tr("ssh_port_placeholder"))),
            username: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("ssh_username_placeholder"))),
            path: cx.new(|cx| InputState::new(window, cx).placeholder(tr("ssh_path_placeholder"))),
            config: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("ssh_config_placeholder"))),
            identity: cx
                .new(|cx| InputState::new(window, cx).placeholder(tr("ssh_identity_placeholder"))),
            secret: cx.new(|cx| InputState::new(window, cx).masked(true)),
            authentication: SshAuthentication::Password,
            advanced: false,
            error: None,
            history,
            history_field: None,
            history_selection: None,
            history_scroll: ScrollHandle::new(),
            history_popover: None,
            on_remove_history: Box::new(on_remove_history),
            _subscriptions: vec![],
        };
        for field in [
            SshHistoryField::Host,
            SshHistoryField::Port,
            SshHistoryField::Username,
            SshHistoryField::Path,
            SshHistoryField::Identity,
            SshHistoryField::Config,
        ] {
            let input = form.input(field).clone();
            form._subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event, window, cx| match event {
                    InputEvent::Focus | InputEvent::Change
                        if input.focus_handle(cx).is_focused(window) =>
                    {
                        this.show_history(field, cx)
                    }
                    InputEvent::Blur if this.history_field == Some(field) => this.close_history(cx),
                    _ => {}
                },
            ));
        }
        form
    }

    fn input(&self, field: SshHistoryField) -> &Entity<InputState> {
        match field {
            SshHistoryField::Host => &self.host,
            SshHistoryField::Port => &self.port,
            SshHistoryField::Username => &self.username,
            SshHistoryField::Path => &self.path,
            SshHistoryField::Identity => &self.identity,
            SshHistoryField::Config => &self.config,
        }
    }

    fn candidates(&self, field: SshHistoryField, cx: &App) -> Vec<String> {
        let query = self.input(field).read(cx).value().to_lowercase();
        self.history
            .values(field)
            .iter()
            .filter(|value| value.to_lowercase().contains(query.as_str()))
            .cloned()
            .collect()
    }

    fn show_history(&mut self, field: SshHistoryField, cx: &mut Context<Self>) {
        if self.candidates(field, cx).is_empty() {
            self.close_history(cx);
            return;
        }
        self.history_field = Some(field);
        self.history_selection = None;
        self.history_scroll.set_offset(point(px(0.), px(0.)));
        if self.history_popover.is_none() {
            self.history_popover = Some(GlobalState::register_deferred_popover(cx));
        }
        cx.notify();
    }

    fn close_history(&mut self, cx: &mut Context<Self>) {
        self.history_field = None;
        self.history_selection = None;
        self.history_popover = None;
        cx.notify();
    }

    fn select_history(
        &mut self,
        field: SshHistoryField,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input(field)
            .update(cx, |input, cx| input.set_value(value, window, cx));
        self.input(field).focus_handle(cx).focus(window, cx);
        self.close_history(cx);
        // Returning focus to the input may emit Focus; close after it is delivered too.
        let form = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = form.update(cx, |this, cx| this.close_history(cx));
        });
    }

    fn history_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.history_field else {
            return;
        };
        let values = self.candidates(field, cx);
        match key {
            "escape" => self.close_history(cx),
            "down" if !values.is_empty() => {
                self.history_selection =
                    Some(self.history_selection.map_or(0, |i| (i + 1) % values.len()));
                cx.notify();
            }
            "up" if !values.is_empty() => {
                self.history_selection = Some(
                    self.history_selection
                        .map_or(values.len() - 1, |i| (i + values.len() - 1) % values.len()),
                );
                cx.notify();
            }
            "enter" if self.history_selection.is_some() => {
                if let Some(value) = self.history_selection.and_then(|i| values.get(i)) {
                    self.select_history(field, value, window, cx);
                }
            }
            _ => return,
        }
        if let Some(index) = self.history_selection {
            self.history_scroll.scroll_to_item(index);
        }
        cx.stop_propagation();
        window.prevent_default();
    }

    fn history_input(
        &self,
        id: &'static str,
        field: SshHistoryField,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let input = self.input(field);
        Popup::new(SharedString::from(format!("{id}-popup")), Input::new(input))
            .w_full()
            .min_w_0()
            .offset(px(4.))
            .capture_any_mouse_down(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                if event.button == MouseButton::Left
                    && this
                        .input(field)
                        .read(cx)
                        .input_bounds()
                        .contains(&event.position)
                {
                    this.show_history(field, cx);
                }
            }))
            .when(self.history_field == Some(field), |popup| {
                let values = self.candidates(field, cx);
                popup.content(
                    v_flex()
                        .id("ssh-history-menu")
                        .test_support()
                        .w(input.read(cx).input_bounds().size.width)
                        .popover_style(cx)
                        .p_1()
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .track_scroll(&self.history_scroll)
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .on_mouse_down_out(cx.listener(
                            move |this, event: &MouseDownEvent, _, cx| {
                                if !this
                                    .input(field)
                                    .read(cx)
                                    .input_bounds()
                                    .contains(&event.position)
                                {
                                    this.close_history(cx);
                                }
                            },
                        ))
                        .children(values.into_iter().enumerate().map(|(index, value)| {
                            let remove = value.clone();
                            h_flex()
                                .id(("ssh-history-item", index))
                                .gap_1()
                                .rounded_sm()
                                .hover(|style| style.bg(cx.theme().list_hover))
                                .when(self.history_selection == Some(index), |row| {
                                    row.bg(cx.theme().list_active)
                                })
                                .child(
                                    div()
                                        .id(("ssh-history-value", index))
                                        .test_support()
                                        .flex_1()
                                        .min_w_0()
                                        .px_2()
                                        .py_1()
                                        .cursor_pointer()
                                        .child(div().truncate().child(value.clone()))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.select_history(field, &value, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new(("ssh-history-remove", index))
                                        .icon(IconName::X)
                                        .xsmall()
                                        .ghost()
                                        .tab_stop(false)
                                        .tooltip(tr("ssh_remove_history"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.history.remove(field, &remove);
                                            (this.on_remove_history)(field, &remove, cx);
                                            this.input(field).focus_handle(cx).focus(window, cx);
                                            this.show_history(field, cx);
                                        })),
                                )
                        })),
                )
            })
    }

    fn history_field(
        &self,
        id: &'static str,
        label: &'static str,
        field: SshHistoryField,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .id(id)
            .test_support()
            .min_w_0()
            .flex_1()
            .gap_1()
            .child(tr(label))
            .child(self.history_input("history-input", field, cx))
    }

    pub fn prefill(
        &mut self,
        target: &RepositoryLocation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let RepositoryLocation::Ssh { options, path } = target else {
            return;
        };
        self.authentication = options.authentication;
        self.advanced = options.config_file.is_some();
        for (input, value) in [
            (&self.host, options.host.clone()),
            (
                &self.port,
                options.port.map(|p| p.to_string()).unwrap_or_default(),
            ),
            (&self.username, options.username.clone().unwrap_or_default()),
            (&self.path, path.clone()),
            (
                &self.identity,
                options
                    .identity_file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
            (
                &self.config,
                options
                    .config_file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    pub fn can_connect(&self, cx: &App) -> bool {
        self.location(cx).is_ok()
    }

    fn location(&self, cx: &App) -> anyhow::Result<RepositoryLocation> {
        let port = self.port.read(cx).value().trim().to_owned();
        let username = self.username.read(cx).value().trim().to_owned();
        let identity = self.identity.read(cx).value().trim().to_owned();
        let config = self.config.read(cx).value().trim().to_owned();
        let secret = self.secret.read(cx).value().to_string();
        let password = self.authentication == SshAuthentication::Password;
        ensure!(
            !password || !secret.is_empty(),
            "{}",
            tr("ssh_password_required")
        );
        if !secret.is_empty() {
            SshSecret::validate(&secret)?;
        }
        let target = RepositoryLocation::Ssh {
            path: self.path.read(cx).value().to_string(),
            options: SshOptions {
                host: self.host.read(cx).value().trim().to_owned(),
                port: if port.is_empty() {
                    None
                } else {
                    Some(port.parse().context(tr("ssh_invalid_port"))?)
                },
                username: (!username.is_empty()).then_some(username),
                identity_file: (!password && !identity.is_empty()).then(|| identity.into()),
                config_file: (!config.is_empty()).then(|| config.into()),
                authentication: if password {
                    SshAuthentication::Password
                } else if secret.is_empty() {
                    SshAuthentication::Key
                } else {
                    SshAuthentication::KeyWithPassphrase
                },
            },
        };
        target.validate()?;
        Ok(target)
    }

    pub fn connection(&self, cx: &App) -> anyhow::Result<(RepositoryLocation, Option<SshSecret>)> {
        let target = self.location(cx)?;
        let secret = self.secret.read(cx).value().to_string();
        let secret = if secret.is_empty() {
            None
        } else {
            Some(SshSecret::new(secret, std::env::current_exe()?)?)
        };
        Ok((target, secret))
    }

    fn secret_field(
        id: &'static str,
        label: &'static str,
        input: &Entity<InputState>,
    ) -> impl IntoElement {
        v_flex()
            .id(id)
            .test_support()
            .min_w_0()
            .flex_1()
            .gap_1()
            .child(tr(label))
            .child(Input::new(input).mask_toggle())
    }

    fn file_field(
        &self,
        id: &'static str,
        label: &'static str,
        field: SshHistoryField,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let input = self.input(field).clone();
        v_flex().gap_1().child(tr(label)).child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(self.history_input(id, field, cx)),
                )
                .child(
                    Button::new(id)
                        .icon(IconName::FolderOpen)
                        .small()
                        .ghost()
                        .tooltip(tr("ssh_choose_file"))
                        .on_click(cx.listener(move |_, _, window, cx| {
                            let input = input.clone();
                            let prompt = cx.prompt_for_paths(PathPromptOptions {
                                files: true,
                                directories: false,
                                multiple: false,
                                prompt: Some(tr("ssh_choose_file").into()),
                            });
                            cx.spawn_in(window, async move |_, cx| {
                                if let Ok(Ok(Some(paths))) = prompt.await
                                    && let Some(path) = paths.first()
                                {
                                    let _ = cx.update(|window, cx| {
                                        input.update(cx, |input, cx| {
                                            input.set_value(path.display().to_string(), window, cx)
                                        })
                                    });
                                }
                            })
                            .detach();
                        })),
                ),
        )
    }
}

impl Render for RemoteRepositoryForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let password = self.authentication == SshAuthentication::Password;
        v_flex()
            .id("ssh-form")
            .test_support()
            .gap_3()
            .text_sm()
            .capture_action(cx.listener(|this, _: &input::MoveDown, window, cx| {
                this.history_key("down", window, cx)
            }))
            .capture_action(
                cx.listener(|this, _: &input::MoveUp, window, cx| {
                    this.history_key("up", window, cx)
                }),
            )
            .capture_action(cx.listener(|this, _: &input::Escape, window, cx| {
                this.history_key("escape", window, cx)
            }))
            .capture_action(
                cx.listener(|this, _: &gpui_kit::base::actions::Cancel, window, cx| {
                    this.history_key("escape", window, cx)
                }),
            )
            .capture_action(cx.listener(|this, action: &input::Enter, window, cx| {
                if !action.secondary && !action.shift {
                    this.history_key("enter", window, cx)
                }
            }))
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ssh_description")),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(self.history_field("ssh-host", "ssh_host", SshHistoryField::Host, cx))
                    .child(div().w(px(120.)).child(self.history_field(
                        "ssh-port",
                        "ssh_port",
                        SshHistoryField::Port,
                        cx,
                    ))),
            )
            .child(self.history_field(
                "ssh-username",
                "ssh_username",
                SshHistoryField::Username,
                cx,
            ))
            .child(self.history_field("ssh-path", "ssh_path", SshHistoryField::Path, cx))
            .child(
                h_flex().gap_2().children(
                    [
                        (true, "ssh-auth-password", "ssh_auth_password"),
                        (false, "ssh-auth-key", "ssh_auth_key"),
                    ]
                    .map(|(is_password, id, label)| {
                        Button::new(id)
                            .label(tr(label))
                            .small()
                            .outline()
                            .selected(password == is_password)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_history(cx);
                                this.authentication = if is_password {
                                    SshAuthentication::Password
                                } else {
                                    SshAuthentication::Key
                                };
                                this.secret.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                    input.set_masked(true, window, cx);
                                });
                                this.error = None;
                                cx.notify();
                            }))
                    }),
                ),
            )
            .when(!password, |view| {
                view.child(self.file_field(
                    "ssh-choose-key",
                    "ssh_identity",
                    SshHistoryField::Identity,
                    cx,
                ))
            })
            .child(Self::secret_field(
                "ssh-secret",
                if password {
                    "ssh_password"
                } else {
                    "ssh_passphrase"
                },
                &self.secret,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ssh_secret_ephemeral")),
            )
            .child(
                Button::new("ssh-advanced")
                    .label(tr("ssh_advanced"))
                    .icon(if self.advanced {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_history(cx);
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            )
            .when(self.advanced, |view| {
                view.child(self.file_field(
                    "ssh-choose-config",
                    "ssh_config",
                    SshHistoryField::Config,
                    cx,
                ))
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_color(cx.theme().danger).child(error))
            })
    }
}
