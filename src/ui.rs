use crate::strings::{language_name, tr};
use gpui_kit::{
    assets::IconName,
    component::{
        ActiveTheme, Sizable, Theme, ThemeMode,
        button::{Button, ButtonVariants},
        menu::{DropdownMenu, PopupMenuItem},
    },
    *,
};
use patchlook::preferences::{Appearance, Language, Preferences};
use std::rc::Rc;

/// Maximum source-line length in UTF-8 bytes for word-level diff emphasis.
pub const DIFF_INLINE_MAX_LINE_BYTES: usize = 3_000;

/// Application-wide choices shared by repository and scratch windows.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplaySettings {
    pub language: Language,
    pub appearance: Appearance,
}

impl Global for DisplaySettings {}

impl DisplaySettings {
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }

    pub fn initialize(preferences: &Preferences, cx: &mut App) {
        if !cx.has_global::<Self>() {
            cx.set_global(Self {
                language: preferences.language,
                appearance: preferences.appearance,
            });
        }
    }

    pub fn copy_to(self, preferences: &mut Preferences) {
        preferences.language = self.language;
        preferences.appearance = self.appearance;
    }

    fn publish(self, cx: &mut App) {
        if cx.try_global::<Self>() != Some(&self) {
            cx.set_global(self);
        }
    }
}

pub fn apply_language(language: Language, cx: &mut App) {
    DisplaySettings {
        language,
        ..DisplaySettings::get(cx)
    }
    .publish(cx);
    let resolved = language.resolve(sys_locale::get_locales());
    let code = resolved.code().expect("Resolved language has a locale");
    if &*component::locale() != code {
        component::set_locale(code);
        cx.refresh_windows();
    }
    crate::update_menus(cx);
}

pub fn apply_appearance(appearance: Appearance, window: &mut Window, cx: &mut App) {
    DisplaySettings {
        appearance,
        ..DisplaySettings::get(cx)
    }
    .publish(cx);
    cx.set_window_appearance(match appearance {
        Appearance::System => None,
        Appearance::Light => Some(WindowAppearance::Light),
        Appearance::Dark => Some(WindowAppearance::Dark),
    });
    match appearance {
        Appearance::System => Theme::sync_system_appearance(Some(window), cx),
        Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
    }
}

pub fn language_menu(
    on_select: impl Fn(Language, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_select = Rc::new(on_select);
    div().occlude().child(
        Button::new("language")
            .icon(IconName::Languages)
            .child(div().text_xs().child(tr("language_short")))
            .small()
            .ghost()
            .tooltip(tr("language"))
            .accessibility_label(tr("language"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
                let selected = DisplaySettings::get(cx).language;
                Language::ALL.into_iter().fold(
                    menu.min_w(px(210.))
                        .max_h(px(360.))
                        .scrollable(true)
                        .check_side(component::Side::Right),
                    |menu, language| {
                        let on_select = on_select.clone();
                        menu.item(
                            PopupMenuItem::new(language_name(language))
                                .checked(selected == language)
                                .on_click(move |_, window, cx| on_select(language, window, cx)),
                        )
                    },
                )
            }),
    )
}

pub fn theme_button(
    cx: &App,
    on_select: impl Fn(Appearance, &mut Window, &mut App) + 'static,
) -> Button {
    let current = DisplaySettings::get(cx).appearance;
    let (icon, current_label) = appearance_label(current);
    let (_, next_label) = appearance_label(current.next());
    let label = tr("theme_cycle")
        .replace("{current}", tr(current_label))
        .replace("{next}", tr(next_label));
    Button::new("theme")
        .occlude()
        .icon(icon)
        .small()
        .ghost()
        .tooltip(label.clone())
        .accessibility_label(label)
        .on_click(move |_, window, cx| {
            on_select(DisplaySettings::get(cx).appearance.next(), window, cx)
        })
}

fn appearance_label(appearance: Appearance) -> (IconName, &'static str) {
    match appearance {
        Appearance::Light => (IconName::Sun, "light"),
        Appearance::Dark => (IconName::Moon, "dark"),
        Appearance::System => (IconName::Monitor, "system"),
    }
}

pub fn icon_button(id: &'static str, icon: IconName, label: &'static str) -> Button {
    Button::new(id)
        .icon(icon)
        .small()
        .ghost()
        .tooltip(tr(label))
        .accessibility_label(tr(label))
}

pub fn toolbar_divider(cx: &App) -> Div {
    div()
        .flex_none()
        .w(px(1.))
        .h(px(14.))
        .mx_1()
        .bg(cx.theme().border)
}
