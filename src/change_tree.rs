use crate::{
    file_icons::file_icon,
    strings::{area_label, tr},
};
use gpui_kit::{
    assets::IconName,
    base::actions::{SelectLeft, SelectRight},
    base::{TestSupportExt, Tree, TreeItem, TreeState},
    component::{
        ActiveTheme, Icon, Sizable, h_flex,
        scroll::{Scrollbar, ScrollbarMode},
        tooltip::Tooltip,
    },
    prelude::FluentBuilder,
    *,
};
use patchlook::git::{Area, Change, DiffStats};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    path::Path,
    rc::Rc,
};

/// The optional index identifies a file; otherwise the path identifies a directory.
pub type EntryActionRenderer =
    Rc<dyn Fn(Option<usize>, &str, &str, &[usize], &mut App) -> AnyElement>;

struct Node {
    file_index: Option<usize>,
    file_count: usize,
    status: char,
    path: SharedString,
    move_source: Option<SharedString>,
    stats: Option<DiffStats>,
    area: Option<Area>,
    files: Vec<usize>,
    group: Option<bool>,
}

pub struct ChangeTree {
    state: Entity<TreeState>,
    nodes: Rc<HashMap<SharedString, Node>>,
    _subscription: Subscription,
    progress: Vec<Option<(usize, usize)>>,
}

#[derive(Default)]
struct Directory {
    directories: BTreeMap<OsString, Directory>,
    files: Vec<usize>,
}

impl Directory {
    fn from_changes(changes: &[Change]) -> Self {
        let mut root = Self::default();
        for (index, change) in changes.iter().enumerate() {
            let mut directory = &mut root;
            if let Some(parent) = change.path.parent() {
                for segment in parent.components() {
                    directory = directory
                        .directories
                        .entry(segment.as_os_str().to_owned())
                        .or_default();
                }
            }
            directory.files.push(index);
        }
        root
    }

    fn group(changes: &[Change], untracked: bool) -> Self {
        let mut directory = Self::from_changes(changes);
        fn retain(directory: &mut Directory, changes: &[Change], untracked: bool) {
            directory
                .files
                .retain(|&index| (changes[index].area == Area::Untracked) == untracked);
            directory.directories.retain(|_, child| {
                retain(child, changes, untracked);
                !child.files.is_empty() || !child.directories.is_empty()
            });
        }
        retain(&mut directory, changes, untracked);
        directory
    }

    fn into_item(
        mut self,
        mut label: String,
        path: &Path,
        root: bool,
        changes: &[Change],
        stats: &[Option<DiffStats>],
        nodes: &mut HashMap<SharedString, Node>,
    ) -> TreeItem {
        let mut full_path = path.to_path_buf();
        // Compact only linear directory chains. Keep the repository root visible.
        while !root && self.files.is_empty() && self.directories.len() == 1 {
            let (name, child) = self.directories.pop_first().unwrap();
            label.push('/');
            label.push_str(&name.to_string_lossy());
            full_path.push(name);
            self = child;
        }
        let mut children = Vec::new();
        for (name, directory) in self.directories {
            children.push(directory.into_item(
                name.to_string_lossy().into_owned(),
                &full_path.join(&name),
                false,
                changes,
                stats,
                nodes,
            ));
        }
        self.files.sort_by(|a, b| {
            changes[*a]
                .path
                .file_name()
                .cmp(&changes[*b].path.file_name())
        });
        for index in self.files {
            let change = &changes[index];
            let id = file_id(index);
            nodes.insert(
                id.clone(),
                Node {
                    file_index: Some(index),
                    file_count: 1,
                    status: change.status,
                    path: change.path.display().to_string().into(),
                    move_source: move_source(change),
                    stats: stats[index],
                    area: Some(change.area),
                    files: vec![index],
                    group: None,
                },
            );
            children.push(TreeItem::new(
                id,
                change
                    .path
                    .file_name()
                    .unwrap_or(change.path.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        let files = children
            .iter()
            .flat_map(|child| nodes[&child.id].files.iter().copied())
            .collect();
        let count = children
            .iter()
            .map(|child| nodes[&child.id].file_count)
            .sum();
        let stats = children
            .iter()
            .filter_map(|child| nodes[&child.id].stats)
            .sum();
        let id: SharedString = if root {
            "tree-root".into()
        } else {
            format!("tree-directory-{}", nodes.len()).into()
        };
        nodes.insert(
            id.clone(),
            Node {
                file_index: None,
                file_count: count,
                status: ' ',
                path: full_path.display().to_string().into(),
                move_source: None,
                stats: Some(stats),
                area: None,
                files,
                group: None,
            },
        );
        TreeItem::new(id, label).children(children).expanded(true)
    }
}

fn file_id(index: usize) -> SharedString {
    format!("tree-file-{index}").into()
}

fn move_source(change: &Change) -> Option<SharedString> {
    if change.status != 'R' {
        return None;
    }
    let previous = change.previous_path.as_deref()?;
    let same_name = previous.file_name() == change.path.file_name();
    let source = previous
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect::<Vec<_>>();
    let destination = change
        .path
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect::<Vec<_>>();
    let common = source
        .iter()
        .zip(&destination)
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts = vec!["..".to_owned(); destination.len() - common];
    parts.extend(
        source[common..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );
    if !same_name {
        parts.push(
            previous
                .file_name()
                .unwrap_or(previous.as_os_str())
                .to_string_lossy()
                .into_owned(),
        );
    }
    let mut source = parts.join("/");
    if same_name {
        source.push('/');
    }
    Some(source.into())
}

fn has_collapsed_directory(item: &TreeItem) -> bool {
    item.is_folder() && (!item.is_expanded() || item.children.iter().any(has_collapsed_directory))
}

fn navigate_tree(state: &Entity<TreeState>, right: bool, cx: &mut App) -> bool {
    state.update(cx, |tree, cx| {
        let Some(index) = tree.selected_index() else {
            return false;
        };
        let Some(entry) = tree.entry(index) else {
            return false;
        };
        if entry.is_folder() && entry.is_expanded() != right {
            // Let Tree's existing action expand or collapse the selected folder.
            return false;
        }
        let next = if right {
            tree.entry(index + 1).map(|_| index + 1)
        } else {
            let depth = entry.depth();
            (0..index).rev().find(|index| {
                tree.entry(*index)
                    .is_some_and(|entry| entry.depth() < depth)
            })
        };
        if let Some(next) = next {
            tree.set_selected_index(Some(next), cx);
            tree.scroll_to_item(next, ScrollStrategy::Center);
            true
        } else {
            false
        }
    })
}

fn set_directories_expanded(mut item: TreeItem, expanded: bool) -> TreeItem {
    if item.is_folder() {
        item.children = item
            .children
            .into_iter()
            .map(|child| set_directories_expanded(child, expanded))
            .collect();
        item = item.expanded(expanded);
    }
    item
}

pub fn working_caption(area: Area, progress: Option<(usize, usize)>) -> String {
    if area == Area::Partial
        && let Some((staged, total)) = progress.filter(|(_, total)| *total > 0)
    {
        tr("staging_progress")
            .replace("{staged}", &staged.to_string())
            .replace("{total}", &total.to_string())
    } else {
        area_label(area).to_owned()
    }
}

pub fn file_status_color(status: char, cx: &App) -> Hsla {
    match status {
        'A' | 'C' => cx.theme().green,
        'D' => cx.theme().muted_foreground,
        'U' => cx.theme().danger,
        _ => cx.theme().blue,
    }
}

pub fn stats_label(stats: Option<DiffStats>, cx: &App) -> impl IntoElement {
    h_flex()
        .id("file-diff-stats")
        .test_support()
        .flex_none()
        .gap_2()
        .text_xs()
        .role(accesskit::Role::Label)
        .map(|row| match stats {
            Some(stats) => row
                .aria_label(format!("+{} −{}", stats.additions, stats.deletions))
                .child(
                    div()
                        .text_color(cx.theme().success)
                        .child(format!("+{}", stats.additions)),
                )
                .child(
                    div()
                        .text_color(cx.theme().danger)
                        .child(format!("−{}", stats.deletions)),
                ),
            None => row
                .aria_label(tr("binary_file"))
                .text_color(cx.theme().muted_foreground)
                .child(tr("binary_file")),
        })
}

impl ChangeTree {
    pub fn new<T: 'static>(
        changes: &[Change],
        stats: &[Option<DiffStats>],
        repository: &Path,
        working: bool,
        cx: &mut Context<T>,
        on_file: impl Fn(&mut T, usize, &mut Context<T>) + 'static,
    ) -> Self {
        let mut nodes = HashMap::new();
        let mut root = Directory::from_changes(changes).into_item(
            repository
                .file_name()
                .unwrap_or(repository.as_os_str())
                .to_string_lossy()
                .into_owned(),
            Path::new(""),
            true,
            changes,
            stats,
            &mut nodes,
        );
        if working {
            let parent = nodes.remove(&root.id).unwrap();
            nodes.clear();
            let mut groups = Vec::new();
            for untracked in [false, true] {
                let directory = Directory::group(changes, untracked);
                if directory.files.is_empty() && directory.directories.is_empty() {
                    continue;
                }
                let start = nodes
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                let mut group = directory.into_item(
                    tr(if untracked {
                        "untracked_files"
                    } else {
                        "tracked_changes"
                    })
                    .to_owned(),
                    Path::new(""),
                    true,
                    changes,
                    stats,
                    &mut nodes,
                );
                for (_, node) in nodes.iter_mut().filter(|(id, _)| !start.contains(*id)) {
                    node.group = Some(untracked);
                }
                let id: SharedString = if untracked {
                    "tree-untracked"
                } else {
                    "tree-tracked"
                }
                .into();
                let node = nodes.remove(&group.id).unwrap();
                nodes.insert(id.clone(), node);
                group.id = id;
                groups.push(group);
            }
            nodes.insert(root.id.clone(), parent);
            root.children = groups;
        }
        nodes.get_mut(&root.id).unwrap().path = repository.display().to_string().into();
        let state = cx.new(|cx| TreeState::new(cx).items(vec![root]));
        let nodes = Rc::new(nodes);
        let selection_nodes = nodes.clone();
        let subscription = cx.observe(&state, move |owner, state, cx| {
            let file = state
                .read(cx)
                .selected_item()
                .and_then(|item| selection_nodes[&item.id].file_index);
            if let Some(index) = file {
                on_file(owner, index, cx);
            }
            cx.notify();
        });
        Self {
            state,
            nodes,
            _subscription: subscription,
            progress: Vec::new(),
        }
    }

    pub fn set_progress(&mut self, progress: Vec<Option<(usize, usize)>>) {
        self.progress = progress;
    }

    pub fn update_group_labels(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            let Some(mut root) = state.entry(0).map(|entry| entry.item().clone()) else {
                return;
            };
            let mut changed = false;
            for group in &mut root.children {
                let key = match group.id.as_str() {
                    "tree-tracked" => "tracked_changes",
                    "tree-untracked" => "untracked_files",
                    _ => continue,
                };
                if group.label.as_str() != tr(key) {
                    group.label = tr(key).into();
                    changed = true;
                }
            }
            if changed {
                let selected = state.selected_index();
                state.set_items(vec![root], cx);
                state.set_selected_index(selected, cx);
            }
        });
    }

    pub fn has_collapsed_directories(&self, cx: &App) -> bool {
        self.state
            .read(cx)
            .entry(0)
            .is_some_and(|root| has_collapsed_directory(root.item()))
    }

    pub fn toggle_directories(&self, cx: &mut App) {
        let expand = self.has_collapsed_directories(cx);
        self.state.update(cx, |state, cx| {
            let Some(root) = state.entry(0).map(|entry| entry.item().clone()) else {
                return;
            };
            let selected = state.selected_item().map(|item| item.id.clone());
            state.set_items(vec![set_directories_expanded(root, expand)], cx);
            state.set_selected_index(selected.and_then(|id| state.index_of(&id)), cx);
            state.scroll_to_item(0, ScrollStrategy::Top);
        });
    }

    pub fn reveal_file(&self, index: usize, cx: &mut App) {
        let id = file_id(index);
        self.state.update(cx, |state, cx| {
            state.reveal_item(&id, ScrollStrategy::Center, cx);
            state.set_selected_index(state.index_of(&id), cx);
        });
    }

    pub fn restore_expansion(&self, previous: &Self, cx: &mut App) {
        fn collect(
            item: &TreeItem,
            nodes: &HashMap<SharedString, Node>,
            expanded: &mut HashMap<(Option<bool>, SharedString), bool>,
        ) {
            if item.is_folder() {
                let node = &nodes[&item.id];
                expanded.insert((node.group, node.path.clone()), item.is_expanded());
                for child in &item.children {
                    collect(child, nodes, expanded);
                }
            }
        }
        fn restore(
            mut item: TreeItem,
            nodes: &HashMap<SharedString, Node>,
            expanded: &HashMap<(Option<bool>, SharedString), bool>,
        ) -> TreeItem {
            if item.is_folder() {
                if let Some(value) =
                    expanded.get(&(nodes[&item.id].group, nodes[&item.id].path.clone()))
                {
                    item = item.expanded(*value);
                }
                item.children = item
                    .children
                    .into_iter()
                    .map(|child| restore(child, nodes, expanded))
                    .collect();
            }
            item
        }
        let mut expanded = HashMap::new();
        if let Some(root) = previous.state.read(cx).entry(0) {
            collect(root.item(), &previous.nodes, &mut expanded);
        }
        self.state.update(cx, |state, cx| {
            let selected = state.selected_item().map(|item| item.id.clone());
            if let Some(root) = state.entry(0).map(|entry| entry.item().clone()) {
                state.set_items(vec![restore(root, &self.nodes, &expanded)], cx);
                state.set_selected_index(selected.and_then(|id| state.index_of(&id)), cx);
            }
        });
    }

    pub fn render(
        &self,
        branch: Option<String>,
        show_stats: bool,
        working: bool,
        on_activate_file: impl Fn(&usize, &mut Window, &mut App) + 'static,
        entry_action: Option<EntryActionRenderer>,
        cx: &App,
    ) -> impl IntoElement {
        let on_activate_file = Rc::new(on_activate_file);
        let nodes = self.nodes.clone();
        let progress = self.progress.clone();
        let left_state = self.state.clone();
        let right_state = self.state.clone();
        let focus_state = self.state.clone();
        let scroll = self
            .state
            .read_with(cx, |state, _| state.scroll_handle().clone());
        div()
            .relative()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                focus_state.update(cx, |tree, cx| tree.focus(window, cx));
            })
            .capture_action(move |_: &SelectLeft, _, cx| {
                if navigate_tree(&left_state, false, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            })
            .capture_action(move |_: &SelectRight, _, cx| {
                if navigate_tree(&right_state, true, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            })
            .child(
                Tree::new(&self.state)
                    .size_full()
                    .list_style(StyleRefinement::default().size_full())
                    .item(move |_, entry, state, _, cx| {
                        let item = entry.item();
                        let node = &nodes[&item.id];
                        let file_index = node.file_index;
                        let action_id = if entry.is_root() {
                            String::new()
                        } else if node.path.is_empty() {
                            item.id.to_string()
                        } else if node.group == Some(true) {
                            format!("untracked:{}", node.path)
                        } else {
                            node.path.to_string()
                        };
                        let activate = on_activate_file.clone();
                        let move_annotation = node.move_source.as_ref().map(|source| {
                            (
                                format!("← {source}"),
                                tr("moved_from").replace("{path}", source),
                            )
                        });
                        let tooltip = match &move_annotation {
                            Some((_, label)) => format!("{}\n{label}", node.path).into(),
                            None => node.path.clone(),
                        };
                        let status_color = file_status_color(node.status, cx);
                        let row_height = px(22.);
                        h_flex()
                            .id(item.id.clone())
                            .test_support()
                            .h(row_height)
                            .w_full()
                            .min_w_0()
                            .pr_2()
                            .pl(px(8. + entry.depth() as f32 * 18.))
                            .gap_1()
                            .text_sm()
                            .line_height(row_height)
                            .cursor_pointer()
                            .when(state.is_selected(), |row| {
                                row.bg(cx.theme().secondary_active).when_some(
                                    file_index,
                                    |row, index| {
                                        // Activating an already-selected file can reopen its preview.
                                        row.on_click(move |_, window, cx| {
                                            activate(&index, window, cx)
                                        })
                                    },
                                )
                            })
                            .hover(|style| {
                                style.bg(if state.is_selected() {
                                    cx.theme().secondary_active
                                } else {
                                    cx.theme().list_hover
                                })
                            })
                            .child(
                                h_flex()
                                    .w(px(12.))
                                    .flex_none()
                                    .justify_center()
                                    .text_xs()
                                    .text_color(status_color)
                                    .when(entry.is_folder(), |slot| {
                                        slot.child(
                                            Icon::new(if entry.is_expanded() {
                                                IconName::ChevronDown
                                            } else {
                                                IconName::ChevronRight
                                            })
                                            .xsmall()
                                            .text_color(cx.theme().muted_foreground),
                                        )
                                    }),
                            )
                            .when_some(entry_action.as_ref(), |row, render| {
                                row.child(render(
                                    file_index,
                                    &action_id,
                                    if entry.is_root() {
                                        ""
                                    } else if node.path.is_empty() {
                                        item.label.as_str()
                                    } else {
                                        node.path.as_str()
                                    },
                                    &node.files,
                                    cx,
                                ))
                            })
                            .child(if file_index.is_some() {
                                file_icon(Path::new(node.path.as_str())).into_any_element()
                            } else {
                                Icon::default()
                                    .path("ui-icons/folder.svg")
                                    .size(px(16.))
                                    .flex_none()
                                    .text_color(cx.theme().muted_foreground)
                                    .into_any_element()
                            })
                            .child(
                                div()
                                    .id(format!("tree-name:{}", item.id))
                                    .min_w_0()
                                    .truncate()
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(tooltip.clone()).build(window, cx)
                                    })
                                    .when(file_index.is_some(), |label| {
                                        label.text_color(status_color)
                                    })
                                    .child(item.label.clone()),
                            )
                            .when_some(move_annotation, |row, (source, label)| {
                                row.child(
                                    div()
                                        .id("tree-move-source")
                                        .test_support()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .role(accesskit::Role::Label)
                                        .aria_label(label)
                                        .child(source),
                                )
                            })
                            .when(file_index.is_none(), |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            tr("tree_file_count")
                                                .replace("{count}", &node.file_count.to_string()),
                                        ),
                                )
                            })
                            .when_some(
                                branch.as_ref().filter(|_| entry.is_root()),
                                |row, branch| {
                                    row.child(
                                        div()
                                            .id("tree-branch")
                                            .test_support()
                                            .min_w_0()
                                            .truncate()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(branch.clone()),
                                    )
                                },
                            )
                            .when(working && file_index.is_some(), |row| {
                                row.child(div().flex_1()).child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(working_caption(
                                            node.area.unwrap(),
                                            file_index
                                                .and_then(|i| progress.get(i).copied().flatten()),
                                        )),
                                )
                            })
                            .when(
                                show_stats && (file_index.is_some() || !entry.is_expanded()),
                                |row| {
                                    row.when(!(working && file_index.is_some()), |row| {
                                        row.child(div().flex_1())
                                    })
                                    .child(stats_label(node.stats, cx))
                                },
                            )
                            .into_any_element()
                    }),
            )
            .child(
                div().absolute().inset_0().child(
                    Scrollbar::vertical(&scroll)
                        .mode(ScrollbarMode::Scrolling)
                        .viewport_from_layout()
                        .styles(|styles| {
                            styles
                                .track(|track| track.width(px(6.)))
                                .thumb(|thumb| thumb.width(px(6.)).inset(px(0.)).radius(px(2.)))
                                .thumb_hover(|thumb| thumb.width(px(6.)).inset(px(0.)))
                                .thumb_active(|thumb| thumb.width(px(6.)).inset(px(0.)))
                        }),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::test::TestWindowExt;
    use patchlook::git::Area;
    use std::cell::Cell;

    fn changes() -> Vec<Change> {
        [
            "README.md",
            "crates/model/src/specs/a.rs",
            "crates/model/src/specs/b.rs",
            ".codex/project-state.md",
            "docs/guides/setup.md",
            "docs/index.md",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, path)| Change {
            path: path.into(),
            previous_path: None,
            status: if index == 1 { 'D' } else { 'M' },
            area: Area::Staged,
        })
        .collect()
    }

    fn stats() -> Vec<Option<DiffStats>> {
        (0..changes().len())
            .map(|index| {
                (index != 5).then_some(DiffStats {
                    additions: index + 1,
                    deletions: index,
                })
            })
            .collect()
    }

    #[test]
    fn move_sources_are_relative_to_the_destination_directory() {
        for (previous, current, expected) in [
            (
                ".codex/project-state.md",
                ".agents/project-state.md",
                "../.codex/",
            ),
            ("src/old.rs", "src/new.rs", "old.rs"),
            ("src/old.rs", "lib/new.rs", "../src/old.rs"),
            ("README.md", "docs/README.md", "../"),
            ("docs/README.md", "README.md", "docs/"),
            ("old.md", "new.md", "old.md"),
            ("old.txt", "old.txt/new.txt", "../old.txt"),
            ("资料/旧 名.txt", "文档/新 名.txt", "../资料/旧 名.txt"),
        ] {
            let mut change = Change {
                path: current.into(),
                previous_path: Some(previous.into()),
                status: 'R',
                area: Area::Staged,
            };
            assert_eq!(move_source(&change).as_deref(), Some(expected));
            change.status = 'C';
            assert!(move_source(&change).is_none());
        }
    }

    #[test]
    fn directories_are_compacted_counted_and_sorted_without_losing_file_indices() {
        let changes = changes();
        let mut nodes = HashMap::new();
        let root = Directory::from_changes(&changes).into_item(
            "repository".into(),
            Path::new(""),
            true,
            &changes,
            &stats(),
            &mut nodes,
        );
        assert_eq!(root.label.as_str(), "repository");
        assert_eq!(nodes[&root.id].file_count, 6);
        assert_eq!(
            nodes[&root.id].stats,
            Some(DiffStats {
                additions: 15,
                deletions: 10
            })
        );
        assert_eq!(
            root.children
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            [".codex", "crates/model/src/specs", "docs", "README.md"]
        );
        let compact = &root.children[1];
        assert_eq!(nodes[&compact.id].file_count, 2);
        assert_eq!(
            nodes[&compact.id].stats,
            Some(DiffStats {
                additions: 5,
                deletions: 3
            })
        );
        assert_eq!(nodes[&compact.children[0].id].file_index, Some(1));
        assert_eq!(nodes[&compact.children[0].id].status, 'D');
        assert_eq!(root.children[2].children[0].label.as_str(), "guides");
        assert_eq!(root.children[2].children[1].label.as_str(), "index.md");
        for (index, change) in changes.iter().enumerate() {
            assert_eq!(
                nodes[&file_id(index)].path.as_str(),
                change.path.to_str().unwrap()
            );
        }
    }

    #[test]
    fn same_path_in_index_and_worktree_keeps_separate_stats_and_selection_ids() {
        let changes = [Area::Staged, Area::Unstaged].map(|area| Change {
            path: "src/shared.rs".into(),
            previous_path: None,
            status: 'M',
            area,
        });
        let stats = [
            Some(DiffStats {
                additions: 3,
                deletions: 1,
            }),
            Some(DiffStats {
                additions: 7,
                deletions: 2,
            }),
        ];
        let mut nodes = HashMap::new();
        let root = Directory::from_changes(&changes).into_item(
            "repository".into(),
            Path::new(""),
            true,
            &changes,
            &stats,
            &mut nodes,
        );
        assert_eq!(root.children[0].children.len(), 2);
        assert_eq!(
            nodes[&root.id].stats,
            Some(DiffStats {
                additions: 10,
                deletions: 3
            })
        );
        for index in 0..2 {
            let node = &nodes[&file_id(index)];
            assert_eq!(node.file_index, Some(index));
            assert_eq!(node.area, Some(changes[index].area));
            assert_eq!(node.stats, stats[index]);
        }
    }

    struct Viewer {
        tree: ChangeTree,
        show_stats: bool,
        opened: Rc<Cell<Option<usize>>>,
    }
    impl Render for Viewer {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.tree.render(
                Some("main".into()),
                self.show_stats,
                false,
                |_, _, _| {},
                None,
                cx,
            )
        }
    }

    #[gpui_kit::test]
    fn arrow_keys_walk_parents_children_and_following_entries(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (handle, viewer) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Viewer {
                    tree: ChangeTree::new(
                        &changes(),
                        &stats(),
                        Path::new("repository"),
                        false,
                        cx,
                        |viewer: &mut Viewer, index, _| viewer.opened.set(Some(index)),
                    ),
                    show_stats: false,
                    opened: Rc::new(Cell::new(None)),
                })
            })
            .unwrap()
        });
        let select = |action: Box<dyn Action>, cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.dispatch_action(action, cx);
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        let selected = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                viewer
                    .read(cx)
                    .tree
                    .state
                    .read(cx)
                    .selected_item()
                    .map(|item| (item.label.to_string(), item.is_expanded()))
            })
        };
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.click(file_id(4), cx);
        })
        .unwrap();
        select(Box::new(SelectLeft), cx);
        assert_eq!(selected(cx), Some(("guides".into(), true)));
        select(Box::new(SelectLeft), cx);
        assert_eq!(selected(cx), Some(("guides".into(), false)));
        select(Box::new(SelectLeft), cx);
        assert_eq!(selected(cx), Some(("docs".into(), true)));
        select(Box::new(SelectRight), cx);
        assert_eq!(selected(cx), Some(("guides".into(), false)));
        select(Box::new(SelectRight), cx);
        assert_eq!(selected(cx), Some(("guides".into(), true)));
        select(Box::new(SelectRight), cx);
        assert_eq!(selected(cx).unwrap().0, "setup.md");
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(4));
        select(Box::new(SelectRight), cx);
        assert_eq!(selected(cx).unwrap().0, "index.md");
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(5));
        select(Box::new(SelectRight), cx);
        assert_eq!(selected(cx).unwrap().0, "README.md");
        select(Box::new(SelectRight), cx);
        assert_eq!(selected(cx).unwrap().0, "README.md");
        select(Box::new(gpui_kit::base::actions::SelectUp), cx);
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(5));
        select(Box::new(gpui_kit::base::actions::SelectUp), cx);
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(4));
        select(Box::new(gpui_kit::base::actions::SelectDown), cx);
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(5));
    }

    #[gpui_kit::test]
    fn move_annotation_remains_visible_without_line_stats(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut changes = changes();
        changes[3].path = ".agents/project-state.md".into();
        changes[3].previous_path = Some(".codex/project-state.md".into());
        changes[3].status = 'R';
        let mut stats = stats();
        stats[3] = Some(DiffStats::default());
        let (handle, viewer) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Viewer {
                    tree: ChangeTree::new(
                        &changes,
                        &stats,
                        Path::new("repository"),
                        false,
                        cx,
                        |viewer: &mut Viewer, index, _| viewer.opened.set(Some(index)),
                    ),
                    show_stats: false,
                    opened: Rc::new(Cell::new(None)),
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(window.find_all("tree-move-source").len(), 1);
            let label = tr("moved_from").replace("{path}", "../.codex/");
            assert_eq!(
                window.within(file_id(3)).find("tree-move-source").label(),
                Some(label.as_str()),
            );
            assert!(window.find_all("file-diff-stats").is_empty());
            window.click(file_id(3), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(3));
    }

    #[gpui_kit::test]
    fn folder_clicks_fold_and_leaf_clicks_open_the_original_file(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (handle, viewer) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Viewer {
                    tree: ChangeTree::new(
                        &changes(),
                        &stats(),
                        Path::new("repository"),
                        false,
                        cx,
                        |viewer: &mut Viewer, index, _| viewer.opened.set(Some(index)),
                    ),
                    show_stats: false,
                    opened: Rc::new(Cell::new(None)),
                })
            })
            .unwrap()
        });
        let folder = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                let folder = viewer
                    .read(cx)
                    .tree
                    .nodes
                    .iter()
                    .find(|(_, node)| {
                        node.file_index.is_none()
                            && Path::new(node.path.as_str()) == Path::new("crates/model/src/specs")
                    })
                    .unwrap()
                    .0
                    .clone();
                assert!(window.try_find(file_id(1)).is_some());
                window.click(folder.clone(), cx);
                assert!(window.try_find(file_id(1)).is_none());
                assert_eq!(viewer.read(cx).opened.get(), None);
                window.click(folder.clone(), cx);
                window.click(file_id(2), cx);
                folder
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(2));
        cx.update_window(handle, |_, window, cx| {
            window.click(folder, cx);
            viewer.update(cx, |viewer, cx| viewer.tree.reveal_file(1, cx));
            window.render_frame(cx);
            assert!(window.try_find(file_id(1)).is_some());
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(1));
    }

    #[gpui_kit::test]
    fn directory_toggle_and_stats_follow_visible_tree_state(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (handle, viewer) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| Viewer {
                    tree: ChangeTree::new(
                        &changes(),
                        &stats(),
                        Path::new("repository"),
                        false,
                        cx,
                        |viewer: &mut Viewer, index, _| viewer.opened.set(Some(index)),
                    ),
                    show_stats: false,
                    opened: Rc::new(Cell::new(None)),
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find_all("file-diff-stats").is_empty());
            viewer.update(cx, |viewer, cx| {
                viewer.show_stats = true;
                cx.notify();
            });
            window.render_frame(cx);
            assert_eq!(window.find_all("file-diff-stats").len(), changes().len());
            assert_eq!(
                window.within(file_id(1)).find("file-diff-stats").label(),
                Some("+2 −1")
            );
            assert_eq!(
                window.within(file_id(5)).find("file-diff-stats").label(),
                Some(tr("binary_file"))
            );
            let folder = viewer
                .read(cx)
                .tree
                .nodes
                .iter()
                .find(|(_, node)| {
                    Path::new(node.path.as_str()) == Path::new("crates/model/src/specs")
                })
                .unwrap()
                .0
                .clone();
            window.click(folder.clone(), cx);
            window.render_frame(cx);
            assert_eq!(
                window.within(folder).find("file-diff-stats").label(),
                Some("+5 −3")
            );
            assert!(viewer.read(cx).tree.has_collapsed_directories(cx));

            viewer.update(cx, |viewer, cx| viewer.tree.toggle_directories(cx));
            window.render_frame(cx);
            assert!(!viewer.read(cx).tree.has_collapsed_directories(cx));
            assert!(window.try_find(file_id(1)).is_some());
            viewer.update(cx, |viewer, cx| viewer.tree.toggle_directories(cx));
            window.render_frame(cx);
            assert!(viewer.read(cx).tree.has_collapsed_directories(cx));
            assert!(window.try_find(file_id(0)).is_none());
            assert_eq!(window.find_all("file-diff-stats").len(), 1);
            assert_eq!(
                window.within("tree-root").find("file-diff-stats").label(),
                Some("+15 −10")
            );

            // Opening only the root after collapse-all leaves its children folded.
            window.click("tree-root", cx);
            window.render_frame(cx);
            assert!(window.try_find(file_id(0)).is_some());
            assert!(window.try_find(file_id(1)).is_none());
            assert!(viewer.read(cx).tree.has_collapsed_directories(cx));
            viewer.update(cx, |viewer, cx| {
                viewer.show_stats = false;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find_all("file-diff-stats").is_empty());
            viewer.update(cx, |viewer, cx| viewer.tree.toggle_directories(cx));
            window.render_frame(cx);
            window.click(file_id(2), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| viewer.read(cx).opened.get()), Some(2));
    }
}
