#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod about;
mod app;
mod change_tree;
mod file_icons;
mod image_diff;
mod strings;
use app::{About, OpenRepository, Patchlook, Quit, Refresh};
use gpui_kit::*;
use strings::tr;

fn main() {
    let repository = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    application()
        .with_assets(file_icons::AppAssets)
        .run(move |cx| {
            init(cx);
            let dark = std::rc::Rc::new(
                serde_json::from_str::<component::ThemeConfig>(include_str!("../themes/dark.json"))
                    .expect("The bundled dark theme must be valid"),
            );
            let light_overrides: component::ThemeConfig =
                serde_json::from_str(include_str!("../themes/light.json"))
                    .expect("The bundled light theme must be valid");
            let mut light = component::Theme::global(cx).light_theme.as_ref().clone();
            light.name = light_overrides.name;
            light.colors.accent = light_overrides.colors.accent;
            light.colors.list_hover = light_overrides.colors.list_hover;
            light.colors.secondary = light_overrides.colors.secondary;
            light.colors.secondary_hover = light_overrides.colors.secondary_hover;
            light.colors.secondary_active = light_overrides.colors.secondary_active;
            light.colors.button_active = light_overrides.colors.button_active;
            if let Some(overrides) = light_overrides.highlight {
                let highlight = light
                    .highlight
                    .as_mut()
                    .expect("The default light theme must include syntax colors");
                highlight.editor_foreground = overrides.editor_foreground;
                highlight.syntax.variable = overrides.syntax.variable;
                highlight.syntax.property = overrides.syntax.property;
                highlight.syntax.embedded = overrides.syntax.embedded;
            }
            component::Theme::update(cx, |theme| {
                theme.dark_theme = dark;
                theme.light_theme = std::rc::Rc::new(light);
            });
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("secondary-o", OpenRepository, None),
                KeyBinding::new("secondary-r", Refresh, None),
                KeyBinding::new("f5", Refresh, None),
                KeyBinding::new("secondary-q", Quit, None),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
            open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.), px(560.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(tr("app_name").into()),
                        ..component::TitleBar::title_bar_options()
                    }),
                    app_id: Some(concat!("dev.", env!("CARGO_PKG_NAME"), ".app").into()),
                    ..component::TitleBar::window_options()
                },
                cx,
                |window, cx| cx.new(|cx| Patchlook::new(repository, window, cx)),
            )
            .expect("Cannot open Patchlook");
            cx.activate(true);
        });
}

fn update_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            disabled: false,
            name: tr("app_name").into(),
            items: vec![
                MenuItem::action(tr("about"), About),
                MenuItem::separator(),
                MenuItem::action(tr("quit"), Quit),
            ],
        },
        Menu {
            disabled: false,
            name: tr("file_menu").into(),
            items: vec![
                MenuItem::action(tr("open_repository"), OpenRepository),
                MenuItem::action(tr("refresh"), Refresh),
            ],
        },
        Menu {
            disabled: false,
            name: tr("edit_menu").into(),
            items: vec![
                MenuItem::action(tr("copy"), component::input::Copy),
                MenuItem::action(tr("select_all"), component::input::SelectAll),
            ],
        },
    ]);
}
