use crate::{
    app::{About, Quit},
    strings::tr,
    ui::{DisplaySettings, apply_appearance, apply_language, icon_button, toolbar_divider},
};
use gpui_kit::{
    assets::IconName,
    base::TestSupportExt,
    component::{
        ActiveTheme, Disableable, Icon, Selectable, Sizable, Theme, TitleBar,
        button::{Button, ButtonVariants},
        diff::{Diff, DiffChangeIndicator, DiffFile, DiffHunkSeparator, DiffMode, DiffState},
        h_flex,
        input::{Editor, EditorState},
        menu::{DropdownMenu, PopupMenuItem},
        tooltip::Tooltip,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use patchlook::{
    preferences::{Appearance, Language, Preferences},
    text_diff::{self, ReadTextError},
};
use std::path::PathBuf;

gpui_kit::actions!(
    patchlook,
    [
        NewTextComparison,
        CompareText,
        RefreshComparison,
        CloseTextComparison
    ]
);

pub fn init(cx: &mut App) {
    cx.on_action(|_: &NewTextComparison, cx| {
        if let Err(error) = open(cx) {
            eprintln!("Cannot open text comparison: {error:#}");
        }
    });
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewTextComparison, None),
        KeyBinding::new("secondary-enter", CompareText, Some("TextComparison")),
        KeyBinding::new(
            "secondary-enter",
            CompareText,
            Some("TextComparison > Input"),
        ),
        KeyBinding::new("secondary-r", RefreshComparison, Some("TextComparison")),
        KeyBinding::new("f5", RefreshComparison, Some("TextComparison")),
        KeyBinding::new("secondary-w", CloseTextComparison, Some("TextComparison")),
    ]);
}

// Highlighter identifiers, shared by the input and diff renderers.
const SYNTAX_LANGUAGES: &[&str] = &[
    "bash",
    "c",
    "cpp",
    "csharp",
    "css",
    "go",
    "html",
    "java",
    "javascript",
    "json",
    "kotlin",
    "markdown",
    "python",
    "ruby",
    "rust",
    "sql",
    "swift",
    "toml",
    "tsx",
    "typescript",
    "xml",
    "yaml",
];

struct Source {
    editor: Entity<EditorState>,
    path: Option<PathBuf>,
}

pub struct TextComparison {
    sources: [Source; 2],
    diff: Entity<DiffState>,
    result: bool,
    identical: bool,
    soft_wrap: bool,
    language: Option<SharedString>,
    error: Option<String>,
    task: Option<Task<()>>,
    focus: FocusHandle,
    about: Entity<crate::about::AboutPanel>,
    #[cfg(test)]
    preferences_directory: tempfile::TempDir,
    _subscriptions: Vec<Subscription>,
}

pub fn open(cx: &mut App) -> anyhow::Result<()> {
    let bounds = Bounds::centered(None, size(px(1120.), px(760.)), cx);
    open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(800.), px(480.))),
            titlebar: Some(TitlebarOptions {
                title: Some(tr("text_comparison").into()),
                ..TitleBar::title_bar_options()
            }),
            app_id: Some(concat!("dev.", env!("CARGO_PKG_NAME"), ".app").into()),
            ..TitleBar::window_options()
        },
        cx,
        |window, cx| cx.new(|cx| TextComparison::new(window, cx)),
    )?;
    Ok(())
}

impl TextComparison {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(not(test))]
        let preferences = Preferences::read().unwrap_or_default();
        #[cfg(test)]
        let preferences = Preferences::default();
        DisplaySettings::initialize(&preferences, cx);
        let soft_wrap = false;
        let sources = std::array::from_fn(|_| Source {
            editor: cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("text")
                    .line_number(true)
                    .soft_wrap(soft_wrap)
                    .placeholder(tr("compare_placeholder"))
            }),
            path: None,
        });
        let diff = cx.new(|cx| DiffState::new([], cx).with_mode(DiffMode::Split));
        let subscriptions = vec![
            cx.observe(&diff, |_, _, cx| cx.notify()),
            cx.observe_global_in::<DisplaySettings>(window, |this, window, cx| {
                this.update_translations(window, cx);
                cx.notify();
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                if DisplaySettings::get(cx).appearance == Appearance::System {
                    Theme::sync_system_appearance(Some(window), cx);
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active()
                    && DisplaySettings::get(cx).language == Language::System
                {
                    apply_language(Language::System, cx);
                    this.update_translations(window, cx);
                }
            }),
        ];
        window.focus(&sources[0].editor.focus_handle(cx), cx);
        Self {
            sources,
            diff,
            soft_wrap,
            result: false,
            identical: false,
            language: None,
            error: None,
            task: None,
            focus: cx.focus_handle(),
            about: cx.new(crate::about::AboutPanel::new),
            #[cfg(test)]
            preferences_directory: tempfile::tempdir().unwrap(),
            _subscriptions: subscriptions,
        }
    }

    fn update_translations(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.set_window_title(tr("text_comparison"));
        for source in &self.sources {
            source.editor.update(cx, |editor, cx| {
                editor.set_placeholder(tr("compare_placeholder"), window, cx);
            });
        }
    }

    fn set_interface_language(
        &mut self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        apply_language(language, cx);
        self.update_translations(window, cx);
        self.save_display_settings(cx);
    }

    fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        apply_appearance(appearance, window, cx);
        self.save_display_settings(cx);
    }

    fn save_display_settings(&mut self, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<()> {
            // Read the latest preferences so changing a scratch window's theme
            // never overwrites repository history or other window settings.
            #[cfg(not(test))]
            let mut preferences = Preferences::read()?;
            #[cfg(test)]
            let mut preferences: Preferences = {
                let file = self.preferences_directory.path().join("preferences.json");
                if file.exists() {
                    serde_json::from_slice(&std::fs::read(file)?)?
                } else {
                    Preferences::default()
                }
            };
            DisplaySettings::get(cx).copy_to(&mut preferences);
            #[cfg(not(test))]
            preferences.write()?;
            #[cfg(test)]
            preferences.write_to(&self.preferences_directory.path().join("preferences.json"))?;
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(format!("{}: {error:#}", tr("settings_failed")));
        }
        cx.notify();
    }

    fn busy(&self) -> bool {
        self.task.is_some()
    }

    fn syntax(&self) -> SharedString {
        self.language.clone().unwrap_or_else(|| {
            self.sources
                .iter()
                .rev()
                .find_map(|s| s.path.as_ref())
                .map(|path| DiffFile::unchanged(path.to_string_lossy().into_owned(), "").language())
                .unwrap_or_else(|| "text".into())
        })
    }

    fn set_language(&mut self, language: Option<SharedString>, cx: &mut Context<Self>) {
        self.language = language;
        let syntax = self.syntax();
        for source in &self.sources {
            source
                .editor
                .update(cx, |editor, cx| editor.set_highlighter(syntax.clone(), cx));
        }
        self.diff.update(cx, |diff, cx| {
            let files = diff
                .files()
                .iter()
                .cloned()
                .map(|file| file.with_language(syntax.clone()))
                .collect::<Vec<_>>();
            diff.set_files(files, cx);
        });
        cx.notify();
    }

    fn compare(&mut self, _: &CompareText, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let left = self.sources[0].editor.read(cx).value();
        let right = self.sources[1].editor.read(cx).value();
        let syntax = self.syntax();
        let identical = left == right;
        self.error = None;
        let task = cx.background_spawn(async move {
            text_diff::compare(&left, &right).map(|file| file.with_language(syntax))
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.task = None;
                    match result {
                        Ok(file) => {
                            this.diff.update(cx, |diff, cx| diff.set_files([file], cx));
                            this.result = true;
                            this.identical = identical;
                            window.focus(&this.focus, cx);
                        }
                        Err(error) => {
                            this.error = Some(format!("{}: {error:#}", tr("compare_failed")))
                        }
                    }
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || !self.result {
            return;
        }
        self.result = false;
        window.focus(&self.sources[0].editor.focus_handle(cx), cx);
        cx.notify();
    }

    fn swap(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.sources.swap(0, 1);
        self.set_language(self.language.clone(), cx);
    }

    fn choose_file(&mut self, side: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("compare_open_file").into()),
        });
        cx.spawn_in(window, async move |this, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                let _ = cx.update(|window, cx| {
                    this.update(cx, |this, cx| {
                        this.drop_files(side, &paths, window, cx);
                    })
                });
            }
            Ok(Err(error)) => {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("{}: {error:#}", tr("open_file_failed")));
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    fn drop_files(
        &mut self,
        side: usize,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        if !(1..=2).contains(&paths.len()) {
            self.error = Some(tr("compare_drop_count").into());
            cx.notify();
            return;
        }
        self.load_files(
            paths
                .iter()
                .enumerate()
                .map(|(index, path)| (if paths.len() == 2 { index } else { side }, path.clone()))
                .collect(),
            window,
            cx,
        );
    }

    fn refresh_files(
        &mut self,
        _: &RefreshComparison,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = self
            .sources
            .iter()
            .enumerate()
            .filter_map(|(side, source)| source.path.clone().map(|path| (side, path)))
            .collect();
        self.load_files(paths, window, cx);
    }

    fn load_files(
        &mut self,
        paths: Vec<(usize, PathBuf)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() || paths.is_empty() {
            return;
        }
        self.error = None;
        let mut texts =
            std::array::from_fn::<_, 2, _>(|side| self.sources[side].editor.read(cx).value());
        let mut source_paths =
            std::array::from_fn::<_, 2, _>(|side| self.sources[side].path.clone());
        for (side, path) in &paths {
            source_paths[*side] = Some(path.clone());
        }
        let syntax = self.language.clone().unwrap_or_else(|| {
            source_paths
                .iter()
                .rev()
                .flatten()
                .next()
                .map(|path| DiffFile::unchanged(path.to_string_lossy().into_owned(), "").language())
                .unwrap_or_else(|| "text".into())
        });
        let show_result = self.result;
        let not_text = tr("compare_utf8_required");
        let task = cx.background_spawn(async move {
            let mut loaded = Vec::with_capacity(paths.len());
            for (side, path) in paths {
                let text = text_diff::read_text(&path).map_err(|error| {
                    let reason = match error {
                        ReadTextError::Io(error) => error.to_string(),
                        ReadTextError::NotText => not_text.to_owned(),
                    };
                    anyhow::anyhow!("{} — {reason}", path.display())
                })?;
                texts[side] = text.into();
                loaded.push((side, path, texts[side].clone()));
            }
            let identical = texts[0] == texts[1];
            let preview = show_result
                .then(|| {
                    text_diff::compare(&texts[0], &texts[1]).map(|file| file.with_language(syntax))
                })
                .transpose()?;
            Ok::<_, anyhow::Error>((loaded, preview, identical))
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.task = None;
                    match result {
                        Ok((loaded, preview, identical)) => {
                            // Publish the snapshots and their diff together. Failed reads
                            // leave both sides and the existing result untouched.
                            for (side, path, text) in loaded {
                                this.sources[side].path = Some(path);
                                this.sources[side]
                                    .editor
                                    .update(cx, |editor, cx| editor.set_value(text, window, cx));
                            }
                            let syntax = this.syntax();
                            for source in &this.sources {
                                source.editor.update(cx, |editor, cx| {
                                    editor.set_highlighter(syntax.clone(), cx)
                                });
                            }
                            if let Some(preview) = preview {
                                this.diff
                                    .update(cx, |diff, cx| diff.set_files([preview], cx));
                                this.identical = identical;
                            }
                        }
                        Err(error) => this.error = Some(format!("{error:#}")),
                    }
                    cx.notify();
                })
            });
        }));
        cx.notify();
    }

    fn application_menu(&self, cx: &App) -> impl IntoElement {
        let focus = self.focus.clone();
        div().occlude().child(
            icon_button("comparison-menu", IconName::Menu, "application_menu")
                .text_color(cx.theme().muted_foreground)
                .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, window, cx| {
                    menu.action_context(window.focused(cx).unwrap_or_else(|| focus.clone()))
                        .menu(tr("new_text_comparison"), Box::new(NewTextComparison))
                        .separator()
                        .menu(tr("about"), Box::new(About))
                        .separator()
                        .menu(tr("quit"), Box::new(Quit))
                }),
        )
    }

    fn language_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let selected = self.language.clone();
        let label: SharedString = match self.language.as_deref() {
            None => tr("compare_auto_language").into(),
            Some("text") => tr("compare_plain_text").into(),
            Some(language) => language.to_owned().into(),
        };
        Button::new("comparison-syntax")
            .icon(IconName::Code)
            .small()
            .ghost()
            .disabled(self.busy())
            .text_color(cx.theme().muted_foreground)
            .tooltip(format!("{}: {label}", tr("compare_syntax")))
            .accessibility_label(tr("compare_syntax"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let choices = std::iter::once((tr("compare_auto_language"), None))
                    .chain(std::iter::once((tr("compare_plain_text"), Some("text"))))
                    .chain(SYNTAX_LANGUAGES.iter().map(|name| (*name, Some(*name))));
                choices.fold(
                    menu.min_w(px(160.)).max_h(px(320.)).scrollable(true),
                    |menu, (label, language)| {
                        let view = view.clone();
                        menu.item(
                            PopupMenuItem::new(label)
                                .checked(selected.as_deref() == language)
                                .on_click(move |_, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.set_language(language.map(Into::into), cx)
                                    });
                                }),
                        )
                    },
                )
            })
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let file = self.diff.read(cx).files().first().cloned();
        let has_changes = self.result && !self.identical;
        let folded = self.diff.read(cx).has_collapsed_context();
        // Each toolbar half owns its controls. The right title always starts
        // at the content midpoint, regardless of the result actions' width.
        h_flex()
            .flex_none()
            .h(px(32.))
            .w_full()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .id("comparison-toolbar-left")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px_2()
                    .gap_1()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .child(
                        icon_button("comparison-run", IconName::Play, "compare_start")
                            .text_color(cx.theme().muted_foreground)
                            .disabled(self.busy())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.compare(&CompareText, window, cx)
                            })),
                    )
                    .child(
                        icon_button("comparison-edit", IconName::Pencil, "compare_edit")
                            .text_color(cx.theme().muted_foreground)
                            .disabled(self.busy() || !self.result)
                            .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx))),
                    )
                    .child(toolbar_divider(cx))
                    .child(self.source_control(0, cx))
                    .child(if self.result {
                        h_flex()
                            .w(px(28.))
                            .justify_center()
                            .flex_none()
                            .child(
                                Icon::new(IconName::ArrowRight)
                                    .xsmall()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .into_any_element()
                    } else {
                        icon_button("comparison-swap", IconName::ArrowLeftRight, "compare_swap")
                            .text_color(cx.theme().muted_foreground)
                            .disabled(self.busy())
                            .on_click(cx.listener(|this, _, _, cx| this.swap(cx)))
                            .into_any_element()
                    }),
            )
            .child(
                h_flex()
                    .id("comparison-toolbar-right")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px_2()
                    .gap_1()
                    .child(self.source_control(1, cx))
                    .when(self.result && self.identical, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("compare_identical")),
                        )
                    })
                    .when_some(file.filter(|_| has_changes), |row, file| {
                        row.child(
                            h_flex()
                                .flex_none()
                                .gap_2()
                                .px_2()
                                .text_xs()
                                .child(
                                    div().text_color(cx.theme().muted_foreground).child(
                                        tr("diff_blocks").replace(
                                            "{count}",
                                            &file.change_block_count().to_string(),
                                        ),
                                    ),
                                )
                                .child(
                                    div()
                                        .text_color(cx.theme().success)
                                        .child(format!("+{}", file.additions())),
                                )
                                .child(
                                    div()
                                        .text_color(cx.theme().danger)
                                        .child(format!("−{}", file.deletions())),
                                ),
                        )
                    })
                    .child(toolbar_divider(cx))
                    .when(self.result, |row| {
                        row.child(
                            icon_button(
                                "comparison-context",
                                if folded {
                                    IconName::ChevronsUpDown
                                } else {
                                    IconName::ChevronsDownUp
                                },
                                if folded {
                                    "expand_context"
                                } else {
                                    "collapse_context"
                                },
                            )
                            .text_color(cx.theme().muted_foreground)
                            .disabled(!has_changes)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.diff.update(cx, |diff, cx| {
                                    if diff.has_collapsed_context() {
                                        diff.expand_unchanged(cx);
                                    } else {
                                        diff.collapse_unchanged(cx);
                                    }
                                })
                            })),
                        )
                    })
                    .child(self.language_menu(cx))
                    .child(
                        icon_button("comparison-wrap", IconName::TextWrap, "soft_wrap")
                            .text_color(cx.theme().muted_foreground)
                            .selected(self.soft_wrap)
                            .toggled(self.soft_wrap)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.soft_wrap = !this.soft_wrap;
                                for source in &this.sources {
                                    source.editor.update(cx, |editor, cx| {
                                        editor.set_soft_wrap(this.soft_wrap, window, cx)
                                    });
                                }
                                cx.notify();
                            })),
                    )
                    .when(self.result, |row| {
                        row.children(
                            [
                                (DiffMode::Split, "split", IconName::Columns2),
                                (DiffMode::Unified, "inline", IconName::Rows2),
                            ]
                            .map(|(mode, label, icon)| {
                                icon_button(label, icon, label)
                                    .text_color(cx.theme().muted_foreground)
                                    .selected(self.diff.read(cx).mode() == mode)
                                    .toggled(self.diff.read(cx).mode() == mode)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.diff.update(cx, |diff, cx| diff.set_mode(mode, cx))
                                    }))
                            }),
                        )
                    }),
            )
    }

    fn source_control(&self, side: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let source = &self.sources[side];
        let title = source
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                tr(if side == 0 {
                    "compare_left"
                } else {
                    "compare_right"
                })
                .into()
            });
        h_flex()
            .id(("comparison-source", side))
            .test_support()
            .flex_1()
            .min_w_0()
            .gap_1()
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                this.drop_files(side, &paths.0, window, cx);
            }))
            .child(
                Button::new(("comparison-open", side))
                    .icon(IconName::FolderOpen)
                    .small()
                    .ghost()
                    .text_color(cx.theme().muted_foreground)
                    .tooltip(tr("compare_open_file"))
                    .accessibility_label(tr("compare_open_file"))
                    .disabled(self.busy())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.choose_file(side, window, cx)),
                    ),
            )
            .child(
                div()
                    .id(("comparison-source-name", side))
                    .test_support()
                    .role(accesskit::Role::Label)
                    .aria_label(title.clone())
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    .child(title)
                    .when_some(source.path.as_ref(), |label, path| {
                        let path = path.display().to_string();
                        label
                            .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx))
                    }),
            )
    }

    fn input_pane(&self, side: usize, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id(("comparison-input", side))
            .test_support()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .when(side == 0, |pane| {
                pane.border_r_1().border_color(cx.theme().border)
            })
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().secondary))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                this.drop_files(side, &paths.0, window, cx);
            }))
            .child(
                div().flex_1().min_h_0().min_w_0().overflow_hidden().child(
                    Editor::new(&self.sources[side].editor)
                        .aria_label(tr(if side == 0 {
                            "compare_left"
                        } else {
                            "compare_right"
                        }))
                        .appearance(false)
                        .bordered(false)
                        .readonly(self.busy())
                        .h_full()
                        .when_some(
                            cx.theme().highlight_theme.style.editor_foreground,
                            |editor, color| editor.text_color(color),
                        ),
                ),
            )
    }
}

impl Render for TextComparison {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .text_size(cx.theme().font_size)
            .key_context("TextComparison")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::compare))
            .on_action(cx.listener(Self::refresh_files))
            .on_action(cx.listener(|_, _: &CloseTextComparison, window, _| window.remove_window()))
            .on_action(cx.listener(|this, _: &About, window, cx| {
                this.about.update(cx, |about, cx| about.show(window, cx));
            }))
            .child(
                TitleBar::new().bg(cx.theme().title_bar).child(
                    h_flex()
                        .size_full()
                        .min_w_0()
                        .gap_2()
                        .px_2()
                        .when(
                            cfg!(any(target_os = "windows", target_os = "linux")),
                            |bar| bar.child(self.application_menu(cx)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .max_w(px(240.))
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(tr("text_comparison")),
                        )
                        .child(
                            icon_button(
                                "comparison-refresh",
                                IconName::RefreshCw,
                                "compare_refresh",
                            )
                            .occlude()
                            .xsmall()
                            .text_color(cx.theme().muted_foreground)
                            .disabled(
                                self.busy()
                                    || !self.sources.iter().any(|source| source.path.is_some()),
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.refresh_files(&RefreshComparison, window, cx)
                                },
                            )),
                        )
                        .child(div().flex_1())
                        .child({
                            let view = cx.entity().downgrade();
                            crate::ui::language_menu(move |language, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.set_interface_language(language, window, cx)
                                });
                            })
                        })
                        .child({
                            let view = cx.entity().downgrade();
                            crate::ui::theme_button(cx, move |appearance, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.set_appearance(appearance, window, cx)
                                });
                            })
                        }),
                ),
            )
            .child(self.controls(cx))
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    h_flex()
                        .flex_none()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .bg(cx.theme().danger.opacity(0.12))
                        .child(div().flex_1().text_sm().child(error))
                        .child(
                            icon_button("comparison-dismiss", IconName::X, "dismiss").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.error = None;
                                    cx.notify();
                                }),
                            ),
                        ),
                )
            })
            .child(if self.result {
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        Diff::new(&self.diff)
                            .header_visible(false)
                            .line_number_selection(false)
                            .change_indicator(DiffChangeIndicator::Bars)
                            .hunk_separator(DiffHunkSeparator::Wave)
                            .soft_wrap(self.soft_wrap)
                            .when_some(
                                cx.theme().highlight_theme.style.editor_foreground,
                                |diff, color| diff.text_color(color),
                            )
                            .flex_1()
                            .min_h_0(),
                    )
                    .into_any_element()
            } else {
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .items_stretch()
                    .child(self.input_pane(0, cx))
                    .child(self.input_pane(1, cx))
                    .into_any_element()
            })
            .child(self.about.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::test::TestWindowExt;

    fn setup(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TextComparison>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            component::set_locale("en");
            init(cx);
            open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1120.), px(760.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| TextComparison::new(window, cx)),
            )
            .unwrap()
        })
    }

    #[gpui_kit::test]
    fn display_controls_share_preferences_without_overwriting_repository_settings(
        cx: &mut TestAppContext,
    ) {
        use patchlook::preferences::Layout;
        let (handle, view) = setup(cx);
        let original = Preferences {
            layout: Layout::Inline,
            soft_wrap: false,
            ..Preferences::default()
        };
        let settings_file = view.read_with(cx, |view, _| {
            view.preferences_directory.path().join("preferences.json")
        });
        original.write_to(&settings_file).unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            view.read(cx).sources[0]
                .editor
                .clone()
                .update(cx, |editor, cx| {
                    editor.set_value("keep this text", window, cx)
                });
            window.click("theme", cx);
            assert_eq!(DisplaySettings::get(cx).appearance, Appearance::Light);
            window.click("language", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let chinese = Language::ALL
                .iter()
                .position(|language| *language == Language::SimplifiedChinese)
                .unwrap();
            window.within("popup-menu").click(chinese, cx);
        })
        .unwrap();
        cx.run_until_parked();
        let (other_handle, other) = cx.update(|cx| {
            open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| TextComparison::new(window, cx))
            })
            .unwrap()
        });
        cx.update_window(other_handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                DisplaySettings::get(cx).language,
                Language::SimplifiedChinese
            );
            assert_eq!(&*component::locale(), "zh-CN");
            assert_eq!(
                window.find("comparison-run").label(),
                Some(tr("compare_start"))
            );
            assert_eq!(
                window.find(("comparison-source-name", 0usize)).label(),
                Some(tr("compare_left"))
            );
            window.click("theme", cx);
            assert_eq!(DisplaySettings::get(cx).appearance, Appearance::Dark);
            assert!(other.read(cx).sources[0].editor.read(cx).value().is_empty());
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                view.read(cx).sources[0].editor.read(cx).value().as_str(),
                "keep this text"
            );
            assert_eq!(
                window.find("theme").label(),
                Some(
                    tr("theme_cycle")
                        .replace("{current}", tr("dark"))
                        .replace("{next}", tr("system"))
                        .as_str()
                )
            );
            assert!(!view.read(cx).result);
        })
        .unwrap();
        let saved: Preferences =
            serde_json::from_slice(&std::fs::read(settings_file).unwrap()).unwrap();
        assert_eq!(saved.language, Language::SimplifiedChinese);
        assert_eq!(saved.appearance, Appearance::Light);
        assert_eq!(saved.layout, original.layout);
        assert_eq!(saved.soft_wrap, original.soft_wrap);
    }

    #[gpui_kit::test]
    fn paste_compare_edit_swap_and_compare_again(cx: &mut TestAppContext) {
        let (handle, view) = setup(cx);
        let values = ["old\nunchanged\n中文\n", "new\nunchanged\n中文\nextra"];
        let mut title_positions = [px(0.); 2];
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for (side, position) in title_positions.iter_mut().enumerate() {
                *position = window
                    .find(("comparison-source-name", side))
                    .bounds()
                    .left();
            }
            assert!(
                (window.find("comparison-toolbar-right").bounds().left()
                    - window.find(("comparison-input", 1usize)).bounds().left())
                .abs()
                    < px(1.)
            );
            for (side, value) in values.iter().enumerate() {
                window.focus(&view.read(cx).sources[side].editor.focus_handle(cx), cx);
                window.render_frame(cx);
                cx.write_to_clipboard(ClipboardItem::new_string((*value).into()));
                window.press(
                    if cfg!(target_os = "macos") {
                        "cmd-v"
                    } else {
                        "ctrl-v"
                    },
                    cx,
                );
                assert_eq!(
                    view.read(cx).sources[side].editor.read(cx).value().as_str(),
                    *value
                );
            }
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-enter"
                } else {
                    "ctrl-enter"
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(view.read(cx).result);
            assert!(!view.read(cx).identical);
            assert_eq!(view.read(cx).diff.read(cx).files()[0].additions(), 2);
            assert!(window.try_find("stage-current-file").is_none());
            assert!(window.try_find("comparison-run").is_some());
            assert!(window.try_find("comparison-previous").is_none());
            assert!(window.try_find("comparison-next").is_none());
            for (side, position) in title_positions.iter().enumerate() {
                assert!(
                    (window
                        .find(("comparison-source-name", side))
                        .bounds()
                        .left()
                        - *position)
                        .abs()
                        < px(1.)
                );
                assert!(window.try_find(("comparison-open", side)).is_some());
            }
            window.click("inline", cx);
            assert_eq!(view.read(cx).diff.read(cx).mode(), DiffMode::Unified);
            window.click("comparison-edit", cx);
            window.render_frame(cx);
            assert!(!view.read(cx).result);
            window.click("comparison-swap", cx);
            assert_eq!(
                view.read(cx).sources[0].editor.read(cx).value().as_str(),
                values[1]
            );
            assert_eq!(
                view.read(cx).sources[1].editor.read(cx).value().as_str(),
                values[0]
            );
            view.read(cx).sources[0]
                .editor
                .clone()
                .update(cx, |editor, cx| editor.set_value(values[0], window, cx));
            window.click("comparison-run", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(view.read(cx).result && view.read(cx).identical);
            assert_eq!(
                view.read(cx).diff.read(cx).files()[0].change_block_count(),
                0
            );
            window.click("comparison-edit", cx);
            for source in &view.read(cx).sources {
                assert_eq!(source.editor.read(cx).value().as_str(), values[0]);
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn file_drop_imports_two_snapshots_and_failed_import_preserves_text(cx: &mut TestAppContext) {
        let (handle, view) = setup(cx);
        let directory = tempfile::tempdir().unwrap();
        let paths = [
            directory.path().join("左 [draft].rs"),
            directory.path().join("right.rs"),
        ];
        let values = ["let value = 1;\r\n", "let value = 2;\n"];
        for (path, value) in paths.iter().zip(values) {
            std::fs::write(path, value).unwrap();
        }
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let position = window.find(("comparison-input", 1usize)).bounds().center();
            window.dispatch_event(
                FileDropEvent::Entered {
                    position,
                    paths: ExternalPaths(paths.clone().into()),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            for (side, value) in values.iter().enumerate() {
                assert_eq!(
                    view.read(cx).sources[side].path.as_ref(),
                    Some(&paths[side])
                );
                assert_eq!(
                    view.read(cx).sources[side].editor.read(cx).value().as_str(),
                    *value
                );
            }
            assert_eq!(view.read(cx).syntax().as_ref(), "rust");
            window.click(("comparison-open", 0usize), cx);
        })
        .unwrap();
        assert!(cx.did_prompt_for_paths());
        let binary = directory.path().join("binary.bin");
        std::fs::write(&binary, [0, 255]).unwrap();
        cx.simulate_path_prompt_response(|_| Some(vec![binary]));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                view.read(cx)
                    .error
                    .as_ref()
                    .unwrap()
                    .contains(tr("compare_utf8_required"))
            );
            assert_eq!(
                view.read(cx).sources[0].editor.read(cx).value().as_str(),
                values[0]
            );
            window.click("comparison-run", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let pasted = "A temporary replacement\n粘贴的文本\n";
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("comparison-edit", cx);
            window.render_frame(cx);
            assert!(!view.read(cx).result);
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-a"
                } else {
                    "ctrl-a"
                },
                cx,
            );
            cx.write_to_clipboard(ClipboardItem::new_string(pasted.into()));
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-v"
                } else {
                    "ctrl-v"
                },
                cx,
            );
            assert_eq!(
                view.read(cx).sources[0].editor.read(cx).value().as_str(),
                pasted
            );
            window.click("comparison-run", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_displayed_sources(&view, [pasted, values[1]], cx);
        for (path, value) in paths.iter().zip(values) {
            assert_eq!(std::fs::read_to_string(path).unwrap(), value);
        }
        view.read_with(cx, |view, _| assert!(view.result && !view.identical));
    }

    #[gpui_kit::test]
    fn refresh_reloads_both_files_atomically_and_allows_replacing_a_result_source(
        cx: &mut TestAppContext,
    ) {
        let (handle, view) = setup(cx);
        let directory = tempfile::tempdir().unwrap();
        let paths = [
            directory.path().join("left.txt"),
            directory.path().join("right.txt"),
        ];
        let initial = ["left\n", "right\n"];
        let updated = ["left from disk\n", "right from disk\n"];
        for (path, text) in paths.iter().zip(initial) {
            std::fs::write(path, text).unwrap();
        }
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.drop_files(0, &paths, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("comparison-edit").is_some());
            window.click("comparison-run", cx);
        })
        .unwrap();
        cx.run_until_parked();
        for (path, text) in paths.iter().zip(updated) {
            std::fs::write(path, text).unwrap();
        }
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("comparison-edit").is_some());
            window.click("comparison-refresh", cx);
            assert!(view.read(cx).result);
            assert_eq!(
                view.read(cx).sources[0].editor.read(cx).value().as_str(),
                initial[0]
            );
        })
        .unwrap();
        cx.run_until_parked();
        assert_displayed_sources(&view, updated, cx);
        std::fs::write(&paths[0], "do not partially publish this\n").unwrap();
        std::fs::remove_file(&paths[1]).unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.click("comparison-refresh", cx)
        })
        .unwrap();
        cx.run_until_parked();
        assert_displayed_sources(&view, updated, cx);
        view.read_with(cx, |view, _| assert!(view.error.is_some()));
        let replacement = directory.path().join("replacement.txt");
        std::fs::write(&replacement, "new source\n").unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("comparison-open", 1usize), cx);
        })
        .unwrap();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|_| Some(vec![replacement.clone()]));
        cx.run_until_parked();
        assert_displayed_sources(&view, [updated[0], "new source\n"], cx);
        view.read_with(cx, |view, _| {
            assert_eq!(view.sources[1].path.as_ref(), Some(&replacement));
            assert!(view.error.is_none());
        });
    }

    fn assert_displayed_sources(
        view: &Entity<TextComparison>,
        expected: [&str; 2],
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::component::diff::{DiffLineRange, DiffSide};
        view.update(cx, |view, cx| {
            assert!(view.result);
            for (side, text) in expected.iter().enumerate() {
                assert_eq!(view.sources[side].editor.read(cx).value().as_str(), *text);
                view.diff.update(cx, |diff, cx| {
                    let side = if side == 0 {
                        DiffSide::Original
                    } else {
                        DiffSide::Modified
                    };
                    let range = DiffLineRange::new(
                        diff.files()[0].path().clone(),
                        side,
                        1,
                        text.lines().count().max(1),
                    );
                    diff.set_selected_lines(Some(range), cx);
                    assert_eq!(diff.selected_text(cx), *text);
                });
            }
        });
    }

    #[gpui_kit::test]
    fn refresh_preserves_the_pasted_side_and_updates_the_result(cx: &mut TestAppContext) {
        let (handle, view) = setup(cx);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.txt");
        std::fs::write(&path, "old\n").unwrap();
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.drop_files(0, std::slice::from_ref(&path), window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            view.read(cx).sources[1]
                .editor
                .clone()
                .update(cx, |editor, cx| {
                    editor.set_value("pasted text\n", window, cx)
                });
            window.render_frame(cx);
            window.click("comparison-run", cx);
        })
        .unwrap();
        cx.run_until_parked();
        std::fs::write(&path, "updated\r\n").unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("comparison-edit").is_some());
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-r"
                } else {
                    "ctrl-r"
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        assert_displayed_sources(&view, ["updated\r\n", "pasted text\n"], cx);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "updated\r\n");
    }

    #[gpui_kit::test]
    fn empty_comparison_and_new_window_shortcut(cx: &mut TestAppContext) {
        let (handle, view) = setup(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("comparison-run", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(view.read(cx).result && view.read(cx).identical);
            assert_eq!(cx.windows().len(), 1);
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-n"
                } else {
                    "ctrl-n"
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(cx.windows().len(), 2);
            assert!(view.read(cx).result);
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-w"
                } else {
                    "ctrl-w"
                },
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.read(|cx| assert_eq!(cx.windows().len(), 1));
    }
}
