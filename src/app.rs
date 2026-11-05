use crate::{
    change_tree::{ChangeTree, file_status_color, stats_label},
    file_icons::file_icon,
    image_diff::{ImageDiff, ImageVersions, image_format},
    strings::tr,
    ui::{DisplaySettings, apply_appearance, apply_language, icon_button, toolbar_divider},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::{SelectableText, TestSupportExt};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, FocusableExt, Icon, Selectable, Sizable, Theme, TitleBar,
        WindowExt,
        button::{Button, ButtonCustomVariant, ButtonVariant, ButtonVariants},
        diff::{
            Diff, DiffChangeIndicator, DiffFile, DiffHunkSeparator, DiffLinePosition, DiffMode,
            DiffSide, DiffState,
        },
        h_flex,
        input::{Input, InputEvent as TextInputEvent, InputState},
        menu::{DropdownMenu, PopupMenuItem},
        resizable::{h_resizable, resizable_panel, v_resizable},
        scroll::{Scrollbar, ScrollbarHandle, ScrollbarMode},
        tooltip::Tooltip,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};
use patchlook::{
    external_open::{ApplicationIcon, FileToOpen, OpenWith, applications_for},
    git::{
        AccessMode, Area, Branch, Change, Commit, DiffStats, Repository, RepositoryLocation,
        ReviewPatch, SshConnectionControl, StageSelection, WriteHistory, WriteScope,
        merge_working_changes,
    },
    preferences::{Appearance, Language, Layout, Preferences},
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const FILE_SELECTION_DELAY: Duration = Duration::from_millis(120);
const COPY_FEEDBACK_DURATION: Duration = Duration::from_secs(2);
const HISTORY_SEARCH_THRESHOLD: usize = 20;
const STAGED_CHANGE_BACKGROUND_OPACITY: f32 = 0.30;
const REPOSITORY_TREE_INDENT: f32 = 20.;
const REPOSITORY_DISCLOSURE_WIDTH: f32 = 24.;
const SUBMODULE_ROW_HEIGHT: f32 = 30.;

gpui_kit::actions!(patchlook, [About, OpenRepository, Refresh, Quit]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Repositories,
    Working,
    History,
}

#[derive(Clone, Copy)]
enum ChangeOperation {
    Stage(StageSelection),
    DiscardBlock(usize),
}

enum FileContent {
    Text(Vec<DiffFile>, Option<Arc<ReviewPatch>>),
    Image(ImageVersions),
}

impl FileContent {
    fn load(repo: &Repository, change: &Change, commit: Option<&Commit>) -> anyhow::Result<Self> {
        if image_format(&change.path).is_some() {
            Ok(Self::Image(ImageVersions::decode(
                change,
                repo.file_versions(change, commit)?,
            )?))
        } else {
            if commit.is_none() {
                Self::from_review(repo.review_patch(change)?)
            } else {
                Ok(Self::Text(
                    DiffFile::parse(&repo.patch(change, commit)?)?,
                    None,
                ))
            }
        }
    }

    fn from_review(review: ReviewPatch) -> anyhow::Result<Self> {
        let files = review
            .patches()
            .map(DiffFile::parse)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(Self::Text(files, Some(Arc::new(review))))
    }
}

/// Lists and their initial preview are prepared together before replacing visible content.
struct PreparedChanges {
    changes: Vec<Change>,
    stats: Vec<Option<DiffStats>>,
    progress: Vec<Option<(usize, usize)>>,
    preview: Option<anyhow::Result<FileContent>>,
    preview_index: usize,
}

impl PreparedChanges {
    /// A write refreshes its paths and the visible preview. Unrelated rows keep
    /// their existing counts until explicit refresh, just as before the write.
    fn after_write(repo: &Repository, previous: Self, touched: &[PathBuf]) -> anyhow::Result<Self> {
        let raw = repo.working_changes()?;
        let changes = merge_working_changes(&raw);
        let preview_index = previous
            .changes
            .get(previous.preview_index)
            .and_then(|selected| changes.iter().position(|c| c.path == selected.path))
            .unwrap_or(0);
        let old_indices = changes
            .iter()
            .map(|c| previous.changes.iter().position(|old| old == c))
            .collect::<Vec<_>>();
        let reload = changes
            .iter()
            .enumerate()
            .map(|(i, c)| {
                i == preview_index
                    || old_indices[i].is_none()
                    || touched.contains(&c.path)
                    || c.previous_path
                        .as_ref()
                        .is_some_and(|p| touched.contains(p))
            })
            .collect::<Vec<_>>();
        let refreshed_preview = reload
            .iter()
            .take(preview_index)
            .filter(|reload| **reload)
            .count();
        let updated = Self::load(
            repo,
            raw.into_iter()
                .filter(|entry| {
                    changes
                        .iter()
                        .enumerate()
                        .any(|(i, c)| reload[i] && c.path == entry.path)
                })
                .collect(),
            None,
            Some(refreshed_preview),
        )?;
        let mut stats = Vec::with_capacity(changes.len());
        let mut progress = Vec::with_capacity(changes.len());
        for (i, change) in changes.iter().enumerate() {
            let (source, index) = if reload[i] {
                (
                    &updated,
                    updated
                        .changes
                        .iter()
                        .position(|c| c.path == change.path)
                        .unwrap(),
                )
            } else {
                (&previous, old_indices[i].unwrap())
            };
            stats.push(source.stats[index]);
            progress.push(source.progress[index]);
        }
        Ok(Self {
            changes,
            stats,
            progress,
            preview: updated.preview,
            preview_index,
        })
    }

    fn load(
        repo: &Repository,
        changes: Vec<Change>,
        commit: Option<&Commit>,
        preview_index: Option<usize>,
    ) -> anyhow::Result<Self> {
        if commit.is_some() {
            let stats = repo.file_stats(&changes, commit)?;
            let preview = preview_index
                .and_then(|index| changes.get(index))
                .map(|change| FileContent::load(repo, change, commit));
            return Ok(Self {
                progress: vec![None; changes.len()],
                changes,
                stats,
                preview,
                preview_index: preview_index.unwrap_or(0),
            });
        }
        let merged = merge_working_changes(&changes);
        let preview = preview_index
            .and_then(|index| merged.get(index))
            .map(|change| {
                if image_format(&change.path).is_some() {
                    FileContent::load(repo, change, None)
                } else {
                    repo.review_patch_from_changes(change, &changes)
                        .and_then(FileContent::from_review)
                }
            });
        let mut counts = HashMap::new();
        if let Some(Ok(FileContent::Text(_, Some(review)))) = &preview
            && let Some(stats) = review.text_stats()
        {
            counts.insert(merged[preview_index.unwrap()].path.clone(), Some(stats));
        }
        let progress = merged
            .iter()
            .enumerate()
            .map(|(index, change)| {
                if change.area == Area::Partial {
                    if preview_index == Some(index)
                        && let Some(Ok(FileContent::Text(_, Some(review)))) = &preview
                    {
                        return Ok(Some(review.progress()));
                    }
                    let review = repo.review_patch_from_changes(change, &changes)?;
                    if let Some(stats) = review.text_stats() {
                        counts.insert(change.path.clone(), Some(stats));
                    }
                    Ok(Some(review.progress()))
                } else {
                    Ok(None)
                }
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        // Preview/progress already read these text diffs. Only ask Git for line
        // counts of the remaining paths, preserving binary and metadata handling.
        let remaining = changes
            .into_iter()
            .filter(|change| !counts.contains_key(&change.path))
            .collect::<Vec<_>>();
        for (change, stats) in remaining.iter().zip(repo.file_stats(&remaining, None)?) {
            let total = counts
                .entry(change.path.clone())
                .or_insert(Some(DiffStats::default()));
            *total = total.zip(stats).map(|(mut total, stats)| {
                total += stats;
                total
            });
        }
        let stats = merged.iter().map(|change| counts[&change.path]).collect();
        Ok(Self {
            changes: merged,
            stats,
            progress,
            preview,
            preview_index: preview_index.unwrap_or(0),
        })
    }
}

/// Working changes and commit changes have independent lifetimes, selection and previews.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Working,
    History,
}

impl View {
    fn source(self) -> Source {
        match self {
            Self::Working => Source::Working,
            Self::Repositories | Self::History => Source::History,
        }
    }
}

struct ChangeState {
    loaded: bool,
    changes: Vec<Change>,
    file_stats: Vec<Option<DiffStats>>,
    progress: Vec<Option<(usize, usize)>>,
    change_tree: Option<ChangeTree>,
    total_stats: Option<DiffStats>,
    file_scroll: UniformListScrollHandle,
    selected_file: Option<usize>,
    diff: Entity<DiffState>,
    staging_patch: Option<Arc<ReviewPatch>>,
    image: Option<Entity<ImageDiff>>,
    image_subscription: Option<Subscription>,
    _diff_subscription: Subscription,
    files_loading: bool,
    patch_loading: bool,
    empty_patch: bool,
    error: Option<String>,
    load_task: Option<Task<()>>,
    patch_task: Option<Task<()>>,
    last_file_selection: Option<Instant>,
}

impl ChangeState {
    fn new(layout: Layout, cx: &mut Context<Patchlook>) -> Self {
        let diff = cx.new(|cx| DiffState::new([], cx).with_mode(diff_mode(layout)));
        let subscription = cx.observe(&diff, |_, _, cx| cx.notify());
        Self {
            loaded: false,
            changes: Vec::new(),
            file_stats: Vec::new(),
            progress: Vec::new(),
            change_tree: None,
            total_stats: None,
            file_scroll: UniformListScrollHandle::new(),
            selected_file: None,
            diff,
            staging_patch: None,
            image: None,
            image_subscription: None,
            _diff_subscription: subscription,
            files_loading: false,
            patch_loading: false,
            empty_patch: false,
            error: None,
            load_task: None,
            patch_task: None,
            last_file_selection: None,
        }
    }

    fn cancel_pending(&mut self) {
        self.load_task = None;
        self.patch_task = None;
        self.last_file_selection = None;
        self.files_loading = false;
        if self.patch_loading {
            // A cancelled selection has not produced its preview yet.
            self.selected_file = None;
        }
        self.patch_loading = false;
    }
}

struct HistoryState {
    files: ChangeState,
    commits: Vec<Commit>,
    selected_commit: Option<usize>,
    has_more: bool,
    revision: Option<String>,
    query: String,
    search_available: bool,
    scroll: UniformListScrollHandle,
    loading: bool,
    load_task: Option<Task<()>>,
}

impl HistoryState {
    fn new(layout: Layout, cx: &mut Context<Patchlook>) -> Self {
        Self {
            files: ChangeState::new(layout, cx),
            commits: Vec::new(),
            selected_commit: None,
            has_more: false,
            revision: None,
            query: String::new(),
            search_available: false,
            scroll: UniformListScrollHandle::new(),
            loading: false,
            load_task: None,
        }
    }
}

enum HistoryRequest {
    Branch(Option<String>),
    Search,
    More,
}

pub struct Patchlook {
    access_mode: AccessMode,
    write_task: Option<Task<()>>,
    write_history: WriteHistory,
    preferences: Preferences,
    about: Entity<crate::about::AboutPanel>,
    external_files: Vec<tempfile::TempDir>,
    #[cfg(test)]
    preferences_directory: tempfile::TempDir,
    repository: Option<Repository>,
    // Keep authenticated sessions alive independently of the visible repository snapshot.
    remote_sessions: HashMap<RepositoryLocation, Repository>,
    selected_repository: Option<RepositoryLocation>,
    pending_connection: Option<(RepositoryLocation, SshConnectionControl)>,
    remote_errors: HashMap<RepositoryLocation, String>,
    branch: String,
    branches: Vec<Branch>,
    repository_scroll: ScrollHandle,
    collapsed_repository_groups: [bool; 2],
    collapsed_repositories: HashSet<RepositoryLocation>,
    branch_scroll: UniformListScrollHandle,
    view: View,
    working: ChangeState,
    history: HistoryState,
    repository_preview: bool,
    history_search: Entity<InputState>,
    commit_copied: bool,
    copy_feedback_task: Option<Task<()>>,
    loading: bool,
    error: Option<String>,
    load_task: Option<Task<()>>,
    host_verification: crate::ssh_host::HostVerification,
    _subscriptions: Vec<Subscription>,
    focus: FocusHandle,
}

impl Patchlook {
    pub fn new(
        initial: Option<RepositoryLocation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (preferences, error) = match Preferences::read() {
            Ok(preferences) => (preferences, None),
            Err(error) => (Preferences::default(), Some(format!("{error:#}"))),
        };
        Self::from_preferences(initial, preferences, error, window, cx)
    }

    fn from_preferences(
        initial: Option<RepositoryLocation>,
        mut preferences: Preferences,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        DisplaySettings::initialize(&preferences, cx);
        DisplaySettings::get(cx).copy_to(&mut preferences);
        apply_appearance(preferences.appearance, window, cx);
        apply_language(preferences.language, cx);
        let history_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("search_commits_placeholder")));
        let subscriptions = vec![
            cx.observe_global_in::<DisplaySettings>(window, |this, window, cx| {
                DisplaySettings::get(cx).copy_to(&mut this.preferences);
                this.update_translations(window, cx);
                cx.notify();
            }),
            cx.subscribe(&history_search, |this, input, event, cx| {
                if matches!(event, TextInputEvent::PressEnter { .. })
                    || (matches!(event, TextInputEvent::Change)
                        && input.read(cx).value().is_empty()
                        && !this.history.query.is_empty())
                {
                    this.submit_history_search(cx);
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.preferences.appearance == Appearance::System {
                    Theme::sync_system_appearance(Some(window), cx);
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() && this.preferences.language == Language::System {
                    apply_language(Language::System, cx);
                    if let Some(tree) = &this.working.change_tree {
                        tree.update_group_labels(cx);
                    }
                    this.history_search.update(cx, |input, cx| {
                        input.set_placeholder(tr("search_commits_placeholder"), window, cx);
                    });
                }
            }),
        ];
        let saved_remote = preferences
            .repository
            .as_ref()
            .filter(|path| initial.is_none() && path.is_remote())
            .cloned();
        let initial = initial.or_else(|| {
            preferences
                .repository
                .clone()
                .filter(|path| !path.is_remote())
        });
        let view = if saved_remote.is_some() {
            View::Repositories
        } else {
            View::Working
        };
        let app = Self {
            access_mode: AccessMode::default(),
            write_task: None,
            write_history: WriteHistory::default(),
            working: ChangeState::new(preferences.layout, cx),
            history: HistoryState::new(preferences.layout, cx),
            preferences,
            about: cx.new(crate::about::AboutPanel::new),
            host_verification: crate::ssh_host::HostVerification::new(window, cx),
            external_files: Vec::new(),
            #[cfg(test)]
            preferences_directory: tempfile::tempdir().unwrap(),
            repository: None,
            remote_sessions: HashMap::new(),
            selected_repository: saved_remote,
            pending_connection: None,
            remote_errors: HashMap::new(),
            branch: String::new(),
            branches: vec![],
            repository_scroll: ScrollHandle::new(),
            collapsed_repository_groups: [false; 2],
            collapsed_repositories: HashSet::new(),
            branch_scroll: UniformListScrollHandle::new(),
            view,
            repository_preview: false,

            history_search,

            commit_copied: false,
            copy_feedback_task: None,
            loading: false,

            error,
            load_task: None,
            _subscriptions: subscriptions,
            focus: cx.focus_handle(),
        };
        window.focus(&app.focus, cx);
        if let Some(path) = initial {
            // The Kit root (and its dialog layer) is installed after this constructor returns.
            cx.defer_in(window, move |this, window, cx| {
                this.open_saved_repository(path, window, cx);
            });
        }
        app
    }

    fn show_about(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        self.about.update(cx, |about, cx| about.show(window, cx));
    }

    fn save_preferences(&mut self) {
        #[cfg(not(test))]
        let result = self.preferences.write();
        #[cfg(test)]
        let result = self
            .preferences
            .write_to(&self.preferences_directory.path().join("preferences.json"));
        if let Err(error) = result {
            self.error = Some(format!("{}: {error:#}", tr("settings_failed")));
        }
    }

    fn set_access_mode(&mut self, mode: AccessMode, cx: &mut Context<Self>) {
        if self.write_task.is_none() && !self.repository.as_ref().is_some_and(Repository::is_remote)
        {
            self.access_mode = mode;
            cx.notify();
        }
    }

    fn staging_visible(&self) -> bool {
        self.access_mode == AccessMode::ReadWrite
            && self.view == View::Working
            && !self.repository.as_ref().is_some_and(Repository::is_remote)
    }

    fn staging_disabled(&self) -> bool {
        self.loading
            || self.working.files_loading
            || self.working.patch_loading
            || self.write_task.is_some()
    }

    fn write_change(
        &mut self,
        change: Change,
        operation: ChangeOperation,
        patch: Option<Arc<ReviewPatch>>,
        cx: &mut Context<Self>,
    ) {
        if change.area == Area::Conflict {
            return;
        }
        let side = if matches!(operation, ChangeOperation::DiscardBlock(_)) {
            DiffSide::Original
        } else {
            self.staging_anchor_side()
        };
        let scope = if matches!(operation, ChangeOperation::DiscardBlock(_)) {
            WriteScope::Worktree(change.path.clone())
        } else {
            WriteScope::index(std::slice::from_ref(&change))
        };
        self.write_repository(
            side,
            move |repo, mode, history| {
                history.record(repo, mode, scope, || match operation {
                    ChangeOperation::Stage(selection) => match selection {
                        StageSelection::File => repo.set_staged(mode, &change, selection, None),
                        StageSelection::Block(block) => repo.set_review_block(
                            mode,
                            patch
                                .as_deref()
                                .ok_or_else(|| anyhow::anyhow!("No text patch available"))?,
                            block,
                        ),
                    },
                    ChangeOperation::DiscardBlock(block) => repo.discard_review_block(
                        mode,
                        patch
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("No text patch available"))?,
                        block,
                    ),
                })
            },
            cx,
        );
    }

    fn staging_anchor_side(&self) -> DiffSide {
        DiffSide::Modified
    }

    fn write_repository(
        &mut self,
        stable_side: DiffSide,
        write: impl FnOnce(&Repository, AccessMode, &mut WriteHistory) -> anyhow::Result<Vec<PathBuf>>
        + Send
        + 'static,
        cx: &mut Context<Self>,
    ) {
        if !self.staging_visible() || self.staging_disabled() {
            return;
        }
        let Some(repo) = self.repository.clone() else {
            return;
        };
        let mode = self.access_mode;
        let mut history = self.write_history.clone();
        let previous = PreparedChanges {
            changes: self.working.changes.clone(),
            stats: self.working.file_stats.clone(),
            progress: self.working.progress.clone(),
            preview: None,
            preview_index: self.working.selected_file.unwrap_or(0),
        };
        self.working.cancel_pending();
        self.working.files_loading = true;
        self.working.error = None;
        self.write_task = Some(cx.spawn(async move |this, cx| {
            let (history, result) = cx
                .background_spawn(async move {
                    let result: anyhow::Result<anyhow::Result<PreparedChanges>> =
                        write(&repo, mode, &mut history)
                            .map(|paths| PreparedChanges::after_write(&repo, previous, &paths));
                    (history, result)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.write_task = None;
                this.write_history = history;
                this.working.files_loading = false;
                match result {
                    Ok(Ok(prepared)) => {
                        let previous_tree = this.working.change_tree.take();
                        let scroll = this.working.file_scroll.clone();
                        this.install_changes(Source::Working, prepared, Some(stable_side), cx);
                        this.working.file_scroll = scroll;
                        if let (Some(tree), Some(previous)) =
                            (&this.working.change_tree, previous_tree)
                        {
                            tree.restore_expansion(&previous, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        this.working.error =
                            Some(format!("{}: {error:#}", tr("write_refresh_failed")))
                    }
                    Err(error) => {
                        this.working.error = Some(format!("{}: {error:#}", tr("write_failed")))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn restore_write(&mut self, undo: bool, cx: &mut Context<Self>) {
        self.write_repository(
            DiffSide::Original,
            move |repo, mode, history| {
                if undo {
                    history.undo(repo, mode)
                } else {
                    history.redo(repo, mode)
                }
            },
            cx,
        );
    }

    fn staging_button(
        &self,
        id: impl Into<ElementId>,
        change: Change,
        selection: StageSelection,
        cx: &mut Context<Self>,
    ) -> Button {
        let checked = match selection {
            StageSelection::File => change.area == Area::Staged,
            StageSelection::Block(block) => self
                .working
                .staging_patch
                .as_ref()
                .is_some_and(|p| p.is_staged(block)),
        };
        let partial = selection == StageSelection::File && change.area == Area::Partial;
        let label = tr(match (checked, selection) {
            (false, StageSelection::File) => "stage_file",
            (true, StageSelection::File) => "unstage_file",
            (false, StageSelection::Block(_)) => "stage_block",
            (true, StageSelection::Block(_)) => "unstage_block",
        });
        let patch = self.working.staging_patch.clone();
        Button::new(id)
            .icon(if checked {
                IconName::SquareCheck
            } else if partial {
                IconName::SquareMinus
            } else {
                IconName::Square
            })
            .xsmall()
            .ghost()
            .toggled(checked)
            .text_color(cx.theme().muted_foreground)
            .disabled(self.staging_disabled() || change.area == Area::Conflict)
            .tooltip(label)
            .accessibility_label(format!("{label}: {}", change.path.display()))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.write_change(
                    change.clone(),
                    ChangeOperation::Stage(selection),
                    patch.clone(),
                    cx,
                );
            }))
    }

    fn staging_directory_button(
        &self,
        id: &str,
        directory: &str,
        indices: &[usize],
        cx: &mut Context<Self>,
    ) -> Button {
        let changes = indices
            .iter()
            .map(|&i| self.working.changes[i].clone())
            .collect::<Vec<_>>();
        let (count, staged, conflict) =
            changes
                .iter()
                .fold((0, 0, false), |(count, staged, conflict), change| {
                    (
                        count + 1,
                        staged + usize::from(matches!(change.area, Area::Staged | Area::Partial)),
                        conflict || change.area == Area::Conflict,
                    )
                });
        let all_staged = count > 0 && changes.iter().all(|c| c.area == Area::Staged);
        let label = tr(if all_staged {
            "unstage_directory"
        } else if staged > 0 {
            "stage_directory_remaining"
        } else {
            "stage_directory"
        });
        let directory = directory.to_owned();
        Button::new(format!("stage-folder:{id}"))
            .icon(if all_staged {
                IconName::SquareCheck
            } else if staged > 0 {
                IconName::SquareMinus
            } else {
                IconName::Square
            })
            .xsmall()
            .ghost()
            .toggled(all_staged)
            .text_color(cx.theme().muted_foreground)
            .disabled(self.staging_disabled() || count == 0 || conflict)
            .tooltip(label)
            .accessibility_label(format!(
                "{label}: {}",
                if directory.is_empty() {
                    "."
                } else {
                    &directory
                }
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                let changes = changes.clone();
                this.write_repository(
                    this.staging_anchor_side(),
                    move |repo, mode, history| {
                        history.record(repo, mode, WriteScope::index(&changes), || {
                            repo.set_files_staged(mode, &changes, !all_staged)
                        })
                    },
                    cx,
                );
            }))
    }

    fn staging_gutter_actions(
        &self,
        position: &DiffLinePosition,
        side: DiffSide,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let part = self
            .working
            .diff
            .read(cx)
            .files()
            .iter()
            .position(|file| file.path() == position.path())
            .unwrap_or(0);
        let block = self.working.staging_patch.as_ref().and_then(|patch| {
            patch.block_target(
                part,
                position.side() == DiffSide::Original,
                position.line(),
                side == DiffSide::Original,
            )
        });
        let Some(block) = block else {
            return div().into_any_element();
        };
        let Some(change) = self
            .working
            .selected_file
            .and_then(|i| self.working.changes.get(i))
        else {
            return div().into_any_element();
        };
        let (hover_opacity, active_opacity) = if cx.theme().mode.is_dark() {
            (0.12, 0.18)
        } else {
            (0.06, 0.10)
        };
        let action_style = ButtonCustomVariant::new(cx)
            .hover(cx.theme().foreground.opacity(hover_opacity))
            .active(cx.theme().foreground.opacity(active_opacity));
        if side == DiffSide::Modified {
            return self
                .staging_button(
                    ("stage-block", block),
                    change.clone(),
                    StageSelection::Block(block),
                    cx,
                )
                .custom(action_style)
                .into_any_element();
        }
        if self
            .working
            .staging_patch
            .as_ref()
            .is_some_and(|patch| patch.is_staged(block))
        {
            return div().into_any_element();
        }
        let change = change.clone();
        Button::new(("discard-block", block))
            .icon(IconName::ChevronsRight)
            .xsmall()
            .custom(action_style)
            .text_color(cx.theme().muted_foreground)
            .disabled(self.staging_disabled())
            .tooltip(tr("discard_block"))
            .accessibility_label(tr("discard_block"))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.confirm_discard(change.clone(), block, window, cx);
            }))
            .into_any_element()
    }

    fn confirm_discard(
        &self,
        change: Change,
        block: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.staging_visible() || self.staging_disabled() {
            return;
        }
        let Some(patch) = self.working.staging_patch.clone() else {
            return;
        };
        let root = self.repository.as_ref().map(|repo| repo.location.clone());
        let app = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let app = app.clone();
            let change = change.clone();
            let patch = patch.clone();
            let root = root.clone();
            dialog
                .confirm()
                .title(tr("discard_block_title"))
                .description(format!(
                    "{}\n{}",
                    change.path.display(),
                    tr("discard_block_description")
                ))
                .ok_text(tr("discard_block"))
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(tr("keep_changes"))
                .on_ok(move |_, _, cx| {
                    let _ = app.update(cx, |this, cx| {
                        if this.repository.as_ref().map(|repo| &repo.location) == root.as_ref() {
                            this.write_change(
                                change.clone(),
                                ChangeOperation::DiscardBlock(block),
                                Some(patch.clone()),
                                cx,
                            );
                        }
                    });
                    true
                })
        });
    }

    pub fn open_repository(&mut self, _: &OpenRepository, _: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("open_repository").into()),
        });
        cx.spawn(async move |this, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| this.load(path, cx));
                }
            }
            Ok(Err(error)) => {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(error.to_string());
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    fn repository_menu(
        &self,
        id: &'static str,
        icon: IconName,
        compact: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let app = cx.entity().downgrade();
        let local = app.clone();
        let tooltip = self
            .repository
            .as_ref()
            .map(|repo| format!("{}\n{}", tr("open_repository"), repo.location))
            .unwrap_or_else(|| tr("open_repository").to_owned());
        div().occlude().child(
            icon_button(id, icon, "open_repository")
                .when(compact, |b| b.xsmall())
                .text_color(cx.theme().muted_foreground)
                .tooltip(tooltip)
                .disabled(self.write_task.is_some())
                .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                    let local = local.clone();
                    let remote = app.clone();
                    menu.item(
                        PopupMenuItem::new(tr("open_repository"))
                            .icon(IconName::FolderOpen)
                            .on_click(move |_, window, cx| {
                                let _ = local.update(cx, |this, cx| {
                                    this.open_repository(&OpenRepository, window, cx)
                                });
                            }),
                    )
                    .item(
                        PopupMenuItem::new(tr("connect_ssh"))
                            .icon(IconName::Server)
                            .on_click(move |_, window, cx| {
                                let _ = remote.update(cx, |this, cx| {
                                    this.open_remote_repository(window, cx);
                                });
                            }),
                    )
                }),
        )
    }

    fn open_remote_repository(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::remote_repository::RemoteRepositoryForm> {
        self.remote_repository_dialog(None, window, cx)
    }

    fn open_saved_repository(
        &mut self,
        path: RepositoryLocation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.write_task.is_some() {
            return;
        }
        if self
            .preferences
            .submodule(&path)
            .is_some_and(|module| !module.initialized)
        {
            return;
        }
        if self.selected_repository.as_ref() != Some(&path) {
            self.cancel_repository_load();
        }
        self.selected_repository = Some(path.clone());
        cx.notify();
        if self
            .pending_connection
            .as_ref()
            .is_some_and(|(target, _)| *target == path)
        {
            return;
        }
        if self
            .repository
            .as_ref()
            .is_some_and(|repo| repo.location == path)
        {
            return;
        }
        if self.connected_repository(&path).is_none()
            && matches!(&path, RepositoryLocation::Ssh { options, .. } if options.authentication.requires_secret())
        {
            self.remote_repository_dialog(Some(path), window, cx);
        } else {
            self.load(path, cx);
        }
    }

    fn connected_repository(&self, path: &RepositoryLocation) -> Option<Repository> {
        self.remote_sessions
            .get(path)
            .or_else(|| {
                let family = self.preferences.repository_family(path);
                self.remote_sessions
                    .iter()
                    .find(|(other, _)| self.preferences.repository_family(other) == family)
                    .map(|(_, repo)| repo)
            })
            .cloned()
    }

    fn remote_repository_dialog(
        &mut self,
        target: Option<RepositoryLocation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::remote_repository::RemoteRepositoryForm> {
        let history = self.preferences.ssh_input_history.clone();
        let app = cx.entity().downgrade();
        let form = cx.new(|cx| {
            let mut form = crate::remote_repository::RemoteRepositoryForm::new(
                history,
                move |field, value, cx| {
                    let _ = app.update(cx, |this, cx| {
                        this.preferences.ssh_input_history.remove(field, value);
                        this.save_preferences();
                        cx.notify();
                    });
                },
                window,
                cx,
            );
            if let Some(target) = &target {
                form.prefill(target, window, cx);
            }
            form
        });
        let form_view = form.clone();
        let app = cx.entity().downgrade();
        let initial_focus = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                form_view.read(cx).host.focus_handle(cx).focus(window, cx);
            }
            let form = form_view.clone();
            let app = app.clone();
            let can_connect = form_view.read(cx).can_connect(cx);
            dialog
                .title(tr("connect_ssh"))
                .w(px(500.))
                .child(form_view.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .child(Button::new("ssh-cancel").label(tr("cancel")).outline()),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("ssh-connect")
                                    .label(tr("connect"))
                                    .primary()
                                    .disabled(!can_connect),
                            ),
                        ),
                )
                .on_ok(move |_, _, cx| match form.read(cx).connection(cx) {
                    Ok((target, secret)) => {
                        let _ = app.update(cx, |this, cx| {
                            this.preferences.ssh_input_history.remember(&target);
                            this.save_preferences();
                            this.view = View::Repositories;
                            this.load_repository(target, secret, cx);
                        });
                        true
                    }
                    Err(error) => {
                        form.update(cx, |form, cx| {
                            form.error = Some(error.to_string());
                            cx.notify();
                        });
                        false
                    }
                })
        });
        form
    }

    fn files_for(&self, source: Source) -> &ChangeState {
        match source {
            Source::Working => &self.working,
            Source::History => &self.history.files,
        }
    }

    fn files_for_mut(&mut self, source: Source) -> &mut ChangeState {
        match source {
            Source::Working => &mut self.working,
            Source::History => &mut self.history.files,
        }
    }

    fn active_files(&self) -> &ChangeState {
        self.files_for(self.view.source())
    }

    fn active_files_mut(&mut self) -> &mut ChangeState {
        self.files_for_mut(self.view.source())
    }

    fn current_commit(&self) -> Option<&Commit> {
        (self.view.source() == Source::History)
            .then(|| {
                self.history
                    .selected_commit
                    .and_then(|i| self.history.commits.get(i))
            })
            .flatten()
    }

    fn visible_error(&self) -> Option<&String> {
        self.error.as_ref().or(self.active_files().error.as_ref())
    }

    fn viewed_branch(&self) -> Option<&str> {
        self.repository
            .as_ref()
            .and_then(|repo| self.preferences.viewed_branch(&repo.location))
    }

    fn viewed_branch_name(&self) -> String {
        self.viewed_branch()
            .and_then(|reference| {
                self.branches
                    .iter()
                    .find(|branch| branch.reference == reference)
            })
            .map(|branch| branch.name.clone())
            .unwrap_or_else(|| self.branch.clone())
    }

    fn browse_branch(&mut self, reference: Option<String>, cx: &mut Context<Self>) {
        if self.loading
            || (reference.as_deref() == self.viewed_branch()
                && self.history.files.loaded
                && !self.history.loading)
        {
            return;
        }
        self.load_history_page(HistoryRequest::Branch(reference), cx);
    }

    fn load(&mut self, path: impl Into<RepositoryLocation>, cx: &mut Context<Self>) {
        let path = path.into();
        if self
            .repository
            .as_ref()
            .is_some_and(|repo| repo.location == path)
        {
            self.cancel_repository_load();
            self.selected_repository = Some(path);
            cx.notify();
            return;
        }
        self.load_repository(path, None, cx);
    }

    /// Opening/refreshing a repository replaces its two snapshots. Tab changes never call this.
    fn load_repository(
        &mut self,
        path: RepositoryLocation,
        secret: Option<patchlook::ssh_auth::SshSecret>,
        cx: &mut Context<Self>,
    ) {
        if self.write_task.is_some() {
            return;
        }
        self.cancel_repository_load();
        self.selected_repository = Some(path.clone());
        self.remote_errors.remove(&path);
        if path.is_remote() {
            self.preferences.add_repository(path.clone());
            self.save_preferences();
        }
        self.working.cancel_pending();
        self.history.files.cancel_pending();
        self.history.load_task = None;
        self.history.loading = false;
        self.loading = true;
        self.error = None;
        let source = self.view.source();
        let preview = self.view != View::Repositories;
        let query = self.history.query.clone();
        let reference = self.preferences.viewed_branch(&path).map(str::to_owned);
        let existing = secret
            .is_none()
            .then(|| self.connected_repository(&path))
            .flatten();
        let is_submodule = self.preferences.submodule(&path).is_some();
        let reuses_connection = existing.is_some();
        let control = SshConnectionControl::default();
        if path.is_remote() && existing.is_none() {
            self.collapsed_repository_groups[1] = false;
            self.pending_connection = Some((path.clone(), control.clone()));
        }
        let requested = path.clone();
        let verification = self.host_verification.interaction(control.clone());
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let repo = match existing {
                        Some(repo) if repo.location == path => repo,
                        Some(repo) => repo.open_related(path.clone())?,
                        None => Repository::open_with_control(
                            path.clone(),
                            secret,
                            Some(verification?),
                            control,
                        )?,
                    };
                    if is_submodule {
                        repo.ensure_submodule_directory(&path)?;
                    }
                    let submodules = repo.submodules()?;
                    let branches = repo.branches()?;
                    let (branch, page, changes) = match source {
                        Source::Working => {
                            let snapshot = repo.snapshot()?;
                            (snapshot.branch, None, snapshot.changes)
                        }
                        Source::History => {
                            let branch = repo.branch_name()?;
                            let page = repo.history_filtered(0, reference.as_deref(), &query)?;
                            let changes = page
                                .commits
                                .first()
                                .map(|commit| repo.commit_changes(commit))
                                .transpose()?
                                .unwrap_or_default();
                            (branch, Some(page), changes)
                        }
                    };
                    let commit = page.as_ref().and_then(|page| page.commits.first());
                    let prepared =
                        PreparedChanges::load(&repo, changes, commit, preview.then_some(0))?;
                    anyhow::Ok((repo, submodules, branch, branches, page, prepared))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.pending_connection = None;
                match result {
                    Ok((repo, submodules, branch, branches, page, prepared)) => {
                        if repo.is_remote() {
                            if !reuses_connection
                                && let Some(old) = this.connected_repository(&requested)
                            {
                                this.remote_sessions
                                    .retain(|_, other| !old.shares_connection(other));
                                old.disconnect();
                            }
                            this.remote_sessions
                                .insert(repo.location.clone(), repo.clone());
                        }
                        if this.repository.as_ref().map(|old| &old.location) != Some(&repo.location)
                        {
                            this.write_history = WriteHistory::default();
                        }
                        this.working = ChangeState::new(this.preferences.layout, cx);
                        let query = std::mem::take(&mut this.history.query);
                        this.history = HistoryState::new(this.preferences.layout, cx);
                        this.history.query = query;
                        this.repository_preview = false;
                        this.reset_copy_feedback();
                        let requested_branch = this
                            .preferences
                            .viewed_branch(&requested)
                            .map(str::to_owned);
                        if requested != repo.location {
                            this.preferences
                                .recent_repositories
                                .retain(|entry| entry.path != requested);
                        }
                        this.preferences.remember_repository(repo.location.clone());
                        this.preferences
                            .remember_submodules(&repo.location, &submodules);
                        if requested != repo.location && requested_branch.is_some() {
                            this.preferences
                                .set_viewed_branch(&repo.location, requested_branch);
                        }
                        this.remote_errors.remove(&repo.location);
                        this.selected_repository = Some(repo.location.clone());
                        if repo.is_remote() {
                            this.access_mode = AccessMode::ReadOnly;
                        }
                        this.repository = Some(repo);
                        this.branch = branch;
                        this.branches = branches;
                        this.save_preferences();
                        if let Some(page) = page {
                            this.history.revision = page.revision;
                            this.history.commits = page.commits;
                            this.history.has_more = page.has_more;
                            this.history.search_available =
                                this.history.commits.len() > HISTORY_SEARCH_THRESHOLD;
                            this.history.selected_commit =
                                (!this.history.commits.is_empty()).then_some(0);
                        }
                        this.apply_changes(source, prepared, cx);
                    }
                    Err(error) => {
                        let error = format!("{}: {error:#}", tr("load_failed"));
                        if requested.is_remote() {
                            this.remote_errors.insert(requested, error.clone());
                        }
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(repo) = &self.repository {
            self.load_repository(repo.location.clone(), None, cx);
        }
    }

    fn disconnect_repository(
        &mut self,
        path: &RepositoryLocation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !path.is_remote() || self.write_task.is_some() {
            return;
        }
        if self.loading
            && !self
                .pending_connection
                .as_ref()
                .is_some_and(|(target, _)| target == path)
        {
            return;
        }
        if self
            .pending_connection
            .as_ref()
            .is_some_and(|(target, _)| target == path)
        {
            self.cancel_repository_load();
        }
        let connected = self.connected_repository(path);
        let clears_current = self.repository.as_ref().is_some_and(|repo| {
            &repo.location == path
                || connected
                    .as_ref()
                    .is_some_and(|other| repo.shares_connection(other))
        });
        if let Some(repo) = connected {
            self.remote_sessions
                .retain(|_, other| !repo.shares_connection(other));
            repo.disconnect();
        }
        if clears_current {
            self.clear_repository(window, cx);
            self.preferences.repository = None;
            self.save_preferences();
        }
        self.remote_errors.remove(path);
        self.error = None;
        cx.notify();
    }

    fn reconnect_repository(
        &mut self,
        path: RepositoryLocation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.write_task.is_some() {
            return;
        }
        self.cancel_repository_load();
        self.disconnect_repository(&path, window, cx);
        self.open_saved_repository(path, window, cx);
    }

    fn cancel_repository_load(&mut self) {
        if let Some((_, control)) = self.pending_connection.take() {
            control.disconnect();
        }
        self.load_task = None;
        self.loading = false;
    }

    fn clear_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_repository_load();
        self.repository = None;
        self.write_history = WriteHistory::default();
        self.branch.clear();
        self.branches.clear();
        self.branch_scroll = UniformListScrollHandle::new();
        self.working = ChangeState::new(self.preferences.layout, cx);
        self.history = HistoryState::new(self.preferences.layout, cx);
        self.repository_preview = false;
        self.reset_copy_feedback();
        self.error = None;
        self.history_search
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn remove_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || self.write_task.is_some() {
            return;
        }
        let Some(index) = self.selected_repository.as_ref().and_then(|path| {
            if self.preferences.submodule(path).is_some() {
                return None;
            }
            self.preferences
                .recent_repositories
                .iter()
                .position(|entry| &entry.path == path)
        }) else {
            return;
        };
        let path = self.preferences.recent_repositories[index].path.clone();
        if let Some(repo) = self.connected_repository(&path) {
            self.remote_sessions
                .retain(|_, other| !repo.shares_connection(other));
            repo.disconnect();
        }
        let removed = self.preferences.remove_repository_tree(&path);
        for path in &removed {
            self.remote_errors.remove(path);
            self.collapsed_repositories.remove(path);
        }
        let next = self
            .preferences
            .recent_repositories
            .iter()
            .filter(|entry| entry.submodule.is_none())
            .nth(index)
            .or_else(|| {
                self.preferences
                    .recent_repositories
                    .iter()
                    .rfind(|entry| entry.submodule.is_none())
            })
            .map(|entry| entry.path.clone());

        if self
            .repository
            .as_ref()
            .is_some_and(|repo| removed.contains(&repo.location))
        {
            self.clear_repository(window, cx);
        }
        self.selected_repository = next.clone();
        self.save_preferences();
        if let Some(path) = next {
            self.open_saved_repository(path, window, cx);
        }
        cx.notify();
    }

    fn switch_view(&mut self, view: View, cx: &mut Context<Self>) {
        if self.view == view || self.loading {
            return;
        }
        self.view = view;
        if view == View::Repositories {
            self.repository_preview = false;
        }
        self.ensure_view_loaded(cx);
        cx.notify();
    }

    fn ensure_view_loaded(&mut self, cx: &mut Context<Self>) {
        if self.repository.is_none() {
            return;
        }
        match self.view.source() {
            Source::Working if !self.working.loaded && !self.working.files_loading => {
                self.load_working(cx);
            }
            Source::History if !self.history.files.loaded && !self.history.loading => {
                let reference = self.viewed_branch().map(str::to_owned);
                self.load_history_page(HistoryRequest::Branch(reference), cx);
            }
            Source::History if self.view == View::History && !self.history.files.files_loading => {
                self.ensure_history_preview(cx);
            }
            _ => {}
        }
    }

    fn load_working(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repository.clone() else {
            return;
        };
        self.working.files_loading = true;
        self.working.error = None;
        self.working.load_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let snapshot = repo.snapshot()?;
                    PreparedChanges::load(&repo, snapshot.changes, None, Some(0))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.working.files_loading = false;
                match result {
                    Ok(prepared) => this.apply_changes(Source::Working, prepared, cx),
                    Err(error) => this.working.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
    }

    fn ensure_history_preview(&mut self, cx: &mut Context<Self>) {
        if self.history.files.loaded
            && self.history.files.selected_file.is_none()
            && !self.history.files.changes.is_empty()
        {
            self.select_source_file(Source::History, 0, cx);
        }
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        if !self.loading && !self.history.loading && self.history.has_more {
            self.load_history_page(HistoryRequest::More, cx);
        }
    }

    fn submit_history_search(&mut self, cx: &mut Context<Self>) {
        if self.loading || (self.history.loading && self.history.revision.is_none()) {
            return;
        }
        self.history.query = self.history_search.read(cx).value().trim().to_owned();
        self.load_history_page(HistoryRequest::Search, cx);
    }

    /// A new branch resolves a new tip; searches and pagination stay on the loaded tip.
    fn load_history_page(&mut self, request: HistoryRequest, cx: &mut Context<Self>) {
        let Some(repo) = self.repository.clone() else {
            return;
        };
        let reset = !matches!(request, HistoryRequest::More);
        let branch = match request {
            HistoryRequest::Branch(reference) => Some(reference),
            HistoryRequest::Search | HistoryRequest::More => None,
        };
        self.history.load_task = None;
        if reset {
            self.history.files.cancel_pending();
        }
        self.history.loading = true;
        self.history.files.error = None;
        let offset = if reset { 0 } else { self.history.commits.len() };
        let revision = branch
            .clone()
            .unwrap_or_else(|| self.history.revision.clone());
        let query = self.history.query.clone();
        let preview = self.view == View::History;
        self.history.load_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let page = repo.history_filtered(offset, revision.as_deref(), &query)?;
                    let prepared = if reset {
                        let commit = page.commits.first();
                        let changes = commit
                            .map(|commit| repo.commit_changes(commit))
                            .transpose()?
                            .unwrap_or_default();
                        Some(PreparedChanges::load(
                            &repo,
                            changes,
                            commit,
                            preview.then_some(0),
                        )?)
                    } else {
                        None
                    };
                    anyhow::Ok((page, prepared))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.history.loading = false;
                match result {
                    Ok((page, prepared)) => {
                        if reset {
                            this.reset_copy_feedback();
                            this.history.commits.clear();
                            this.history.scroll = UniformListScrollHandle::new();
                            this.repository_preview = false;
                        }
                        this.history.revision = page.revision;
                        this.history.commits.extend(page.commits);
                        this.history.has_more = page.has_more;
                        if this.history.query.is_empty() {
                            this.history.search_available =
                                this.history.commits.len() > HISTORY_SEARCH_THRESHOLD;
                        }
                        if let Some(prepared) = prepared {
                            this.history.selected_commit =
                                (!this.history.commits.is_empty()).then_some(0);
                            this.apply_changes(Source::History, prepared, cx);
                            if this.view == View::History {
                                this.ensure_history_preview(cx);
                            }
                        }
                        if let Some(reference) = branch {
                            let repo = this.repository.as_ref().unwrap();
                            // Lazy first entry and Tab changes do not rewrite preferences.
                            if this.preferences.viewed_branch(&repo.location)
                                != reference.as_deref()
                            {
                                this.preferences
                                    .set_viewed_branch(&repo.location, reference);
                                this.save_preferences();
                            }
                        }
                    }
                    Err(error) => this.history.files.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn reset_copy_feedback(&mut self) {
        self.copy_feedback_task = None;
        self.commit_copied = false;
    }

    fn select_commit(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.loading || self.history.loading {
            return;
        }
        if self.history.selected_commit == Some(index)
            && self.history.files.loaded
            && !self.history.files.files_loading
            && self.history.files.error.is_none()
        {
            return;
        }
        let Some(repo) = self.repository.clone() else {
            return;
        };
        let commit = self.history.commits[index].clone();
        self.history.files.cancel_pending();
        self.history.files.files_loading = true;
        self.history.files.error = None;
        let preview = self.view == View::History;
        self.history.files.load_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let changes = repo.commit_changes(&commit)?;
                    PreparedChanges::load(&repo, changes, Some(&commit), preview.then_some(0))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.history.files.files_loading = false;
                match result {
                    Ok(prepared) => {
                        this.reset_copy_feedback();
                        this.repository_preview = false;
                        this.history.selected_commit = Some(index);
                        this.apply_changes(Source::History, prepared, cx);
                        if this.view == View::History {
                            this.ensure_history_preview(cx);
                        }
                    }
                    Err(error) => this.history.files.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn apply_changes(&mut self, source: Source, prepared: PreparedChanges, cx: &mut Context<Self>) {
        self.install_changes(source, prepared, None, cx);
    }

    fn install_changes(
        &mut self,
        source: Source,
        prepared: PreparedChanges,
        stable_side: Option<DiffSide>,
        cx: &mut Context<Self>,
    ) {
        let previous = self.files_for(source);
        let stable_side = stable_side.filter(|_| {
            previous
                .selected_file
                .and_then(|i| previous.changes.get(i))
                .is_some_and(|old| {
                    prepared
                        .changes
                        .get(prepared.preview_index)
                        .is_some_and(|new| old.path == new.path)
                })
        });
        if stable_side.is_none() {
            *self.files_for_mut(source) = ChangeState::new(self.preferences.layout, cx);
        }
        let mut tree = self.repository.as_ref().map(|repo| {
            ChangeTree::new(
                &prepared.changes,
                &prepared.stats,
                &repo.root,
                source == Source::Working,
                cx,
                move |this, index, cx| {
                    // Retained hidden trees must never drive the currently displayed data set.
                    if this.view.source() == source
                        && this.files_for(source).selected_file != Some(index)
                    {
                        this.select_file(index, cx);
                    }
                },
            )
        });
        if let Some(tree) = &mut tree {
            tree.set_progress(prepared.progress.clone());
        }
        let files = self.files_for_mut(source);
        files.progress = prepared.progress;
        files.loaded = true;
        files.error = None;
        files.total_stats = Some(prepared.stats.iter().flatten().copied().sum());
        files.change_tree = tree;
        files.changes = prepared.changes;
        files.file_stats = prepared.stats;
        if let Some(preview) = prepared.preview {
            let files = self.files_for_mut(source);
            files.selected_file = Some(prepared.preview_index);
            if let Some(tree) = &files.change_tree {
                tree.reveal_file(prepared.preview_index, cx);
            }
            self.apply_file_result(source, preview, stable_side, cx);
        }
    }

    fn apply_file_result(
        &mut self,
        source: Source,
        result: anyhow::Result<FileContent>,
        stable_side: Option<DiffSide>,
        cx: &mut Context<Self>,
    ) {
        let files = self.files_for_mut(source);
        match result {
            Ok(content) => {
                let (diffs, image, staging) = match content {
                    FileContent::Text(diffs, staging) => (diffs, None, staging),
                    FileContent::Image(image) => (Vec::new(), Some(image), None),
                };
                let same_content = stable_side.is_some()
                    && files
                        .staging_patch
                        .as_ref()
                        .zip(staging.as_ref())
                        .is_some_and(|(old, new)| old.patches().eq(new.patches()));
                files.staging_patch = staging;
                files.empty_patch = diffs.is_empty() && image.is_none();
                files.image = image.map(|image| cx.new(|_| ImageDiff::new(image)));
                files.image_subscription = files
                    .image
                    .as_ref()
                    .map(|image| cx.observe(image, |_, _, cx| cx.notify()));
                files.diff.update(cx, |diff, cx| {
                    if same_content {
                        return;
                    }
                    if let Some(side) = stable_side {
                        diff.refresh_files(diffs, side, cx);
                    } else {
                        diff.set_files(diffs, cx);
                    }
                });
            }
            Err(error) => {
                files.staging_patch = None;
                files.image = None;
                files.image_subscription = None;
                files.diff.update(cx, |diff, cx| diff.set_files([], cx));
                files.error = Some(format!("{}: {error:#}", tr("load_failed")));
            }
        }
    }

    fn toggle_file_list(&mut self, cx: &mut Context<Self>) {
        self.preferences.tree_file_list = !self.preferences.tree_file_list;
        self.save_preferences();
        if let Some(index) = self.active_files().selected_file {
            if self.preferences.tree_file_list {
                if let Some(tree) = &self.active_files().change_tree {
                    tree.reveal_file(index, cx);
                }
            } else {
                self.active_files()
                    .file_scroll
                    .scroll_to_item(index, ScrollStrategy::Center);
            }
        }
        cx.notify();
    }

    fn select_file(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.loading || self.active_files().files_loading {
            return;
        }
        if self.view == View::Repositories {
            if !self.repository_preview {
                let files = &mut self.history.files;
                // Entering from the tree must not reveal another file's retained preview.
                // A pending selection may still be displaying the previously loaded file.
                if files.selected_file != Some(index) || files.patch_loading {
                    files.diff.update(cx, |diff, cx| diff.set_files([], cx));
                    files.image = None;
                    files.image_subscription = None;
                    files.empty_patch = false;
                }
            }
            self.repository_preview = true;
        }
        self.select_source_file(self.view.source(), index, cx);
        cx.notify();
    }

    fn select_source_file(&mut self, source: Source, index: usize, cx: &mut Context<Self>) {
        if self.loading || self.files_for(source).files_loading {
            return;
        }
        if self.files_for(source).selected_file == Some(index)
            && self.files_for(source).error.is_none()
        {
            return;
        }
        let Some(repo) = self.repository.clone() else {
            return;
        };
        let commit = if source == Source::History {
            self.history
                .selected_commit
                .map(|i| self.history.commits[i].clone())
        } else {
            None
        };
        let files = self.files_for_mut(source);
        let change = files.changes[index].clone();
        let now = cx.background_executor().now();
        // Only a burst of selections needs to settle. An isolated selection
        // should start loading immediately, including its first preview.
        let debounce = files
            .last_file_selection
            .replace(now)
            .is_some_and(|previous| now.duration_since(previous) < FILE_SELECTION_DELAY);
        files.selected_file = Some(index);
        if let Some(tree) = &files.change_tree {
            tree.reveal_file(index, cx);
        }
        files
            .file_scroll
            .scroll_to_item(index, ScrollStrategy::Center);
        files.patch_task = None;
        files.patch_loading = true;
        files.error = None;
        files.empty_patch = false;
        files.patch_task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(FILE_SELECTION_DELAY).await;
            }
            let result = cx
                .background_spawn(async move { FileContent::load(&repo, &change, commit.as_ref()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.files_for_mut(source).patch_loading = false;
                this.apply_file_result(source, result, None, cx);
                cx.notify();
            });
        }));
    }

    fn previous_file(&self) -> Option<usize> {
        self.active_files()
            .selected_file
            .and_then(|index| index.checked_sub(1))
    }

    fn next_file(&self) -> Option<usize> {
        self.active_files()
            .selected_file
            .map(|index| index + 1)
            .filter(|index| *index < self.active_files().changes.len())
    }

    fn commit_message_button(&self, cx: &mut Context<Self>) -> Button {
        icon_button(
            "toggle-commit-message",
            IconName::PanelBottom,
            if self.preferences.show_commit_message {
                "hide_commit_message"
            } else {
                "show_commit_message"
            },
        )
        .text_color(cx.theme().muted_foreground)
        .selected(self.preferences.show_commit_message)
        .toggled(self.preferences.show_commit_message)
        .on_click(cx.listener(|this, _, _, cx| {
            this.preferences.show_commit_message = !this.preferences.show_commit_message;
            this.save_preferences();
            cx.notify();
        }))
    }

    fn commit_id_button(&self, commit: &Commit, cx: &mut Context<Self>) -> Button {
        let id = commit.id.clone();
        Button::new("copy-commit-id")
            .label(if self.commit_copied {
                tr("commit_copied").to_owned()
            } else {
                commit.short_id.clone()
            })
            .xsmall()
            .ghost()
            .min_w(px(72.))
            .text_color(cx.theme().muted_foreground)
            .tooltip(format!("{}\n{}", tr("copy_commit_id"), commit.id))
            .accessibility_label(tr("copy_commit_id"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
                this.commit_copied = true;
                this.copy_feedback_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(COPY_FEEDBACK_DURATION).await;
                    let _ = this.update(cx, |this, cx| {
                        this.commit_copied = false;
                        cx.notify();
                    });
                }));
                cx.notify();
            }))
    }

    fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.preferences.layout = layout;
        for source in [Source::Working, Source::History] {
            self.files_for(source)
                .diff
                .update(cx, |diff, cx| diff.set_mode(diff_mode(layout), cx));
        }
        self.save_preferences();
        cx.notify();
    }

    fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preferences.appearance = appearance;
        apply_appearance(appearance, window, cx);
        self.save_preferences();
        cx.notify();
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences.language = language;
        apply_language(language, cx);
        self.update_translations(window, cx);
        self.save_preferences();
        cx.notify();
    }

    fn update_translations(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tree) = &self.working.change_tree {
            tree.update_group_labels(cx);
        }
        self.history_search.update(cx, |input, cx| {
            input.set_placeholder(tr("search_commits_placeholder"), window, cx);
        });
    }
}

fn diff_mode(layout: Layout) -> DiffMode {
    match layout {
        Layout::Split => DiffMode::Split,
        Layout::Inline => DiffMode::Unified,
    }
}

fn status_label(status: char) -> &'static str {
    tr(match status {
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'T' => "type_changed",
        'U' => "conflict",
        _ => "modified",
    })
}

/// Map the same lowercase literal matches used by Git search back to UTF-8 source ranges.
fn search_match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let folded = text.to_lowercase();
    let matches = folded.match_indices(&query);
    if text.is_ascii() {
        return matches
            .map(|(start, value)| start..start + value.len())
            .collect();
    }

    // Lowercasing can change byte length or expand a character (for example İ).
    let mut folded_offset = 0;
    let spans = text
        .char_indices()
        .map(|(start, ch)| {
            let folded_start = folded_offset;
            folded_offset += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
            (folded_start..folded_offset, start..start + ch.len_utf8())
        })
        .collect::<Vec<_>>();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (start, value) in matches {
        let first = spans.partition_point(|(folded, _)| folded.end <= start);
        let last = spans.partition_point(|(folded, _)| folded.start < start + value.len()) - 1;
        let range = spans[first].1.start..spans[last].1.end;
        if let Some(previous) = ranges
            .last_mut()
            .filter(|previous| previous.end > range.start)
        {
            previous.end = range.end;
        } else {
            ranges.push(range);
        }
    }
    ranges
}

fn list_action_style(cx: &App) -> ButtonCustomVariant {
    ButtonCustomVariant::new(cx)
        .hover(cx.theme().foreground.opacity(0.12))
        .active(cx.theme().foreground.opacity(0.18))
}

fn application_menu_item(
    name: impl Into<SharedString>,
    icon: Option<ApplicationIcon>,
    fallback: IconName,
) -> PopupMenuItem {
    let name = name.into();
    let image: Option<ImageSource> = icon.map(|icon| match icon {
        ApplicationIcon::Png(bytes) => Arc::new(Image::from_bytes(ImageFormat::Png, bytes)).into(),
        ApplicationIcon::File(path) => path.into(),
    });
    PopupMenuItem::element(move |_, _| {
        let icon = match &image {
            Some(image) => img(image.clone())
                .size(px(16.))
                .object_fit(ObjectFit::Contain)
                .flex_none()
                .into_any_element(),
            None => Icon::new(fallback).size(px(16.)).into_any_element(),
        };
        h_flex()
            .id(name.clone())
            .test_support()
            .role(accesskit::Role::Label)
            .aria_label(name.clone())
            .gap_2()
            .child(icon)
            .child(name.clone())
    })
}

fn list_scrollbar(id: &'static str, handle: &(impl ScrollbarHandle + Clone)) -> Div {
    div().absolute().inset_0().child(
        Scrollbar::vertical(handle)
            .id(id)
            .mode(ScrollbarMode::Scrolling)
            .viewport_from_layout()
            .styles(|styles| {
                styles
                    .track(|track| track.width(px(6.)))
                    .thumb(|thumb| thumb.width(px(6.)).inset(px(0.)).radius(px(2.)))
                    .thumb_hover(|thumb| thumb.width(px(6.)).inset(px(0.)))
                    .thumb_active(|thumb| thumb.width(px(6.)).inset(px(0.)))
            }),
    )
}

impl Patchlook {
    fn access_mode_control(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let read_only = self.access_mode == AccessMode::ReadOnly;
        let remote = self.repository.as_ref().is_some_and(Repository::is_remote);
        let busy = self.write_task.is_some() || remote;
        let mode_label = if remote {
            "ssh_read_only"
        } else if read_only {
            "read_only_mode"
        } else {
            "read_write_mode"
        };
        h_flex()
            .gap_1()
            .pl_1()
            .child(
                div()
                    .id("access-mode-indicator")
                    .test_support()
                    .size(px(6.))
                    .flex_none()
                    .rounded_full()
                    .bg(if read_only {
                        cx.theme().green
                    } else {
                        gpui_kit::component::orange_500()
                    })
                    .role(accesskit::Role::Label)
                    .aria_label(tr(mode_label)),
            )
            .child(
                icon_button(
                    "access-mode",
                    if read_only {
                        IconName::Lock
                    } else {
                        IconName::LockOpen
                    },
                    mode_label,
                )
                .toggled(read_only)
                .disabled(busy)
                // A short write lock blocks activation without dimming the control.
                .text_color(cx.theme().foreground)
                .focus_ring(false)
                .tab_stop(!busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_access_mode(
                        if read_only {
                            AccessMode::ReadWrite
                        } else {
                            AccessMode::ReadOnly
                        },
                        cx,
                    );
                })),
            )
    }

    fn application_menu(&self, cx: &App) -> impl IntoElement {
        let fallback_focus = self.focus.clone();
        // Block the titlebar's native drag hitbox around the whole trigger.
        div().occlude().child(
            icon_button("application-menu", IconName::Menu, "application_menu")
                .text_color(cx.theme().muted_foreground)
                .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, window, cx| {
                    menu.action_context(
                        window.focused(cx).unwrap_or_else(|| fallback_focus.clone()),
                    )
                    .menu(
                        tr("new_text_comparison"),
                        Box::new(crate::text_compare::NewTextComparison),
                    )
                    .separator()
                    .menu(tr("about"), Box::new(About))
                    .separator()
                    .menu(tr("quit"), Box::new(Quit))
                }),
        )
    }

    fn language_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let app = cx.entity().downgrade();
        crate::ui::language_menu(move |language, window, cx| {
            let _ = app.update(cx, |this, cx| this.set_language(language, window, cx));
        })
    }

    fn external_file_target(&self) -> Option<FileToOpen> {
        if self.repository.as_ref()?.is_remote() {
            return None;
        }
        let files = self.active_files();
        if self.loading
            || files.files_loading
            || files.patch_loading
            || (self.view.source() == Source::History && self.history.loading)
        {
            return None;
        }
        Some(FileToOpen {
            repository: self.repository.clone()?,
            change: files.changes.get(files.selected_file?)?.clone(),
            commit: self.current_commit().cloned(),
        })
    }

    fn open_external(&mut self, target: FileToOpen, application: OpenWith, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { target.launch(&application) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Some(directory)) => this.external_files.push(directory),
                    Ok(None) => {}
                    Err(error) => {
                        this.error = Some(format!("{}: {error:#}", tr("open_file_failed")))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_external_program(&mut self, target: FileToOpen, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("select_application").into()),
        });
        cx.spawn(async move |this, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                if let Some(program) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        this.open_external(target, OpenWith::Program(program), cx)
                    });
                }
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

    fn open_with_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let target = self.external_file_target();
        let app = cx.entity().downgrade();
        icon_button("open-with", IconName::ExternalLink, "open_with")
            .when(target.is_some() || self.write_task.is_some(), |button| {
                button.text_color(cx.theme().muted_foreground)
            })
            .disabled(target.is_none())
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |mut menu, _, cx| {
                let Some(target) = target.clone() else {
                    return menu;
                };
                menu = menu
                    .min_w(px(220.))
                    .max_h(px(360.))
                    .scrollable(true)
                    .label(tr(if target.change.status == 'D' {
                        "open_previous_version"
                    } else {
                        "open_working_file"
                    }));
                let mut choices = vec![(
                    tr("system_default_application").to_owned(),
                    OpenWith::Default,
                    None,
                )];
                match applications_for(&target.path()) {
                    Ok(applications) => {
                        choices.extend(applications.into_iter().map(|application| {
                            (
                                application.name,
                                OpenWith::Recommended(application.id),
                                application.icon,
                            )
                        }))
                    }
                    Err(error) => {
                        let _ = app.update(cx, |this, cx| {
                            this.error = Some(format!("{}: {error:#}", tr("applications_failed")));
                            cx.notify();
                        });
                    }
                }
                for (name, application, icon) in choices {
                    let app = app.clone();
                    let target = target.clone();
                    menu = menu.item(
                        application_menu_item(name, icon, IconName::Monitor).on_click(
                            move |_, _, cx| {
                                let _ = app.update(cx, |this, cx| {
                                    this.open_external(target.clone(), application.clone(), cx)
                                });
                            },
                        ),
                    );
                }
                let app = app.clone();
                menu.separator().item(
                    application_menu_item(tr("other_application"), None, IconName::FolderOpen)
                        .on_click(move |_, _, cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.choose_external_program(target.clone(), cx)
                            });
                        }),
                )
            })
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let branch = match self.view.source() {
            Source::Working => self.branch.clone(),
            Source::History => self.viewed_branch_name(),
        };
        let branch_tooltip = tr(match self.view.source() {
            Source::Working => "working_branch_tooltip",
            Source::History => "viewed_branch_tooltip",
        })
        .replace("{branch}", &branch)
        .replace("{worktree}", &self.branch);
        let repository_name = self.repository.as_ref().map(|repo| repo.location.name());
        let repository_icon = if self.repository.as_ref().is_some_and(Repository::is_remote) {
            IconName::Server
        } else {
            IconName::FolderOpen
        };
        // Controls must hide the underlying Drag hitbox from Windows' native
        // hit testing, which runs before ordinary mouse/click event handlers.
        TitleBar::new().bg(cx.theme().title_bar).child(
            h_flex()
                .size_full()
                .min_w_0()
                .pr_2()
                .gap_2()
                .when(
                    cfg!(any(target_os = "windows", target_os = "linux")),
                    |bar| bar.child(self.application_menu(cx)),
                )
                .child(self.repository_menu("open-repository", repository_icon, false, cx))
                .child(
                    div()
                        .max_w(px(180.))
                        .min_w_0()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(repository_name.unwrap_or_else(|| tr("app_name").to_owned())),
                )
                .when(self.repository.is_some(), |view| {
                    view.child(
                        h_flex()
                            .id("toolbar-branch")
                            .test_support()
                            .role(accesskit::Role::Label)
                            .aria_label(branch_tooltip.clone())
                            .gap_1()
                            .max_w(px(320.))
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .tooltip(move |window, cx| {
                                Tooltip::new(branch_tooltip.clone()).build(window, cx)
                            })
                            .child(Icon::new(IconName::GitBranch).xsmall())
                            .child(div().truncate().child(branch)),
                    )
                })
                .child(
                    icon_button("refresh", IconName::RefreshCw, "refresh")
                        .occlude()
                        .xsmall()
                        .text_color(cx.theme().muted_foreground)
                        .disabled(self.repository.is_none() || self.loading)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.refresh(&Refresh, window, cx)),
                        ),
                )
                .child(div().flex_1())
                .child(
                    h_flex()
                        .gap_1()
                        .child(div().occlude().child(self.access_mode_control(cx)))
                        .child(div().occlude().child(self.language_menu(cx))),
                )
                .child({
                    let app = cx.entity().downgrade();
                    crate::ui::theme_button(cx, move |appearance, window, cx| {
                        let _ =
                            app.update(cx, |this, cx| this.set_appearance(appearance, window, cx));
                    })
                }),
        )
    }

    fn repositories(&self, cx: &mut Context<Self>) -> AnyElement {
        let has_remote = self
            .preferences
            .recent_repositories
            .iter()
            .any(|entry| entry.path.is_remote());
        let has_local = self
            .preferences
            .recent_repositories
            .iter()
            .any(|entry| !entry.path.is_remote());
        let show_groups = has_local && has_remote;
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .flex_none()
                    .h(px(32.))
                    .px_3()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("recent_repositories"))
                    .child(
                        self.preferences
                            .recent_repositories
                            .iter()
                            .filter(|e| e.submodule.is_none())
                            .count()
                            .to_string(),
                    )
                    .child(div().flex_1())
                    .child(self.repository_menu("add-repository", IconName::Plus, true, cx))
                    .child(
                        icon_button("remove-repository", IconName::Minus, "remove_repository")
                            .xsmall()
                            .text_color(cx.theme().muted_foreground)
                            .disabled(
                                self.selected_repository.is_none()
                                    || self.selected_repository.as_ref().is_some_and(|path| {
                                        self.preferences.submodule(path).is_some()
                                    })
                                    || self.loading
                                    || self.write_task.is_some(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.remove_repository(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .id("repositories-list")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.repository_scroll)
                            .children([false, true].into_iter().filter_map(|remote| {
                                let entries: Vec<_> = self
                                    .preferences
                                    .recent_repositories
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, entry)| {
                                        entry.path.is_remote() == remote
                                            && entry.submodule.is_none()
                                    })
                                    .map(|(index, _)| index)
                                    .collect();
                                if entries.is_empty() {
                                    return None;
                                }
                                let group = usize::from(remote);
                                let expanded = !self.collapsed_repository_groups[group];
                                let title = tr(if remote {
                                    "remote_repositories"
                                } else {
                                    "local_repositories"
                                });
                                Some(
                                    v_flex()
                                        .flex_none()
                                        .when(show_groups, |section| {
                                            section.child(
                                                Button::new(if remote {
                                                    "remote-repositories"
                                                } else {
                                                    "local-repositories"
                                                })
                                                .custom(
                                                    ButtonCustomVariant::new(cx)
                                                        .hover(cx.theme().list_hover),
                                                )
                                                .toggled(expanded)
                                                .accessibility_label(title)
                                                .w_full()
                                                .h(px(28.))
                                                .px_3()
                                                .justify_start()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    h_flex()
                                                        .w_full()
                                                        .justify_start()
                                                        .gap_2()
                                                        .text_xs()
                                                        .child(title)
                                                        .child(entries.len().to_string())
                                                        .child(
                                                            Icon::new(if expanded {
                                                                IconName::ChevronDown
                                                            } else {
                                                                IconName::ChevronRight
                                                            })
                                                            .xsmall(),
                                                        ),
                                                )
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.collapsed_repository_groups[group] =
                                                        !this.collapsed_repository_groups[group];
                                                    cx.notify();
                                                })),
                                            )
                                        })
                                        .when(!show_groups || expanded, |section| {
                                            section.children(entries.into_iter().map(|index| {
                                                self.repository_item(index, 0, true, cx)
                                            }))
                                        }),
                                )
                            })),
                    )
                    .child(list_scrollbar(
                        "repositories-scrollbar",
                        &self.repository_scroll,
                    )),
            )
            .into_any_element()
    }

    fn remote_status(&self, path: &RepositoryLocation) -> &'static str {
        if self
            .pending_connection
            .as_ref()
            .is_some_and(|(target, _)| target == path)
        {
            "ssh_connecting"
        } else if self.remote_errors.contains_key(path) {
            "ssh_connection_failed"
        } else if self.connected_repository(path).is_some() {
            "ssh_connected"
        } else {
            "ssh_disconnected"
        }
    }

    fn repository_item(
        &self,
        index: usize,
        depth: usize,
        last: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let path = &self.preferences.recent_repositories[index].path;
        let children: Vec<_> = self
            .preferences
            .recent_repositories
            .iter()
            .enumerate()
            .filter(|(_, e)| e.submodule.as_ref().is_some_and(|s| &s.parent == path))
            .map(|(index, _)| index)
            .collect();
        let expanded = !self.collapsed_repositories.contains(path)
            && depth < self.preferences.recent_repositories.len();
        v_flex()
            .relative()
            .flex_none()
            .child(self.repository_row(index, depth, cx))
            .when(expanded, |group| {
                group.children(children.iter().enumerate().map(|(position, &index)| {
                    self.repository_item(index, depth + 1, position + 1 == children.len(), cx)
                }))
            })
            .when(depth > 0, |group| {
                let left = px(8.
                    + REPOSITORY_DISCLOSURE_WIDTH / 2.
                    + (depth - 1) as f32 * REPOSITORY_TREE_INDENT);
                let middle = px((SUBMODULE_ROW_HEIGHT - 2.) / 2.);
                let color = cx.theme().border;
                group
                    .child(
                        div()
                            .absolute()
                            .left(left)
                            .top_0()
                            .w(px(1.))
                            .map(|line| if last { line.h(middle) } else { line.h_full() })
                            .bg(color),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(left)
                            .top(middle)
                            .w(px(REPOSITORY_TREE_INDENT - REPOSITORY_DISCLOSURE_WIDTH / 2.))
                            .h(px(1.))
                            .bg(color),
                    )
            })
            .when(depth == 0 && expanded && !children.is_empty(), |group| {
                group.pb_1()
            })
    }

    fn repository_row(&self, index: usize, depth: usize, cx: &mut Context<Self>) -> Div {
        let entry = &self.preferences.recent_repositories[index];
        let path = entry.path.clone();
        let initialized = entry.submodule.as_ref().is_none_or(|s| s.initialized);
        let submodule = entry.submodule.is_some();
        let has_children = self
            .preferences
            .recent_repositories
            .iter()
            .any(|e| e.submodule.as_ref().is_some_and(|s| s.parent == path));
        let expanded = !self.collapsed_repositories.contains(&path);
        let selected = self.selected_repository.as_ref() == Some(&path);
        let remote = path.is_remote();
        let group = SharedString::from(format!("repository-row-{index}"));
        let row_path = path.clone();
        let status = if !initialized {
            "submodule_uninitialized"
        } else if remote {
            self.remote_status(&path)
        } else {
            "submodule"
        };
        let tooltip = self
            .remote_errors
            .get(&path)
            .map(|error| format!("{path}\n{error}"))
            .unwrap_or_else(|| path.to_string());
        div()
            .h(px(if submodule { SUBMODULE_ROW_HEIGHT } else { 50. }))
            .flex_none()
            .px_2()
            .pb(px(if submodule { 2. } else { 4. }))
            .child(
                h_flex()
                    .group(group.clone())
                    .size_full()
                    .rounded_md()
                    .when(selected, |row| row.bg(cx.theme().secondary_active))
                    .hover(|row| {
                        row.bg(if selected {
                            cx.theme().secondary_active
                        } else {
                            cx.theme().list_hover
                        })
                    })
                    .child(
                        div()
                            .flex_none()
                            .w(px(depth as f32 * REPOSITORY_TREE_INDENT)),
                    )
                    .when(has_children || submodule, |row| {
                        row.child(div().flex_none().w(px(REPOSITORY_DISCLOSURE_WIDTH)).when(
                            has_children,
                            |slot| {
                                let path = path.clone();
                                slot.child(
                                    Button::new(("repository-expand", index))
                                        .icon(if expanded {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .xsmall()
                                        .w(px(REPOSITORY_DISCLOSURE_WIDTH))
                                        .custom(ButtonCustomVariant::new(cx))
                                        .text_color(cx.theme().muted_foreground)
                                        .accessibility_label(tr("toggle_submodules"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if !this.collapsed_repositories.remove(&path) {
                                                this.collapsed_repositories.insert(path.clone());
                                            }
                                            cx.notify();
                                        })),
                                )
                            },
                        ))
                    })
                    .child(
                        Button::new(("repository", index))
                            .custom(ButtonCustomVariant::new(cx))
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .when(has_children || submodule, |button| button.pl_0())
                            .justify_start()
                            .tooltip(tooltip)
                            .accessibility_label(if submodule {
                                format!("{}: {path}", tr("submodule"))
                            } else {
                                path.to_string()
                            })
                            .disabled(self.write_task.is_some() || !initialized)
                            .child(
                                h_flex()
                                    .gap_2()
                                    .w_full()
                                    .min_w_0()
                                    .child(
                                        Icon::new(if submodule {
                                            IconName::GitBranch
                                        } else if remote {
                                            IconName::Server
                                        } else {
                                            IconName::Folder
                                        })
                                        .small()
                                        .text_color(cx.theme().muted_foreground),
                                    )
                                    .child(
                                        v_flex()
                                            .min_w_0()
                                            .flex_1()
                                            .items_start()
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .w_full()
                                                    .min_w_0()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .min_w_0()
                                                            .flex_1()
                                                            .truncate()
                                                            .child(path.name()),
                                                    )
                                                    .when(remote || !initialized, |row| {
                                                        row.child(
                                                            div()
                                                                .id(("repository-status", index))
                                                                .test_support()
                                                                .text_xs()
                                                                .text_color(
                                                                    cx.theme().muted_foreground,
                                                                )
                                                                .aria_label(tr(status))
                                                                .child(tr(status)),
                                                        )
                                                    }),
                                            )
                                            .when(!submodule, |column| {
                                                column.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .truncate()
                                                        .child(path.to_string()),
                                                )
                                            }),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_saved_repository(row_path.clone(), window, cx)
                            })),
                    )
                    .when(remote && initialized, |row| {
                        let disconnect = path.clone();
                        let reconnect = path.clone();
                        let can_disconnect = self
                            .pending_connection
                            .as_ref()
                            .is_some_and(|(target, _)| target == &path)
                            || (!self.loading && self.connected_repository(&path).is_some());
                        let disconnect_app = cx.entity().downgrade();
                        let reconnect_app = disconnect_app.clone();
                        let busy = self.write_task.is_some();
                        row.child(
                            div()
                                .flex_none()
                                .pr_1()
                                .invisible()
                                .group_hover(group, |style| style.visible())
                                .child(
                                    Button::new(("repository-more", index))
                                        .icon(IconName::Ellipsis)
                                        .xsmall()
                                        .custom(list_action_style(cx))
                                        .text_color(cx.theme().muted_foreground)
                                        .accessibility_label(tr("repository_actions"))
                                        .disabled(busy)
                                        .dropdown_menu_with_anchor(
                                            Anchor::TopRight,
                                            move |menu, _, _| {
                                                let target = disconnect.clone();
                                                let app = disconnect_app.clone();
                                                let reconnect = reconnect.clone();
                                                let reconnect_app = reconnect_app.clone();
                                                menu.item(
                                                    PopupMenuItem::new(tr("ssh_disconnect"))
                                                        .disabled(!can_disconnect)
                                                        .on_click(move |_, window, cx| {
                                                            let _ = app.update(cx, |this, cx| {
                                                                this.disconnect_repository(
                                                                    &target, window, cx,
                                                                )
                                                            });
                                                        }),
                                                )
                                                .item(
                                                    PopupMenuItem::new(tr("ssh_reconnect"))
                                                        .on_click(move |_, window, cx| {
                                                            let _ = reconnect_app.update(
                                                                cx,
                                                                |this, cx| {
                                                                    this.reconnect_repository(
                                                                        reconnect.clone(),
                                                                        window,
                                                                        cx,
                                                                    )
                                                                },
                                                            );
                                                        }),
                                                )
                                            },
                                        ),
                                ),
                        )
                    }),
            )
    }

    fn branch_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = if self.repository.is_some() {
            self.branches.len() + 1
        } else {
            0
        };
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .flex_none()
                    .h(px(32.))
                    .px_3()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("branches"))
                    .child(self.branches.len().to_string()),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "branches-list",
                            count,
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|index| this.branch_row(index, cx))
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(&self.branch_scroll)
                        .size_full(),
                    )
                    .child(list_scrollbar("branches-scrollbar", &self.branch_scroll)),
            )
            .into_any_element()
    }

    fn branch_row(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let branch = index
            .checked_sub(1)
            .and_then(|index| self.branches.get(index));
        let reference = branch.map(|branch| branch.reference.clone());
        let selected = self.viewed_branch() == reference.as_deref();
        let name = branch
            .map(|branch| branch.name.clone())
            .unwrap_or_else(|| tr("head_branch").to_owned());
        let caption = branch.map(|branch| {
            if branch.is_head {
                "worktree_branch"
            } else if branch.remote {
                "remote_branch"
            } else {
                "local_branch"
            }
        });
        div().h(px(36.)).px_2().pb_1().child(
            h_flex()
                .id(("branch-row", index))
                .size_full()
                .rounded_md()
                .when(selected, |row| row.bg(cx.theme().secondary_active))
                .hover(|style| {
                    style.bg(if selected {
                        cx.theme().secondary_active
                    } else {
                        cx.theme().list_hover
                    })
                })
                .child(
                    Button::new(("branch", index))
                        .custom(ButtonCustomVariant::new(cx))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .justify_start()
                        .disabled(self.loading)
                        .tooltip(name.clone())
                        .accessibility_label(name.clone())
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap_2()
                                .child(
                                    Icon::new(if selected {
                                        IconName::Eye
                                    } else {
                                        IconName::GitBranch
                                    })
                                    .xsmall(),
                                )
                                .child(div().flex_1().min_w_0().truncate().text_sm().child(name))
                                .when_some(caption, |row, key| {
                                    row.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr(key)),
                                    )
                                }),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.browse_branch(reference.clone(), cx)
                        })),
                ),
        )
    }

    fn sidebar_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        base::Tabs::new("sidebar-tabs")
            .flex()
            .flex_none()
            .h(px(32.))
            .bg(cx.theme().tab_bar)
            .children(
                [
                    (View::Repositories, "repositories"),
                    (View::Working, "working_tree"),
                    (View::History, "history"),
                ]
                .map(|(view, key)| {
                    let selected = self.view == view;
                    base::Tab::new(key)
                        .flex_1()
                        .h_full()
                        .text_sm()
                        .selected(selected)
                        .accessibility_label(tr(key))
                        .bg(if selected {
                            cx.theme().sidebar
                        } else {
                            cx.theme().tab_bar
                        })
                        .text_color(if selected {
                            cx.theme().foreground
                        } else {
                            cx.theme().muted_foreground
                        })
                        .when(!selected, |tab| {
                            tab.hover(|style| style.bg(cx.theme().list_hover))
                        })
                        .child(tr(key))
                        .on_click(cx.listener(move |this, _, _, cx| this.switch_view(view, cx)))
                }),
            )
    }

    fn diff_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stage_button = self
            .staging_visible()
            .then(|| {
                self.working
                    .selected_file
                    .and_then(|i| self.working.changes.get(i))
                    .map(|change| {
                        self.staging_button(
                            "stage-current-file",
                            change.clone(),
                            StageSelection::File,
                            cx,
                        )
                    })
            })
            .flatten();
        let file = self.active_files().diff.read(cx).files().first();
        let image = self
            .active_files()
            .image
            .as_ref()
            .map(|image| image.read(cx));
        let selected_path = self
            .active_files()
            .selected_file
            .and_then(|i| self.active_files().changes.get(i))
            .map(|change| change.path.as_path());
        let path = image
            .map(ImageDiff::path)
            .or_else(|| file.map(|file| std::path::Path::new(file.path().as_str())))
            .or(selected_path);
        let title = image
            .map(|image| image.title().to_owned())
            .or_else(|| {
                file.map(|file| match (file.original_path(), file.modified_path()) {
                    (Some(old), Some(new)) if old != new => format!("{old} → {new}"),
                    _ => file.path().to_string(),
                })
            })
            .or_else(|| selected_path.map(|path| path.display().to_string()));
        let folded_context = self.active_files().diff.read(cx).has_collapsed_context();
        h_flex()
            .flex_none()
            .h(px(32.))
            .px_2()
            .gap_1()
            .border_b(px(1. / window.scale_factor()))
            .border_color(cx.theme().border)
            .when(self.view == View::Repositories, |row| {
                row.child(
                    icon_button("back-to-files", IconName::ArrowLeft, "back_to_files")
                        .text_color(cx.theme().muted_foreground)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.repository_preview = false;
                            cx.notify();
                        })),
                )
            })
            .when_some(path, |row, path| {
                row.child(self.open_with_menu(cx)).child(file_icon(path))
            })
            .when_some(stage_button, |row, button| row.child(button))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    // Align the visible glyphs with the centered toolbar icon silhouettes.
                    .relative()
                    .top(px(-1.))
                    .child(title.unwrap_or_default()),
            )
            .when(self.staging_visible(), |row| {
                row.child(
                    icon_button("undo-write", IconName::Undo2, "undo_write")
                        .when(self.write_history.can_undo(), |button| {
                            button.text_color(cx.theme().muted_foreground)
                        })
                        .disabled(self.staging_disabled() || !self.write_history.can_undo())
                        .on_click(cx.listener(|this, _, _, cx| this.restore_write(true, cx))),
                )
                .child(
                    icon_button("redo-write", IconName::Redo2, "redo_write")
                        .when(self.write_history.can_redo(), |button| {
                            button.text_color(cx.theme().muted_foreground)
                        })
                        .disabled(self.staging_disabled() || !self.write_history.can_redo())
                        .on_click(cx.listener(|this, _, _, cx| this.restore_write(false, cx))),
                )
            })
            .when_some(file, |row, file| {
                row.child(
                    h_flex()
                        .flex_none()
                        .gap_2()
                        .px_2()
                        .text_xs()
                        .child(
                            div().text_color(cx.theme().muted_foreground).child(
                                tr("diff_blocks")
                                    .replace("{count}", &file.change_block_count().to_string()),
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
            .child(
                icon_button("previous-file", IconName::ChevronUp, "previous_file")
                    .when(self.previous_file().is_some(), |button| {
                        button.text_color(cx.theme().muted_foreground)
                    })
                    .disabled(self.previous_file().is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(index) = this.previous_file() {
                            this.select_file(index, cx);
                        }
                    })),
            )
            .child(
                icon_button("next-file", IconName::ChevronDown, "next_file")
                    .when(self.next_file().is_some(), |button| {
                        button.text_color(cx.theme().muted_foreground)
                    })
                    .disabled(self.next_file().is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(index) = this.next_file() {
                            this.select_file(index, cx);
                        }
                    })),
            )
            .child(toolbar_divider(cx))
            .when(self.active_files().image.is_none(), |row| {
                row.child(
                    icon_button(
                        "toggle-context",
                        if folded_context {
                            IconName::ChevronsUpDown
                        } else {
                            IconName::ChevronsDownUp
                        },
                        if folded_context {
                            "expand_context"
                        } else {
                            "collapse_context"
                        },
                    )
                    .when(file.is_some(), |button| {
                        button.text_color(cx.theme().muted_foreground)
                    })
                    .disabled(file.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.active_files().diff.update(cx, |diff, cx| {
                            if diff.has_collapsed_context() {
                                diff.expand_unchanged(cx);
                            } else {
                                diff.collapse_unchanged(cx);
                            }
                        });
                    })),
                )
                .child(
                    icon_button("soft-wrap", IconName::TextWrap, "soft_wrap")
                        .text_color(cx.theme().muted_foreground)
                        .selected(self.preferences.soft_wrap)
                        .toggled(self.preferences.soft_wrap)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.preferences.soft_wrap = !this.preferences.soft_wrap;
                            this.save_preferences();
                            cx.notify();
                        })),
                )
                .children(
                    [
                        (Layout::Split, "split", IconName::Columns2),
                        (Layout::Inline, "inline", IconName::Rows2),
                    ]
                    .map(|(layout, key, icon)| {
                        icon_button(key, icon, key)
                            .text_color(cx.theme().muted_foreground)
                            .selected(self.preferences.layout == layout)
                            .toggled(self.preferences.layout == layout)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.set_layout(layout, cx)),
                            )
                    }),
                )
            })
            .when_some(self.active_files().image.as_ref(), |row, image| {
                row.child(ImageDiff::toolbar(image, cx))
            })
            .when(self.current_commit().is_some(), |row| {
                row.child(toolbar_divider(cx))
                    .child(self.commit_message_button(cx))
            })
    }

    fn files(&self, cx: &mut Context<Self>) -> AnyElement {
        let tree_view = self.view == View::Repositories || self.preferences.tree_file_list;
        let tree = self
            .active_files()
            .change_tree
            .as_ref()
            .filter(|_| tree_view);
        v_flex()
            .size_full()
            .min_h_0()
            .bg(if self.view == View::Repositories {
                cx.theme().background
            } else {
                cx.theme().sidebar
            })
            .child(
                h_flex()
                    .flex_none()
                    .h(px(32.))
                    .pl_3()
                    .pr_2()
                    .when(!tree_view, |row| row.pr_4())
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("files"))
                    .child(self.active_files().changes.len().to_string())
                    .when(self.view != View::Repositories, |row| {
                        row.child(
                            icon_button(
                                "toggle-file-list",
                                if self.preferences.tree_file_list {
                                    IconName::List
                                } else {
                                    IconName::FolderTree
                                },
                                if self.preferences.tree_file_list {
                                    "flat_file_list"
                                } else {
                                    "tree_file_list"
                                },
                            )
                            .ml_1()
                            .icon(
                                Icon::new(if self.preferences.tree_file_list {
                                    IconName::List
                                } else {
                                    IconName::FolderTree
                                })
                                .text_color(cx.theme().muted_foreground),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_file_list(cx))),
                        )
                        .child(self.file_stats_button(cx))
                    })
                    .child(div().flex_1())
                    .when_some(
                        tree.filter(|_| self.view == View::Repositories),
                        |row, tree| {
                            let expand = tree.has_collapsed_directories(cx);
                            row.child(
                                icon_button(
                                    "toggle-tree-directories",
                                    if expand {
                                        IconName::ChevronsUpDown
                                    } else {
                                        IconName::ChevronsDownUp
                                    },
                                    if expand {
                                        "expand_directories"
                                    } else {
                                        "collapse_directories"
                                    },
                                )
                                .disabled(self.active_files().changes.is_empty())
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Some(tree) = &this.active_files().change_tree {
                                            tree.toggle_directories(cx);
                                        }
                                    },
                                )),
                            )
                            .child(self.file_stats_button(cx))
                            .child(toolbar_divider(cx))
                        },
                    )
                    .when_some(self.active_files().total_stats, |row, stats| {
                        row.child(
                            h_flex()
                                .gap_2()
                                .id("change-totals")
                                .test_support()
                                .role(accesskit::Role::Label)
                                .aria_label(format!(
                                    "{} +{} −{}",
                                    tr("total_changes"),
                                    stats.additions,
                                    stats.deletions
                                ))
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
                        )
                    })
                    .when(
                        self.view == View::Repositories && self.current_commit().is_some(),
                        |row| {
                            row.child(toolbar_divider(cx))
                                .child(self.commit_message_button(cx))
                        },
                    ),
            )
            .child(if let Some(tree) = tree {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(tree.render(
                        match self.view {
                            View::Working => Some(self.branch.clone()),
                            View::Repositories => Some(self.viewed_branch_name()),
                            View::History => None,
                        },
                        self.preferences.show_file_stats,
                        self.view == View::Working,
                        cx.listener(|this, index: &usize, _, cx| this.select_file(*index, cx)),
                        self.staging_visible().then(|| {
                            let app = cx.entity().downgrade();
                            Rc::new(
                                move |index: Option<usize>,
                                      id: &str,
                                      path: &str,
                                      indices: &[usize],
                                      cx: &mut App| {
                                    app.update(cx, |this, cx| {
                                        if let Some(index) = index {
                                            this.staging_button(
                                                ("stage-file", index),
                                                this.working.changes[index].clone(),
                                                StageSelection::File,
                                                cx,
                                            )
                                        } else {
                                            this.staging_directory_button(id, path, indices, cx)
                                        }
                                        .into_any_element()
                                    })
                                    .unwrap_or_else(|_| div().into_any_element())
                                },
                            ) as crate::change_tree::EntryActionRenderer
                        }),
                        cx,
                    ))
                    .into_any_element()
            } else {
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "files-list",
                            self.active_files().changes.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|index| this.file_row(index, cx))
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(&self.active_files().file_scroll)
                        .size_full(),
                    )
                    .child(list_scrollbar(
                        "files-list-scrollbar",
                        &self.active_files().file_scroll,
                    ))
                    .into_any_element()
            })
            .into_any_element()
    }

    fn file_stats_button(&self, cx: &mut Context<Self>) -> Button {
        icon_button(
            "toggle-file-stats",
            IconName::Diff,
            if self.preferences.show_file_stats {
                "hide_file_stats"
            } else {
                "show_file_stats"
            },
        )
        .when(self.view != View::Repositories, |button| {
            button.icon(Icon::new(IconName::Diff).text_color(cx.theme().muted_foreground))
        })
        .selected(self.preferences.show_file_stats)
        .toggled(self.preferences.show_file_stats)
        .on_click(cx.listener(|this, _, _, cx| {
            this.preferences.show_file_stats = !this.preferences.show_file_stats;
            this.save_preferences();
            cx.notify();
        }))
    }

    fn file_row(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let change = &self.active_files().changes[index];
        let filename = change
            .path
            .file_name()
            .unwrap_or(change.path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let caption = if self.view == View::Working {
            crate::change_tree::working_caption(
                change.area,
                self.active_files().progress.get(index).copied().flatten(),
            )
        } else {
            status_label(change.status).to_owned()
        };
        let status_color = file_status_color(change.status, cx);
        h_flex()
            .h(px(50.))
            .w_full()
            .px_2()
            .pb_1()
            .gap_2()
            .when(self.staging_visible(), |row| {
                row.child(self.staging_button(
                    ("stage-file", index),
                    change.clone(),
                    StageSelection::File,
                    cx,
                ))
            })
            .child(
                Button::new(("file", index))
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .hover(cx.theme().list_hover)
                            .active(cx.theme().secondary_active),
                    )
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .justify_start()
                    .px_2()
                    .border_0()
                    .selected(self.active_files().selected_file == Some(index))
                    .tooltip(change.path.display().to_string())
                    .accessibility_label(format!("{}, {}", change.path.display(), caption))
                    .child(
                        h_flex()
                            .gap_2()
                            .w_full()
                            .min_w_0()
                            .child(file_icon(&change.path).size(px(20.)))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .items_start()
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .text_sm()
                                                    .text_color(status_color)
                                                    .truncate()
                                                    .child(filename),
                                            )
                                            .when_some(
                                                self.active_files()
                                                    .file_stats
                                                    .get(index)
                                                    .copied()
                                                    .filter(|_| self.preferences.show_file_stats),
                                                |row, stats| row.child(stats_label(stats, cx)),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                div().flex_1().min_w_0().truncate().child(
                                                    change
                                                        .path
                                                        .parent()
                                                        .filter(|path| !path.as_os_str().is_empty())
                                                        .map(|path| path.display().to_string())
                                                        .unwrap_or_else(|| ".".into()),
                                                ),
                                            )
                                            .child(div().flex_none().child(caption)),
                                    ),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_file(index, cx))),
            )
    }

    fn history(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .min_h_0()
            .bg(if self.view == View::Repositories {
                cx.theme().background
            } else {
                cx.theme().sidebar
            })
            .child(
                h_flex()
                    .flex_none()
                    .h(px(32.))
                    .px_3()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("commits"))
                    .child(self.history.commits.len().to_string())
                    .when(self.view != View::History, |row| {
                        row.child(div().flex_1())
                            .child(Icon::new(IconName::GitBranch).xsmall())
                            .child(
                                div()
                                    .id("history-branch")
                                    .test_support()
                                    .min_w_0()
                                    .truncate()
                                    .child(self.viewed_branch_name()),
                            )
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "commits-list",
                            self.history.commits.len() + usize::from(self.history.has_more),
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|index| {
                                        if index == this.history.commits.len() {
                                            this.load_more_row(cx)
                                        } else {
                                            this.commit_row(index, cx)
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(&self.history.scroll)
                        .size_full(),
                    )
                    .child(list_scrollbar(
                        "commits-list-scrollbar",
                        &self.history.scroll,
                    )),
            )
            .when(
                self.history.search_available || !self.history.query.is_empty(),
                |view| {
                    view.child(
                        h_flex()
                            .flex_none()
                            .p_2()
                            .gap_1()
                            .on_action(cx.listener(
                                |this, _: &component::input::Escape, window, cx| {
                                    window.focus(&this.focus, cx);
                                },
                            ))
                            .child(
                                Input::new(&self.history_search)
                                    .id("history-search-input")
                                    .aria_label(tr("search_commits_placeholder"))
                                    .small()
                                    .cleanable(true)
                                    .flex_1(),
                            )
                            .child(
                                icon_button("search-commits", IconName::Search, "search_commits")
                                    .small()
                                    .disabled(
                                        self.loading
                                            || (self.history.loading
                                                && self.history.revision.is_none()),
                                    )
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.submit_history_search(cx)
                                        }),
                                    ),
                            ),
                    )
                },
            )
            .into_any_element()
    }

    fn load_more_row(&self, cx: &mut Context<Self>) -> Div {
        div().h(px(50.)).px_2().pb_1().child(
            Button::new("more-commits")
                .label(tr("load_more"))
                .small()
                .ghost()
                .size_full()
                .loading(self.history.loading)
                .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
        )
    }

    fn commit_row(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let commit = &self.history.commits[index];
        div().h(px(50.)).px_2().pb_1().child(
            Button::new(("commit", index))
                .custom(
                    ButtonCustomVariant::new(cx)
                        .hover(cx.theme().list_hover)
                        .active(cx.theme().secondary_active),
                )
                .w_full()
                .h_full()
                .justify_start()
                .selected(self.history.selected_commit == Some(index))
                .tooltip(commit.subject.clone())
                .accessibility_label(format!(
                    "{}, {}, {}",
                    commit.subject, commit.author, commit.date
                ))
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .items_start()
                        .gap_1()
                        .child(div().text_sm().truncate().child(
                            StyledText::new(commit.subject.clone()).with_highlights(
                                self.commit_highlights(commit, &commit.subject, cx),
                            ),
                        ))
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(div().flex_1().truncate().child(commit.author.clone()))
                                .child(commit.date.clone()),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select_commit(index, cx))),
        )
    }

    fn commit_highlights(
        &self,
        commit: &Commit,
        text: &str,
        cx: &App,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let query = self.history.query.to_lowercase();
        if query.is_empty() || commit.id.starts_with(&query) {
            return Vec::new();
        }
        search_match_ranges(text, &query)
            .into_iter()
            .map(|range| {
                (
                    range,
                    HighlightStyle {
                        background_color: Some(cx.theme().yellow.opacity(0.3)),
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    fn detail(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let content = if (self.loading
            || self.active_files().files_loading
            || (self.view != View::Working && self.history.loading))
            && !self.active_files().loaded
        {
            self.message("loading", None, cx)
        } else if self.view == View::Repositories
            && !self.repository_preview
            && self.history.selected_commit.is_some()
        {
            self.files(cx)
        } else if self.active_files().patch_loading
            && self.active_files().diff.read(cx).files().is_empty()
            && self.active_files().image.is_none()
        {
            self.message("loading_diff", None, cx)
        } else if self.active_files().empty_patch {
            self.message("no_patch", Some("no_patch_body"), cx)
        } else if self.visible_error().is_some()
            && self.active_files().diff.read(cx).files().is_empty()
            && self.active_files().image.is_none()
        {
            self.message("load_failed", None, cx)
        } else if let Some(image) = &self.active_files().image {
            image.clone().into_any_element()
        } else if self.active_files().selected_file.is_some() {
            Diff::new(&self.active_files().diff)
                .line_number_selection(false)
                .when(
                    self.staging_visible() && self.working.staging_patch.is_some(),
                    |diff| {
                        let app = cx.entity().downgrade();
                        diff.render_gutter_actions(move |position, side, cx| {
                            app.update(cx, |this, cx| {
                                this.staging_gutter_actions(position, side, cx)
                            })
                            .unwrap_or_else(|_| div().into_any_element())
                        })
                    },
                )
                .when_some(
                    cx.theme().highlight_theme.style.editor_foreground,
                    |diff, color| diff.text_color(color),
                )
                .change_indicator(DiffChangeIndicator::Bars)
                .header_visible(
                    self.view == View::Working
                        && self
                            .working
                            .staging_patch
                            .as_ref()
                            .is_some_and(|p| p.separate_versions()),
                )
                .when(self.view == View::Working, |diff| {
                    let review = self.working.staging_patch.clone();
                    let paths = self
                        .working
                        .diff
                        .read(cx)
                        .files()
                        .iter()
                        .map(|f| f.path().clone())
                        .collect::<Vec<_>>();
                    let header_review = review.clone();
                    let header_paths = paths.clone();
                    diff.change_background_opacity(move |position, _| {
                        let staged = paths
                            .iter()
                            .position(|path| path == position.path())
                            .is_some_and(|part| {
                                review.as_ref().is_some_and(|p| {
                                    p.line_staged(
                                        part,
                                        position.side() == DiffSide::Original,
                                        position.line(),
                                    )
                                })
                            });
                        if staged {
                            STAGED_CHANGE_BACKGROUND_OPACITY
                        } else {
                            1.0
                        }
                    })
                    .render_header(move |file, _, cx| {
                        let area = header_paths
                            .iter()
                            .position(|path| path == file.path())
                            .and_then(|part| header_review.as_ref()?.part_area(part));
                        div()
                            .px_3()
                            .py_1()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr(if area == Some(Area::Staged) {
                                "review_index"
                            } else {
                                "review_worktree"
                            }))
                    })
                })
                .soft_wrap(self.preferences.soft_wrap)
                .hunk_separator(DiffHunkSeparator::Wave)
                .size_full()
                .into_any_element()
        } else if self.view == View::Working && self.active_files().changes.is_empty() {
            self.message("clean_title", Some("clean_body"), cx)
        } else if self.view != View::Working && self.history.commits.is_empty() {
            if self.history.query.is_empty() {
                self.message("no_commits_title", Some("no_commits_body"), cx)
            } else {
                self.message("no_matching_commits", Some("no_matching_commits_body"), cx)
            }
        } else {
            self.message("no_files", None, cx)
        };
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .when(
                self.view != View::Repositories || self.repository_preview,
                |view| view.child(self.diff_header(window, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(content),
            )
            .when_some(
                self.current_commit()
                    .filter(|_| self.preferences.show_commit_message),
                |view, commit| {
                    view.child(
                        v_flex()
                            .id("commit-message")
                            .test_support()
                            .flex_none()
                            .max_h(px(200.))
                            .overflow_y_scroll()
                            .px_3()
                            .py_2()
                            .gap_1()
                            .border_t(px(1. / window.scale_factor()))
                            .border_color(cx.theme().border)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
                            )
                            .child(
                                div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                                    SelectableText::new(
                                        SharedString::from(format!("commit-subject-{}", commit.id)),
                                        commit.subject.clone(),
                                    )
                                    .document_order(0),
                                ),
                            )
                            .when(!commit.body.is_empty(), |view| {
                                view.child(
                                    div().text_sm().child(
                                        SelectableText::new(
                                            SharedString::from(format!(
                                                "commit-body-{}",
                                                commit.id
                                            )),
                                            commit.body.clone(),
                                        )
                                        .document_order(1),
                                    ),
                                )
                            })
                            .child(
                                h_flex()
                                    .gap_3()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.commit_id_button(commit, cx))
                                    .child(commit.author.clone())
                                    .child(commit.date.clone())
                                    .when(commit.parents.len() > 1, |row| {
                                        row.child(tr("first_parent"))
                                    }),
                            ),
                    )
                },
            )
            .into_any_element()
    }

    fn message(&self, title: &'static str, body: Option<&'static str>, cx: &App) -> AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_6()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(tr(title)),
            )
            .when_some(body, |view, body| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr(body)),
                )
            })
            .into_any_element()
    }
}

impl Drop for Patchlook {
    fn drop(&mut self) {
        self.cancel_repository_load();
        for repo in self.remote_sessions.values() {
            repo.disconnect();
        }
    }
}

impl Render for Patchlook {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.repository.is_none()
            && self.preferences.recent_repositories.is_empty()
            && self.view != View::Repositories
        {
            if self.loading {
                self.message("loading", None, cx)
            } else {
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(
                        Icon::new(IconName::GitBranch)
                            .size(px(36.))
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        v_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(tr("welcome_title")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr("welcome_body")),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("welcome-open")
                                    .primary()
                                    .label(tr("open_repository"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_repository(&OpenRepository, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("welcome-ssh")
                                    .outline()
                                    .label(tr("connect_ssh"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_remote_repository(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        Button::new("welcome-compare")
                            .ghost()
                            .label(tr("new_text_comparison"))
                            .on_click(|_, _, cx| {
                                cx.dispatch_action(&crate::text_compare::NewTextComparison)
                            }),
                    )
                    .into_any_element()
            }
        } else {
            let sidebar = if self.view == View::Repositories {
                let ratio = self.preferences.repository_sidebar_ratio.unwrap_or(0.5);
                v_resizable("repositories-sidebar")
                    .on_resize(cx.listener(
                        |this, state: &Entity<component::resizable::ResizableState>, _, cx| {
                            let sizes = state.read(cx).sizes();
                            if sizes.len() == 2 && sizes[0] + sizes[1] > px(0.) {
                                this.preferences.repository_sidebar_ratio =
                                    Some(sizes[0] / (sizes[0] + sizes[1]));
                                this.save_preferences();
                            }
                        },
                    ))
                    .child(
                        resizable_panel()
                            .size(px(0.))
                            .flex_grow(ratio)
                            .child(self.repositories(cx)),
                    )
                    .child(
                        resizable_panel()
                            .size(px(0.))
                            .flex_grow(1. - ratio)
                            .child(self.branch_list(cx)),
                    )
                    .into_any_element()
            } else if self.view == View::History {
                v_resizable("history-sidebar")
                    .child(resizable_panel().size(px(350.)).child(self.history(cx)))
                    .child(resizable_panel().overflow_hidden().child(self.files(cx)))
                    .into_any_element()
            } else {
                self.files(cx)
            };
            let detail = if self.view == View::Repositories {
                h_resizable("repository-history")
                    .child(
                        resizable_panel()
                            .size(px(320.))
                            .size_range(px(210.)..px(600.))
                            .child(self.history(cx)),
                    )
                    .child(
                        resizable_panel()
                            .overflow_hidden()
                            .child(self.detail(window, cx)),
                    )
                    .into_any_element()
            } else {
                self.detail(window, cx)
            };
            h_resizable("workspace")
                .child(
                    resizable_panel()
                        .size(px(300.))
                        .size_range(px(230.)..px(500.))
                        .fixed()
                        .child(
                            v_flex()
                                .size_full()
                                .min_h_0()
                                .bg(cx.theme().sidebar)
                                .child(self.sidebar_tabs(cx))
                                .child(div().flex_1().min_h_0().child(sidebar)),
                        ),
                )
                .child(resizable_panel().overflow_hidden().child(detail))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .text_size(cx.theme().font_size)
            .key_context("Patchlook")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open_repository))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::show_about))
            .child(self.toolbar(cx))
            .when_some(self.visible_error().cloned(), |view, error| {
                view.child(
                    h_flex()
                        .flex_none()
                        .gap_3()
                        .px_4()
                        .py_2()
                        .bg(cx.theme().danger.opacity(0.12))
                        .child(div().flex_1().text_sm().child(error))
                        .child(
                            Button::new("dismiss-error")
                                .icon(IconName::X)
                                .small()
                                .ghost()
                                .tooltip(tr("dismiss"))
                                .accessibility_label(tr("dismiss"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.error = None;
                                    this.active_files_mut().error = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(body),
            )
            .child(self.about.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strings::language_name;
    use core::prelude::v1::test;
    use gpui_kit::component::scroll::ScrollbarHandle;
    use gpui_kit::test::{TestQueryExt, TestWindowExt};
    use patchlook::git::Area;

    #[test]
    fn write_refresh_matches_full_refresh_with_many_unrelated_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "core.autocrlf", "false"]);
        for i in 0..20 {
            std::fs::write(root.join(format!("binary-{i:02}.bin")), vec![0; 65536]).unwrap();
        }
        for i in 0..15 {
            std::fs::write(
                root.join(format!("text-{i:02}.txt")),
                "one\nkeep\ntwo\nkeep\nthree\n",
            )
            .unwrap();
        }
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);
        for i in 0..15 {
            std::fs::write(
                root.join(format!("text-{i:02}.txt")),
                "ONE\nkeep\ntwo\nkeep\nthree\n",
            )
            .unwrap();
        }
        git(root, &["add", "-A"]);
        for i in 0..15 {
            std::fs::write(
                root.join(format!("text-{i:02}.txt")),
                "ONE\nkeep\nTWO\nkeep\nTHREE\n",
            )
            .unwrap();
        }
        for i in 0..37 {
            let mut bytes = vec![1; 65536];
            bytes[0] = 0;
            std::fs::write(root.join(format!("binary-{i:02}.bin")), bytes).unwrap();
        }
        let repo = Repository::open(root).unwrap();
        let raw = repo.working_changes().unwrap();
        let changes = merge_working_changes(&raw);
        assert_eq!(changes.len(), 52);
        let target = changes
            .iter()
            .position(|c| c.path == std::path::Path::new("text-14.txt"))
            .unwrap();
        let mut prepared = PreparedChanges::load(&repo, raw, None, Some(target)).unwrap();
        assert_eq!(
            prepared
                .stats
                .iter()
                .filter(|stats| stats.is_none())
                .count(),
            37
        );
        assert_eq!(prepared.progress[target], Some((1, 3)));
        let change = prepared.changes[target].clone();
        let review = repo.review_patch(&change).unwrap();
        let mut history = WriteHistory::default();
        let touched = history
            .record(
                &repo,
                AccessMode::ReadWrite,
                WriteScope::index(&[change]),
                || repo.set_review_block(AccessMode::ReadWrite, &review, 1),
            )
            .unwrap();
        for undo in [false, true] {
            let touched = if undo {
                history.undo(&repo, AccessMode::ReadWrite).unwrap()
            } else {
                touched.clone()
            };
            let start = Instant::now();
            prepared = PreparedChanges::after_write(&repo, prepared, &touched).unwrap();
            let selective_time = start.elapsed();
            let start = Instant::now();
            let complete =
                PreparedChanges::load(&repo, repo.working_changes().unwrap(), None, Some(target))
                    .unwrap();
            eprintln!(
                "52 files / 37 binary: write refresh {selective_time:?}, full refresh {:?}",
                start.elapsed()
            );
            assert_eq!(prepared.changes, complete.changes);
            assert_eq!(prepared.stats, complete.stats);
            assert_eq!(
                prepared.stats.iter().flatten().copied().sum::<DiffStats>(),
                repo.change_stats(&repo.working_changes().unwrap(), None)
                    .unwrap()
            );
            assert_eq!(prepared.progress, complete.progress);
            assert_eq!(prepared.preview_index, target);
            assert_eq!(
                prepared.progress[target],
                Some((if undo { 1 } else { 2 }, 3))
            );
            assert!(prepared.preview.as_ref().unwrap().is_ok());
        }
    }

    #[test]
    fn reused_review_counts_match_git_for_text_and_nontext_changes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "core.autocrlf", "false"]);
        for (name, contents) in [
            ("third.txt", "old\ncontext\nend"),
            ("deleted.txt", "deleted\n"),
            ("renamed.txt", "rename only\n"),
            ("binary.bin", "\0old"),
        ] {
            std::fs::write(root.join(name), contents).unwrap();
        }
        git(root, &["add", "-A"]);
        git(root, &["commit", "-m", "base"]);
        std::fs::write(root.join("third.txt"), "index\ncontext\nend").unwrap();
        git(root, &["add", "third.txt"]);
        git(root, &["mv", "renamed.txt", "renamed [new].txt"]);
        std::fs::remove_file(root.join("deleted.txt")).unwrap();
        for (name, contents) in [
            ("third.txt", "worktree\ncontext\nEND"),
            ("binary.bin", "\0new"),
            ("new.txt", "+++ header-like text\r\n--- still text"),
            ("empty.txt", ""),
        ] {
            std::fs::write(root.join(name), contents).unwrap();
        }
        let repo = Repository::open(root).unwrap();
        let raw = repo.working_changes().unwrap();
        let merged = merge_working_changes(&raw);
        let raw_stats = repo.file_stats(&raw, None).unwrap();
        let expected: Vec<_> = merged
            .iter()
            .map(|file| {
                let mut total = DiffStats::default();
                for (change, stats) in raw.iter().zip(&raw_stats) {
                    if change.path == file.path {
                        total += (*stats)?;
                    }
                }
                Some(total)
            })
            .collect();
        for preview in std::iter::once(None).chain((0..merged.len()).map(Some)) {
            let prepared = PreparedChanges::load(&repo, raw.clone(), None, preview).unwrap();
            assert_eq!(prepared.stats, expected, "preview {preview:?}");
            if let Some(preview) = prepared.preview {
                assert!(preview.is_ok());
            }
        }
    }

    #[test]
    fn failed_initial_preview_does_not_discard_the_file_list() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(directory.path())
                .args(["init", "-b", "main"])
                .output()
                .unwrap()
                .status
                .success()
        );
        std::fs::write(directory.path().join("broken.png"), "not an image").unwrap();
        let repository = Repository::open(directory.path()).unwrap();
        let prepared = PreparedChanges::load(
            &repository,
            vec![Change {
                path: "broken.png".into(),
                previous_path: None,
                status: 'A',
                area: Area::Untracked,
            }],
            None,
            Some(0),
        )
        .unwrap();
        assert_eq!(prepared.changes.len(), 1);
        assert!(prepared.preview.unwrap().is_err());
    }

    fn repository_sidebar_ratio(window: &mut Window) -> f32 {
        let top = window.find("repositories").bounds().bottom();
        let divider = window
            .within("repositories-sidebar")
            .find(("resizable-handle", 0usize))
            .bounds()
            .center()
            .y;
        (divider - top) / (window.viewport_size().height - top)
    }

    #[test]
    fn search_highlights_preserve_case_insensitive_utf8_ranges() {
        for (text, query, expected) in [
            ("Fix FIX fixes", "fix", vec!["Fix", "FIX", "fix"]),
            ("更新中文，中文说明", "中文", vec!["中文", "中文"]),
            ("ÉCOLE école", "école", vec!["ÉCOLE", "école"]),
            ("İstanbul", "i", vec!["İ"]),
            ("ΟΣ ος", "ος", vec!["ΟΣ", "ος"]),
            ("literal [.] match", "[.]", vec!["[.]"]),
            ("no matches", " ", vec![]),
        ] {
            let ranges = search_match_ranges(text, query);
            assert_eq!(
                ranges
                    .iter()
                    .map(|range| &text[range.clone()])
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                ranges.is_empty(),
                query.trim().is_empty()
                    || !text.to_lowercase().contains(&query.trim().to_lowercase())
            );
        }
    }

    fn test_app(window: &mut Window, cx: &mut Context<Patchlook>) -> Patchlook {
        Patchlook {
            access_mode: AccessMode::default(),
            write_task: None,
            write_history: WriteHistory::default(),
            working: ChangeState::new(Layout::Split, cx),
            history: HistoryState::new(Layout::Split, cx),
            preferences: Preferences::default(),
            about: cx.new(crate::about::AboutPanel::new),
            host_verification: crate::ssh_host::HostVerification::new(window, cx),
            external_files: Vec::new(),
            preferences_directory: tempfile::tempdir().unwrap(),
            repository: None,
            remote_sessions: HashMap::new(),
            selected_repository: None,
            pending_connection: None,
            remote_errors: HashMap::new(),
            branch: "main".into(),
            branches: vec![],
            repository_scroll: ScrollHandle::new(),
            collapsed_repository_groups: [false; 2],
            collapsed_repositories: HashSet::new(),
            branch_scroll: UniformListScrollHandle::new(),
            view: View::Working,
            repository_preview: false,

            history_search: cx.new(|cx| InputState::new(window, cx)),

            commit_copied: false,
            copy_feedback_task: None,
            loading: false,

            error: None,
            load_task: None,
            _subscriptions: vec![],
            focus: cx.focus_handle(),
        }
    }

    #[gpui_kit::test]
    fn returning_to_the_current_repository_cancels_an_unfinished_switch(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for directory in [&first, &second] {
            git(directory.path(), &["init", "-b", "main"]);
        }
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.load(first.path().to_owned(), cx);
                    app
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        let original = cx
            .update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| {
                    assert!(app.error.is_none(), "{:?}", app.error);
                    let original = app.repository.as_ref().unwrap().location.clone();
                    app.load(second.path().to_owned(), cx);
                    assert!(app.loading);
                    app.open_saved_repository(original.clone(), window, cx);
                    assert!(!app.loading);
                    assert!(app.load_task.is_none());
                    original
                })
            })
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app.read(cx);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert_eq!(app.repository.as_ref().unwrap().location, original);
            assert_eq!(app.selected_repository.as_ref(), Some(&original));
        });
    }

    #[gpui_kit::test]
    fn repository_groups_hide_single_kind_headers_and_collapse_independently(
        cx: &mut TestAppContext,
    ) {
        use patchlook::git::SshOptions;
        cx.update(gpui_kit::init);
        let local = RepositoryLocation::Local(PathBuf::from("alpha"));
        let remote = RepositoryLocation::Ssh {
            path: "/srv/beta".into(),
            options: SshOptions {
                host: "example".into(),
                ..Default::default()
            },
        };
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.view = View::Repositories;
                    app
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("local-repositories").is_none());
            assert!(window.try_find("remote-repositories").is_none());
            for path in [&local, &remote] {
                app.update(cx, |app, cx| {
                    app.preferences.recent_repositories.clear();
                    app.preferences.add_repository(path.clone());
                    cx.notify();
                });
                window.render_frame(cx);
                assert!(window.try_find("local-repositories").is_none());
                assert!(window.try_find("remote-repositories").is_none());
                assert!(window.find(("repository", 0usize)).visible());
            }
            app.update(cx, |app, cx| {
                app.preferences.add_repository(local.clone());
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find("local-repositories").visible());
            assert!(window.find("remote-repositories").visible());
            window.click("local-repositories", cx);
            window.render_frame(cx);
            assert!(window.try_find(("repository", 0usize)).is_none());
            assert!(window.find(("repository", 1usize)).visible());
            window.click("remote-repositories", cx);
            window.render_frame(cx);
            assert!(window.try_find(("repository", 1usize)).is_none());
            window.click("local-repositories", cx);
            window.render_frame(cx);
            assert!(window.find(("repository", 0usize)).visible());
            assert!(window.try_find(("repository", 1usize)).is_none());
            window.click("local-repositories", cx);
            // A collapsed group must remain reachable when it becomes the only kind.
            for path in [&local, &remote] {
                app.update(cx, |app, cx| {
                    app.preferences.recent_repositories.clear();
                    app.preferences.add_repository(path.clone());
                    cx.notify();
                });
                window.render_frame(cx);
                assert!(window.try_find("local-repositories").is_none());
                assert!(window.try_find("remote-repositories").is_none());
                assert!(window.find(("repository", 0usize)).visible());
            }
            assert!(app.read(cx).load_task.is_none());
            assert!(!app.read(cx).loading);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn submodule_rows_collapse_and_switch_between_independent_repositories(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let directory = tempfile::tempdir().unwrap();
        let parent_path = directory.path();
        let child_path = parent_path.join("deps/child");
        std::fs::create_dir_all(&child_path).unwrap();
        git(parent_path, &["init", "-b", "main"]);
        git(&child_path, &["init", "-b", "child-branch"]);
        std::fs::write(child_path.join("child.txt"), "child content\n").unwrap();
        git(&child_path, &["add", "child.txt"]);
        git(&child_path, &["commit", "-m", "Child commit"]);
        let child = Repository::open(&child_path).unwrap();
        let oid = child.history(0, None).unwrap().commits[0].id.clone();
        for path in ["deps/child", "missing"] {
            git(
                parent_path,
                &["update-index", "--add", "--cacheinfo", "160000", &oid, path],
            );
        }
        git(parent_path, &["commit", "-m", "Parent commit"]);
        std::fs::write(child_path.join("untracked.txt"), "child only\n").unwrap();
        let parent = Repository::open(parent_path).unwrap();
        let missing = parent
            .location
            .join(std::path::Path::new("missing"))
            .unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.view = View::Repositories;
                    app
                })
            })
            .unwrap()
        });
        cx.update(|cx| app.update(cx, |app, cx| app.load(parent.location.clone(), cx)));
        cx.run_until_parked();
        let index = |app: &Patchlook, path: &RepositoryLocation| {
            app.preferences
                .recent_repositories
                .iter()
                .position(|e| &e.path == path)
                .unwrap()
        };
        cx.update_window(handle, |_, window, cx| {
            assert!(app.read(cx).error.is_none(), "{:?}", app.read(cx).error);
            assert_eq!(app.read(cx).preferences.recent_repositories.len(), 3);
            let parent_index = index(app.read(cx), &parent.location);
            let child_index = index(app.read(cx), &child.location);
            let missing_index = index(app.read(cx), &missing);
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(
                window.find(("repository", missing_index)).disabled(),
                Some(true)
            );
            assert_eq!(
                window.find(("repository-status", missing_index)).label(),
                Some(tr("submodule_uninitialized"))
            );
            assert!(
                window.find(("repository", child_index)).bounds().left()
                    > window.find(("repository", parent_index)).bounds().left()
            );
            window.click(("repository-expand", parent_index), cx);
            window.render_frame(cx);
            assert!(window.try_find(("repository", child_index)).is_none());
            window.click(("repository-expand", parent_index), cx);
            window.render_frame(cx);
            window.click(("repository", child_index), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(app.read(cx).error.is_none(), "{:?}", app.read(cx).error);
            assert_eq!(
                app.read(cx).repository.as_ref().unwrap().location,
                child.location
            );
            assert_eq!(app.read(cx).branch, "child-branch");
            assert_eq!(app.read(cx).history.commits[0].subject, "Child commit");
            window.render_frame(cx);
            assert_eq!(window.find("remove-repository").disabled(), Some(true));
            app.update(cx, |app, cx| app.switch_view(View::Working, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(app.read(cx).working.changes.len(), 1);
            assert_eq!(
                app.read(cx).working.changes[0].path,
                PathBuf::from("untracked.txt")
            );
            app.update(cx, |app, cx| app.switch_view(View::Repositories, cx));
            window.render_frame(cx);
            window.render_frame(cx);
            window.click(("repository", index(app.read(cx), &parent.location)), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let state = app.read(cx);
            assert!(state.error.is_none(), "{:?}", state.error);
            assert_eq!(state.repository.as_ref().unwrap().location, parent.location);
            assert_eq!(state.history.commits[0].subject, "Parent commit");
            assert_eq!(state.preferences.recent_repositories.len(), 3);
        });
    }

    #[gpui_kit::test]
    fn remote_repository_rows_show_pending_failure_and_connection_actions(cx: &mut TestAppContext) {
        use patchlook::git::{SshAuthentication, SshOptions};
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        let local = Repository::open(directory.path()).unwrap();
        let remote = RepositoryLocation::Ssh {
            path: "/srv/demo".into(),
            options: SshOptions {
                host: "server.example".into(),
                authentication: SshAuthentication::Password,
                ..Default::default()
            },
        };
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.preferences.remember_repository(local.location.clone());
                    app.repository = Some(local.clone());
                    app.selected_repository = Some(local.location.clone());
                    app.view = View::Repositories;
                    app.collapsed_repository_groups[1] = true;
                    app
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.load_repository(remote.clone(), None, cx));
            assert!(app.read(cx).loading);
            assert_eq!(app.read(cx).preferences.recent_repositories.len(), 2);
            window.render_frame(cx);
            window.render_frame(cx);
            let remote_index = app
                .read(cx)
                .preferences
                .recent_repositories
                .iter()
                .position(|e| e.path == remote)
                .unwrap();
            let local_index = 1 - remote_index;
            assert!(
                window.find("local-repositories").bounds().top()
                    < window.find(("repository", local_index)).bounds().top()
            );
            assert!(
                window.find(("repository", local_index)).bounds().bottom()
                    <= window.find("remote-repositories").bounds().top()
            );
            assert!(
                window.find("remote-repositories").bounds().bottom()
                    <= window.find(("repository", remote_index)).bounds().top()
            );
            assert_eq!(
                window.find(("repository-status", remote_index)).label(),
                Some(tr("ssh_connecting"))
            );
            let control = app.read(cx).pending_connection.as_ref().unwrap().1.clone();
            window.hover(("repository", remote_index), cx);
            window.click(("repository-more", remote_index), cx);
            window.render_frame(cx);
            window.render_frame(cx);
            let action = window
                .find_by_label(tr("ssh_disconnect"))
                .path()
                .last()
                .unwrap()
                .clone();
            window.click(action, cx);
            assert!(control.is_disconnected());
            assert!(!app.read(cx).loading);
            assert_eq!(app.read(cx).remote_status(&remote), "ssh_disconnected");
            assert_eq!(
                app.read(cx).repository.as_ref().unwrap().location,
                local.location
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            assert!(app.read(cx).error.is_none());
            // Missing credentials fail before launching SSH; no network is used by this test.
            app.update(cx, |app, cx| app.load_repository(remote.clone(), None, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(!app.read(cx).loading);
            assert_eq!(app.read(cx).remote_status(&remote), "ssh_connection_failed");
            assert_eq!(app.read(cx).preferences.recent_repositories.len(), 2);
            assert_eq!(
                app.read(cx).repository.as_ref().unwrap().location,
                local.location
            );
            window.render_frame(cx);
            let index = app
                .read(cx)
                .preferences
                .recent_repositories
                .iter()
                .position(|e| e.path == remote)
                .unwrap();
            window.hover(("repository", index), cx);
            window.click(("repository-more", index), cx);
            window.render_frame(cx);
            window.render_frame(cx);
            let action = window
                .find_by_label(tr("ssh_reconnect"))
                .path()
                .last()
                .unwrap()
                .clone();
            window.click(action, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find("ssh-form").visible());
            assert_eq!(window.find("ssh-connect").disabled(), Some(true));
            window.click("ssh-cancel", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("remove-repository", cx);
            assert_eq!(app.read(cx).preferences.recent_repositories.len(), 1);
            assert_eq!(
                app.read(cx).repository.as_ref().unwrap().location,
                local.location
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn startup_keeps_saved_ssh_repositories_disconnected(cx: &mut TestAppContext) {
        use patchlook::git::{SshAuthentication, SshOptions};
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        for authentication in [
            SshAuthentication::Password,
            SshAuthentication::KeyWithPassphrase,
            SshAuthentication::Key,
        ] {
            let saved = RepositoryLocation::Ssh {
                path: "/srv/saved".into(),
                options: SshOptions {
                    host: "saved.example".into(),
                    authentication,
                    ..Default::default()
                },
            };
            let (handle, app) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        let mut preferences = Preferences::default();
                        preferences.remember_repository(saved.clone());
                        Patchlook::from_preferences(None, preferences, None, window, cx)
                    })
                })
                .unwrap()
            });
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                let state = app.read(cx);
                assert!(state.view == View::Repositories);
                assert_eq!(state.selected_repository.as_ref(), Some(&saved));
                assert_eq!(state.preferences.recent_repositories.len(), 1);
                assert!(state.repository.is_none());
                assert!(state.load_task.is_none());
                assert!(state.pending_connection.is_none());
                assert!(!state.loading);
                assert!(state.error.is_none());
                assert!(window.try_find("ssh-form").is_none());
                assert!(window.try_find("ssh-host-verification").is_none());
                assert_eq!(
                    window.find(("repository-status", 0usize)).label(),
                    Some(tr("ssh_disconnected"))
                );
                if authentication.requires_secret() {
                    window.click(("repository", 0usize), cx);
                    window.render_frame(cx);
                    window.render_frame(cx);
                    assert!(window.find("ssh-form").visible());
                    window.click("ssh-cancel", cx);
                }
                window.remove_window();
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn startup_explicit_ssh_target_opens_after_the_window_root_is_ready(cx: &mut TestAppContext) {
        use patchlook::git::{SshAuthentication, SshOptions};
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        for authentication in [
            SshAuthentication::Password,
            SshAuthentication::KeyWithPassphrase,
        ] {
            let explicit = RepositoryLocation::Ssh {
                path: "/srv/explicit".into(),
                options: SshOptions {
                    host: "explicit.example".into(),
                    authentication,
                    ..Default::default()
                },
            };
            let (handle, app) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        Patchlook::from_preferences(
                            Some(explicit.clone()),
                            Preferences {
                                repository: Some(PathBuf::from("unused-local-path").into()),
                                ..Preferences::default()
                            },
                            None,
                            window,
                            cx,
                        )
                    })
                })
                .unwrap()
            });
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                assert!(window.find("ssh-form").visible());
                assert_eq!(app.read(cx).selected_repository.as_ref(), Some(&explicit));
                assert!(!app.read(cx).loading);
                assert!(app.read(cx).repository.is_none());
                assert!(app.read(cx).error.is_none());
                window.click("ssh-cancel", cx);
                window.remove_window();
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn startup_still_restores_local_repositories_and_honors_explicit_paths(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let paths = ["saved", "explicit"].map(|name| {
            let path = directory.path().join(name);
            std::fs::create_dir(&path).unwrap();
            git(&path, &["init", "-b", "main"]);
            Repository::open(&path).unwrap().location
        });
        for initial in [None, Some(paths[1].clone())] {
            let expected = initial.as_ref().unwrap_or(&paths[0]).clone();
            let (handle, app) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        Patchlook::from_preferences(
                            initial,
                            Preferences {
                                repository: Some(paths[0].clone()),
                                ..Preferences::default()
                            },
                            None,
                            window,
                            cx,
                        )
                    })
                })
                .unwrap()
            });
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_eq!(app.read(cx).repository.as_ref().unwrap().location, expected);
                assert!(app.read(cx).working.loaded);
                assert!(app.read(cx).error.is_none());
                window.render_frame(cx);
                assert!(window.try_find("ssh-form").is_none());
                window.remove_window();
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn ssh_input_history_filters_selects_and_removes_without_changing_the_input(
        cx: &mut TestAppContext,
    ) {
        use patchlook::{git::SshOptions, preferences::SshHistoryField};
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| test_app(window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, _| window.activate_window())
            .unwrap();
        cx.run_until_parked();
        let form = cx
            .update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| {
                    for host in ["alpha.example", "beta.example"] {
                        let target = RepositoryLocation::Ssh {
                            path: "/srv/demo".into(),
                            options: SshOptions {
                                host: host.into(),
                                port: Some(2222),
                                username: Some("review".into()),
                                ..Default::default()
                            },
                        };
                        app.preferences.ssh_input_history.remember(&target);
                    }
                    app.save_preferences();
                    app.open_remote_repository(window, cx)
                })
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                form.read(cx).host.focus_handle(cx).is_focused(window),
                "host should initially have focus"
            );
            let menu = window.find("ssh-history-menu").bounds();
            let input = window.find("ssh-host").bounds();
            assert!(menu.top() >= input.bottom());
            assert!(menu.size.width > px(100.));
            assert!(window.find(("ssh-history-value", 1usize)).visible());
            window.input("alpha", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            assert!(window.try_find(("ssh-history-value", 1usize)).is_none());
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                form.read(cx).host.read(cx).value().as_str(),
                "alpha.example"
            );
            assert!(window.try_find("ssh-history-menu").is_none());
            assert!(!app.read(cx).loading);
            window.click(("input", form.read(cx).host.entity_id()), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("ssh-history-remove", 0usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            assert_eq!(
                form.read(cx).host.read(cx).value().as_str(),
                "alpha.example"
            );
            assert!(window.try_find("ssh-history-menu").is_none());
            let app = app.read(cx);
            assert_eq!(
                app.preferences
                    .ssh_input_history
                    .values(SshHistoryField::Host),
                &["beta.example"]
            );
            let persisted: Preferences = serde_json::from_slice(
                &std::fs::read(app.preferences_directory.path().join("preferences.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(
                persisted.ssh_input_history.values(SshHistoryField::Host),
                &["beta.example"]
            );
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-a"
                } else {
                    "ctrl-a"
                },
                cx,
            );
            window.press("backspace", cx);
            assert!(form.read(cx).host.read(cx).value().is_empty());
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(form.read(cx).host.focus_handle(cx).is_focused(window));
            assert!(window.find("ssh-history-menu").visible());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("ssh-history-menu").is_none());
            assert!(window.find("ssh-form").visible());
            form.read(cx).username.focus_handle(cx).focus(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(("ssh-history-value", 0usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(form.read(cx).username.read(cx).value().as_str(), "review");
            form.read(cx).secret.focus_handle(cx).focus(window, cx);
            window.input("never-save-this", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("ssh-history-menu").is_none());
            window.click("ssh-cancel", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                let stored =
                    std::fs::read(app.preferences_directory.path().join("preferences.json"))
                        .unwrap();
                assert!(!String::from_utf8_lossy(&stored).contains("never-save-this"));
                app.preferences = serde_json::from_slice(&stored).unwrap();
                app.open_remote_repository(window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find(("ssh-history-value", 0usize)).visible());
            assert!(window.try_find(("ssh-history-value", 1usize)).is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn ssh_dialog_validates_inputs_live_and_can_be_cancelled(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| test_app(window, cx))
            })
            .unwrap()
        });
        let form = cx
            .update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| app.open_remote_repository(window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(window.find("ssh-connect").disabled(), Some(true));
            window.click("ssh-connect", cx);
            assert!(form.read(cx).error.is_none());
            assert!(!app.read(cx).loading);
        })
        .unwrap();
        let assert_disabled = |cx: &mut TestAppContext, disabled| {
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                // Process input notifications without forcing a full window refresh.
                window.draw(cx).clear(cx);
                assert_eq!(
                    window.find("ssh-connect").disabled() == Some(true),
                    disabled
                );
            })
            .unwrap();
        };
        let inputs = cx.update(|cx| {
            let form = form.read(cx);
            [
                (form.host.clone(), "127.0.0.1", true),
                (form.path.clone(), "/srv/demo", true),
                (form.secret.clone(), "test-password", false),
                (form.port.clone(), "0", true),
                (form.port.clone(), "65536", true),
                (form.port.clone(), "invalid", true),
                (form.port.clone(), "2222", false),
                (form.port.clone(), "", false),
                (form.host.clone(), "", true),
                (form.host.clone(), "example.com", false),
                (form.path.clone(), "relative/path", true),
                (form.path.clone(), "/srv/demo", false),
                (form.secret.clone(), "", true),
            ]
        });
        for (input, value, disabled) in inputs {
            cx.update_window(handle, |_, window, cx| {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            })
            .unwrap();
            assert_disabled(cx, disabled);
        }
        cx.update_window(handle, |_, window, cx| {
            // Password is required, while username and port may use SSH defaults.
            form.read(cx).secret.focus_handle(cx).focus(window, cx);
            window.input("test-password", cx);
        })
        .unwrap();
        assert_disabled(cx, false);
        cx.update_window(handle, |_, window, cx| {
            window.click("ssh-auth-key", cx);
        })
        .unwrap();
        assert_disabled(cx, false);
        cx.update_window(handle, |_, window, cx| {
            // An agent/default key needs no secret, but a passphrase needs a key file.
            form.read(cx).secret.clone().update(cx, |input, cx| {
                input.set_value("key-passphrase", window, cx)
            });
        })
        .unwrap();
        assert_disabled(cx, true);
        let key = tempfile::NamedTempFile::new().unwrap();
        cx.update_window(handle, |_, window, cx| {
            form.read(cx).identity.clone().update(cx, |input, cx| {
                input.set_value(key.path().display().to_string(), window, cx)
            });
        })
        .unwrap();
        assert_disabled(cx, false);
        cx.update_window(handle, |_, window, cx| {
            window.click("ssh-auth-password", cx);
        })
        .unwrap();
        assert_disabled(cx, true);
        cx.update_window(handle, |_, window, cx| {
            assert!(form.read(cx).connection(cx).is_err());
            assert!(!app.read(cx).loading);
            window.click("ssh-cancel", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("ssh-form").is_none());
            assert!(app.read(cx).repository.is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn ssh_auth_form_keeps_secrets_out_of_saved_connections(cx: &mut TestAppContext) {
        use patchlook::git::SshAuthentication;
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| test_app(window, cx))
            })
            .unwrap()
        });
        let form = cx
            .update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| app.open_remote_repository(window, cx))
            })
            .unwrap();
        cx.run_until_parked();
        let target = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                assert!(window.try_find("ssh-choose-config").is_none());
                assert!(window.try_find("ssh-choose-key").is_none());
                assert!(form.read(cx).secret.read(cx).presentation().is_masked());
                for (input, value) in [
                    (form.read(cx).host.clone(), "127.0.0.1"),
                    (form.read(cx).port.clone(), "2222"),
                    (form.read(cx).username.clone(), "review"),
                    (form.read(cx).path.clone(), "/srv/demo"),
                    (form.read(cx).secret.clone(), "session-only-secret"),
                ] {
                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }
                for masked in [false, true, false] {
                    window.render_frame(cx);
                    window.click("toggle-mask", cx);
                    assert_eq!(
                        form.read(cx).secret.read(cx).presentation().is_masked(),
                        masked
                    );
                    assert_eq!(form.read(cx).secret.read(cx).value(), "session-only-secret");
                }
                let (target, secret) = form.read(cx).connection(cx).unwrap();
                assert!(secret.is_some());
                let RepositoryLocation::Ssh { options, .. } = &target else {
                    unreachable!()
                };
                assert_eq!(options.port, Some(2222));
                assert_eq!(options.username.as_deref(), Some("review"));
                assert_eq!(options.authentication, SshAuthentication::Password);
                let mut saved = Preferences::default();
                saved.remember_repository(target.clone());
                assert!(
                    !serde_json::to_string(&saved)
                        .unwrap()
                        .contains("session-only-secret")
                );
                window.click("ssh-auth-key", cx);
                assert!(form.read(cx).secret.read(cx).value().is_empty());
                assert!(form.read(cx).secret.read(cx).presentation().is_masked());
                form.read(cx).secret.clone().update(cx, |input, cx| {
                    input.set_value("key-passphrase", window, cx);
                });
                window.render_frame(cx);
                window.click("toggle-mask", cx);
                assert!(!form.read(cx).secret.read(cx).presentation().is_masked());
                assert_eq!(form.read(cx).secret.read(cx).value(), "key-passphrase");
                window.render_frame(cx);
                assert!(window.find("ssh-choose-key").visible());
                window.click("ssh-advanced", cx);
                window.render_frame(cx);
                assert!(window.find("ssh-choose-config").visible());
                form.read(cx)
                    .port
                    .clone()
                    .update(cx, |input, cx| input.set_value("65536", window, cx));
                assert!(form.read(cx).connection(cx).is_err());
                window.click("ssh-cancel", cx);
                target
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            // Selecting a saved password connection prompts again before any I/O.
            app.update(cx, |app, cx| {
                app.open_saved_repository(target.clone(), window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find("ssh-form").visible());
            assert!(!app.read(cx).loading);
            window.click("ssh-connect", cx);
            assert!(!app.read(cx).loading);
            assert!(app.read(cx).repository.is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    #[ignore = "requires the isolated SSH fixture"]
    fn ssh_submodule_navigation_reuses_parent_and_disconnects_family(cx: &mut TestAppContext) {
        use patchlook::{
            git::{SshAuthentication, SshOptions},
            ssh_auth::SshSecret,
        };
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let parent = RepositoryLocation::Ssh {
            path: "/home/review/repos/modules".into(),
            options: SshOptions {
                host: "127.0.0.1".into(),
                port: Some(2222),
                username: Some("review".into()),
                authentication: SshAuthentication::Password,
                config_file: Some(
                    std::env::var_os("PATCHLOOK_TEST_SSH_CONFIG")
                        .unwrap()
                        .into(),
                ),
                ..Default::default()
            },
        };
        let child = parent
            .join(std::path::Path::new("deps/子模块 [one]"))
            .unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.view = View::Repositories;
                    app
                })
            })
            .unwrap()
        });
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                let secret = SshSecret::new(
                    "patchlook-test".into(),
                    std::env::var_os("PATCHLOOK_TEST_SSH_HELPER")
                        .unwrap()
                        .into(),
                )
                .unwrap();
                app.load_repository(parent.clone(), Some(secret), cx);
            })
        });
        cx.run_until_parked();
        for target in [&child, &parent, &child] {
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| {
                    assert!(app.error.is_none(), "{:?}", app.error);
                    app.open_saved_repository(target.clone(), window, cx);
                    assert!(app.pending_connection.is_none());
                });
                window.render_frame(cx);
                assert!(window.try_find("ssh-form").is_none());
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                let state = app.read(cx);
                assert!(state.error.is_none(), "{:?}", state.error);
                assert_eq!(&state.repository.as_ref().unwrap().location, target);
                assert_eq!(state.access_mode, AccessMode::ReadOnly);
            });
        }
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                assert!(
                    app.remote_sessions[&parent].shares_connection(&app.remote_sessions[&child])
                );
                app.disconnect_repository(&parent, window, cx);
                assert!(app.repository.is_none());
                assert!(app.remote_sessions.is_empty());
                assert_eq!(app.remote_status(&child), "ssh_disconnected");
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    #[ignore = "requires the isolated SSH fixture"]
    fn ssh_sessions_survive_repository_switches_until_explicit_disconnect(cx: &mut TestAppContext) {
        use patchlook::{
            git::{SshAuthentication, SshOptions},
            ssh_auth::SshSecret,
        };
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let config = PathBuf::from(std::env::var_os("PATCHLOOK_TEST_SSH_CONFIG").unwrap());
        let helper = PathBuf::from(std::env::var_os("PATCHLOOK_TEST_SSH_HELPER").unwrap());
        let state = config.parent().unwrap();
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        let local = Repository::open(directory.path()).unwrap().location;
        for (authentication, identity_file, credential) in [
            (SshAuthentication::Password, None, Some("patchlook-test")),
            (SshAuthentication::Key, Some(state.join("id_ed25519")), None),
            (
                SshAuthentication::KeyWithPassphrase,
                Some(state.join("id_ed25519_encrypted")),
                Some("patchlook-test-key"),
            ),
        ] {
            let options = SshOptions {
                host: "127.0.0.1".into(),
                port: Some(2222),
                username: Some("review".into()),
                authentication,
                identity_file,
                config_file: Some(config.clone()),
            };
            let target = RepositoryLocation::Ssh {
                path: "/home/review/repos/demo".into(),
                options: options.clone(),
            };
            let other = RepositoryLocation::Ssh {
                path: "/home/review/repos/unborn".into(),
                options,
            };
            let secret =
                || credential.map(|value| SshSecret::new(value.into(), helper.clone()).unwrap());
            let (handle, app) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| test_app(window, cx))
                })
                .unwrap()
            });
            cx.update(|cx| {
                app.update(cx, |app, cx| {
                    app.load_repository(target.clone(), secret(), cx);
                })
            });
            cx.run_until_parked();
            let original = cx.update(|cx| {
                let app = app.read(cx);
                assert!(app.error.is_none(), "{:?}", app.error);
                assert_eq!(app.remote_status(&target), "ssh_connected");
                app.repository.clone().unwrap()
            });
            // Repeatedly switch back through the same action used by Recent rows.
            // No password/passphrase is supplied after the first successful connection.
            for _ in 0..2 {
                cx.update(|cx| app.update(cx, |app, cx| app.load(local.clone(), cx)));
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| {
                    app.update(cx, |app, cx| {
                        assert!(app.error.is_none(), "{:?}", app.error);
                        assert_eq!(app.repository.as_ref().unwrap().location, local);
                        assert_eq!(app.remote_status(&target), "ssh_connected");
                        assert_eq!(original.branch_name().unwrap(), "main");
                        app.open_saved_repository(target.clone(), window, cx);
                        assert!(app.loading);
                        assert!(app.pending_connection.is_none());
                    });
                    window.render_frame(cx);
                    assert!(window.try_find("ssh-form").is_none());
                })
                .unwrap();
                cx.run_until_parked();
                cx.update(|cx| {
                    let app = app.read(cx);
                    assert!(app.error.is_none(), "{:?}", app.error);
                    assert_eq!(app.repository.as_ref().unwrap().location, target);
                    assert!(app.working.loaded);
                    assert_eq!(app.remote_sessions.len(), 1);
                });
            }
            // A second remote repository and a failed switch leave the first session alive.
            cx.update(|cx| {
                app.update(cx, |app, cx| {
                    app.load_repository(other.clone(), secret(), cx);
                })
            });
            cx.run_until_parked();
            let second = cx.update(|cx| {
                let app = app.read(cx);
                assert!(app.error.is_none(), "{:?}", app.error);
                assert_eq!(app.remote_sessions.len(), 2);
                assert_eq!(app.remote_status(&target), "ssh_connected");
                assert_eq!(app.remote_status(&other), "ssh_connected");
                app.repository.clone().unwrap()
            });
            cx.update(|cx| {
                app.update(cx, |app, cx| {
                    app.load(directory.path().join("missing"), cx);
                })
            });
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| {
                    assert!(app.error.is_some());
                    assert_eq!(app.remote_sessions.len(), 2);
                    assert_eq!(app.repository.as_ref().unwrap().location, other);
                    app.open_saved_repository(target.clone(), window, cx);
                });
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| app.update(cx, |app, cx| app.load(local.clone(), cx)));
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| {
                    assert!(app.error.is_none(), "{:?}", app.error);
                    app.disconnect_repository(&target, window, cx);
                    assert_eq!(app.repository.as_ref().unwrap().location, local);
                    assert_eq!(app.remote_status(&target), "ssh_disconnected");
                    assert_eq!(app.remote_status(&other), "ssh_connected");
                    assert!(original.branch_name().is_err());
                    assert_eq!(second.branch_name().unwrap(), "main");
                    if authentication.requires_secret() {
                        app.open_saved_repository(target.clone(), window, cx);
                        assert!(!app.loading);
                    }
                });
                if authentication.requires_secret() {
                    window.render_frame(cx);
                    window.render_frame(cx);
                    assert!(window.find("ssh-form").visible());
                    window.click("ssh-cancel", cx);
                }
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                app.update(cx, |app, cx| {
                    app.load_repository(target.clone(), secret(), cx);
                })
            });
            cx.run_until_parked();
            let reconnected = cx.update(|cx| {
                let app = app.read(cx);
                assert!(app.error.is_none(), "{:?}", app.error);
                assert_eq!(app.remote_sessions.len(), 2);
                let saved = serde_json::to_string(&app.preferences).unwrap();
                assert!(!saved.contains("patchlook-test"));
                app.repository.clone().unwrap()
            });
            // Closing the window tears down both the visible and background sessions,
            // even when an in-flight reader still owns another Repository handle.
            let weak = app.downgrade();
            cx.update_window(handle, |_, window, _| window.remove_window())
                .unwrap();
            cx.update(|_| drop(app));
            cx.run_until_parked();
            assert!(weak.upgrade().is_none());
            assert!(reconnected.branch_name().is_err());
            assert!(second.branch_name().is_err());
        }
    }

    #[gpui_kit::test]
    #[ignore = "requires the isolated SSH fixture"]
    fn ssh_dialog_loads_readonly_snapshots_and_preserves_local_workflows(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let config = std::env::var("PATCHLOOK_TEST_SSH_CONFIG").unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| test_app(window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.access_mode = AccessMode::ReadWrite;
                let form = app.open_remote_repository(window, cx);
                form.update(cx, |form, _| {
                    form.authentication = patchlook::git::SshAuthentication::Key
                });
                for (input, value) in [
                    (form.read(cx).host.clone(), "patchlook-test"),
                    (form.read(cx).path.clone(), "/home/review/repos/demo"),
                    (form.read(cx).config.clone(), config.as_str()),
                ] {
                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.click("ssh-connect", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.error.is_none(), "{:?}", app.error);
                assert!(app.repository.as_ref().unwrap().is_remote());
                assert_eq!(app.access_mode, AccessMode::ReadOnly);
                assert!(app.view == View::Repositories);
                assert!(app.history.files.loaded);
                assert_eq!(app.history.commits.len(), 100);
                assert!(app.external_file_target().is_none());
                app.set_access_mode(AccessMode::ReadWrite, cx);
                assert_eq!(app.access_mode, AccessMode::ReadOnly);
                assert!(!app.staging_visible());
                app.switch_view(View::Working, cx);
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert!(app.working.loaded);
                assert!(!app.working.changes.is_empty());
                assert!(app.working.error.is_none(), "{:?}", app.working.error);
                app.switch_view(View::History, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert!(app.history.files.loaded);
                assert_eq!(app.history.commits.len(), 100);
                app.browse_branch(Some("refs/heads/feature/demo".into()), cx);
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert_eq!(app.viewed_branch(), Some("refs/heads/feature/demo"));
                app.switch_view(View::Repositories, cx);
                app.switch_view(View::Working, cx);
                assert!(!app.working.files_loading);
                assert!(!app.working.changes.is_empty());
            })
        });
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                let old = app.repository.clone().unwrap();
                let path = old.location.clone();
                app.disconnect_repository(&path, window, cx);
                assert!(app.repository.is_none());
                assert!(app.working.changes.is_empty());
                assert!(app.history.commits.is_empty());
                assert!(app.preferences.repository.is_none());
                assert_eq!(app.preferences.recent_repositories.len(), 1);
                assert!(old.branch_name().is_err());
                app.reconnect_repository(path.clone(), window, cx);
                assert_eq!(app.remote_status(&path), "ssh_connecting");
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app.read(cx);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert!(app.repository.as_ref().unwrap().is_remote());
            assert!(app.working.loaded);
            assert_eq!(app.preferences.recent_repositories.len(), 1);
        });
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        cx.update(|cx| app.update(cx, |app, cx| app.load(directory.path().to_owned(), cx)));
        cx.run_until_parked();
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert!(!app.repository.as_ref().unwrap().is_remote());
                assert_eq!(app.preferences.recent_repositories.len(), 2);
                app.set_access_mode(AccessMode::ReadWrite, cx);
                assert!(app.staging_visible());
            })
        });
    }

    #[gpui_kit::test]
    fn staged_gutter_selections_produce_the_expected_git_commit(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        for layout in [Layout::Split, Layout::Inline] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            git(root, &["init", "-b", "main"]);
            git(root, &["config", "core.autocrlf", "false"]);
            git(root, &["config", "core.hooksPath", ".git/no-hooks"]);
            std::fs::write(root.join("f.txt"), "one\nkeep\ntwo\nkeep\nthree\n").unwrap();
            std::fs::write(root.join("ready.txt"), "base\n").unwrap();
            git(root, &["add", "--all"]);
            git(root, &["commit", "-m", "base"]);
            let working = "ONE\nkeep\nTWO\nkeep\nTHREE\n";
            std::fs::write(root.join("f.txt"), working).unwrap();
            std::fs::write(root.join("ready.txt"), "already staged\n").unwrap();
            git(root, &["add", "ready.txt"]);
            std::fs::write(root.join("leave.txt"), "untracked\n").unwrap();
            let (handle, app) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        let mut app = test_app(window, cx);
                        app.preferences.layout = layout;
                        app.load(root.to_owned(), cx);
                        app
                    })
                })
                .unwrap()
            });
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                window.click("access-mode", cx);
            })
            .unwrap();
            cx.run_until_parked();
            for block in [0usize, 2, 0] {
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    window.render_frame(cx);
                    window.click(("stage-block", block), cx);
                })
                .unwrap();
                cx.run_until_parked();
                cx.update(|cx| {
                    assert!(
                        app.read(cx).working.error.is_none(),
                        "{:?}",
                        app.read(cx).working.error
                    )
                });
            }
            // Commit the exact index generated by real button clicks, without `add` or `-a`.
            git(root, &["commit", "-m", "selected in the UI"]);
            for (path, expected) in [
                ("f.txt", "one\nkeep\ntwo\nkeep\nTHREE\n"),
                ("ready.txt", "already staged\n"),
            ] {
                let result = std::process::Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(["show", &format!("HEAD:{path}")])
                    .output()
                    .unwrap();
                assert!(result.status.success());
                assert_eq!(result.stdout, expected.as_bytes());
            }
            let paths = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["ls-tree", "-r", "--name-only", "HEAD"])
                .output()
                .unwrap();
            assert_eq!(paths.stdout, b"f.txt\nready.txt\n");
            assert_eq!(
                std::fs::read(root.join("f.txt")).unwrap(),
                working.as_bytes()
            );
            let remaining = Repository::open(root).unwrap().snapshot().unwrap().changes;
            assert!(
                remaining
                    .iter()
                    .any(|c| c.path == std::path::Path::new("f.txt") && c.area == Area::Unstaged)
            );
            assert!(
                remaining
                    .iter()
                    .any(|c| c.path == std::path::Path::new("leave.txt")
                        && c.area == Area::Untracked)
            );
        }
    }

    #[gpui_kit::test]
    fn undo_redo_restores_discarded_files_and_staging_through_the_toolbar(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        std::fs::write(directory.path().join("new.txt"), "recover this content\n").unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.access_mode = AccessMode::ReadWrite;
                    app.load(directory.path().to_owned(), cx);
                    app
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(window.find("undo-write").disabled(), Some(true));
            window.click(("discard-block", 0usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!directory.path().join("new.txt").exists());
        for (button, exists) in [
            ("undo-write", true),
            ("redo-write", false),
            ("undo-write", true),
        ] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                assert_ne!(window.find(button).disabled(), Some(true));
                window.click(button, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                assert!(
                    app.read(cx).working.error.is_none(),
                    "{:?}",
                    app.read(cx).working.error
                )
            });
            assert_eq!(directory.path().join("new.txt").exists(), exists);
        }
        assert_eq!(
            std::fs::read(directory.path().join("new.txt")).unwrap(),
            b"recover this content\n"
        );
        for (button, area) in [
            ("stage-current-file", Area::Staged),
            ("undo-write", Area::Untracked),
            ("redo-write", Area::Staged),
        ] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                window.click(button, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| assert_eq!(app.read(cx).working.changes[0].area, area));
        }
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.refresh(&Refresh, window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert!(app.write_history.can_undo());
                app.set_access_mode(AccessMode::ReadOnly, cx);
                app.restore_write(true, cx);
                assert!(app.write_task.is_none());
            })
        });
        let other = tempfile::tempdir().unwrap();
        git(other.path(), &["init", "-b", "main"]);
        cx.update(|cx| app.update(cx, |app, cx| app.load(other.path().to_owned(), cx)));
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(!app.read(cx).write_history.can_undo());
            assert!(!app.read(cx).write_history.can_redo());
        });
    }

    #[gpui_kit::test]
    fn staging_modes_and_buttons_update_the_index_in_both_diff_layouts(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        for layout in [Layout::Split, Layout::Inline] {
            let directory = tempfile::tempdir().unwrap();
            git(directory.path(), &["init", "-b", "main"]);
            git(directory.path(), &["config", "core.autocrlf", "false"]);
            std::fs::write(directory.path().join("f.txt"), "a\nb\nc\nd\n").unwrap();
            git(directory.path(), &["add", "f.txt"]);
            git(directory.path(), &["commit", "-m", "base"]);
            let working = "a\nB\nc\nD\n";
            std::fs::write(directory.path().join("f.txt"), working).unwrap();
            let index_text = || {
                let result = std::process::Command::new("git")
                    .arg("-C")
                    .arg(directory.path())
                    .args(["show", ":f.txt"])
                    .output()
                    .unwrap();
                assert!(result.status.success());
                String::from_utf8(result.stdout).unwrap()
            };
            let (handle, app) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        let mut app = test_app(window, cx);
                        app.preferences.layout = layout;
                        app.load(directory.path().to_owned(), cx);
                        app
                    })
                })
                .unwrap()
            });
            cx.run_until_parked();
            let mut source_positions = Vec::new();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                source_positions = window
                    .find_all("source")
                    .iter()
                    .map(|el| el.bounds().left())
                    .collect::<Vec<_>>();
                assert!(!source_positions.is_empty());
                assert_eq!(app.read(cx).access_mode, AccessMode::ReadOnly);
                assert_eq!(window.find("access-mode").checked(), Some(true));
                assert!(window.try_find("stage-current-file").is_none());
                let number = window.within(("new", 0usize)).find_all(("line", 0usize))[0].bounds();
                for position in [
                    number.center(),
                    point(number.left() + px(2.), number.center().y),
                ] {
                    window.drag(position, position, cx);
                    assert!(
                        app.read(cx)
                            .working
                            .diff
                            .read(cx)
                            .selected_lines()
                            .is_none(),
                        "the line-number gutter must not select a whole line"
                    );
                    assert!(
                        app.read(cx)
                            .working
                            .diff
                            .read(cx)
                            .selected_text(cx)
                            .is_empty()
                    );
                }
                app.update(cx, |app, cx| {
                    app.write_change(
                        app.working.changes[0].clone(),
                        ChangeOperation::Stage(StageSelection::File),
                        None,
                        cx,
                    );
                    assert!(app.write_task.is_none());
                });
                window.click("access-mode", cx);
            })
            .unwrap();
            cx.run_until_parked();

            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                assert_eq!(
                    source_positions,
                    window
                        .find_all("source")
                        .iter()
                        .map(|el| el.bounds().left())
                        .collect::<Vec<_>>()
                );
                assert_eq!(app.read(cx).access_mode, AccessMode::ReadWrite);
                assert_eq!(window.find("access-mode").checked(), Some(false));
                assert!(window.try_find("popup-menu").is_none());
                assert!(window.try_find(("stage-line", 2usize)).is_none());
                assert!(window.try_find(("discard-block", 0usize)).is_some());
                assert_eq!(window.find(("stage-block", 0usize)).checked(), Some(false));
                window.click(("stage-block", 0usize), cx);
                assert!(
                    app.read(cx)
                        .working
                        .diff
                        .read(cx)
                        .selected_lines()
                        .is_none(),
                    "staging a block must not select its source line"
                );
                assert!(
                    app.read(cx)
                        .working
                        .diff
                        .read(cx)
                        .selected_text(cx)
                        .is_empty()
                );
                app.update(cx, |app, cx| {
                    // An in-flight write cannot be relabelled read-only or duplicated.
                    app.set_access_mode(AccessMode::ReadOnly, cx);
                    assert_eq!(app.access_mode, AccessMode::ReadWrite);
                });
                window.render_frame(cx);
                assert_eq!(window.find("access-mode").checked(), Some(false));
                assert_eq!(window.find("open-with").disabled(), Some(true));
                for trigger in ["access-mode", "open-with"] {
                    window.click(trigger, cx);
                    window.render_frame(cx);
                    assert!(
                        window.try_find("popup-menu").is_none(),
                        "a visually stable write lock must still block the menu"
                    );
                }
                assert_eq!(app.read(cx).access_mode, AccessMode::ReadWrite);
                assert_eq!(window.find("access-mode").checked(), Some(false));
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(index_text(), "a\nB\nc\nd\n");
            cx.update(|cx| {
                app.update(cx, |app, cx| {
                    assert!(app.working.error.is_none(), "{:?}", app.working.error);
                    assert_eq!(app.working.changes.len(), 1);
                    assert_eq!(app.working.changes[0].area, Area::Partial);
                    assert_eq!(
                        app.working.staging_patch.as_ref().unwrap().progress(),
                        (1, 2)
                    );
                    assert_eq!(app.working.diff.read(cx).files()[0].change_block_count(), 2);
                })
            });
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                assert!(window.try_find(("discard-block", 0usize)).is_none());
                assert_eq!(window.find(("stage-block", 0usize)).checked(), Some(true));
                window.click(("stage-block", 0usize), cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(index_text(), "a\nb\nc\nd\n");
            for tree in [false, true] {
                for expected in [working, "a\nb\nc\nd\n"] {
                    cx.update_window(handle, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            app.preferences.tree_file_list = tree;
                            cx.notify();
                        });
                        window.render_frame(cx);
                        window.render_frame(cx);
                        window.click(("stage-file", 0usize), cx);
                    })
                    .unwrap();
                    cx.run_until_parked();
                    assert_eq!(index_text(), expected);
                }
            }
            for confirm in [false, true] {
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    window.render_frame(cx);
                    window.click(("discard-block", 0usize), cx);
                })
                .unwrap();
                cx.run_until_parked();
                assert_eq!(
                    std::fs::read_to_string(directory.path().join("f.txt")).unwrap(),
                    working
                );
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    window.render_frame(cx);
                    window.click(if confirm { "ok" } else { "cancel" }, cx);
                })
                .unwrap();
                cx.run_until_parked();
                assert_eq!(index_text(), "a\nb\nc\nd\n");
                assert_eq!(
                    std::fs::read_to_string(directory.path().join("f.txt")).unwrap(),
                    if confirm { "a\nb\nc\nD\n" } else { working }
                );
                cx.update(|cx| {
                    assert!(
                        app.read(cx).working.error.is_none(),
                        "{:?}",
                        app.read(cx).working.error
                    )
                });
            }
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| app.switch_view(View::History, cx));
                window.render_frame(cx);
                assert!(window.try_find("stage-current-file").is_none());
                assert!(window.try_find(("stage-file", 0usize)).is_none());
                window.click("access-mode", cx);
                window.render_frame(cx);
                assert_eq!(app.read(cx).access_mode, AccessMode::ReadOnly);
                assert_eq!(window.find("access-mode").checked(), Some(true));
                assert!(window.try_find("popup-menu").is_none());
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(
                std::fs::read_to_string(directory.path().join("f.txt")).unwrap(),
                "a\nb\nc\nD\n"
            );
        }
    }

    #[gpui_kit::test]
    fn untracked_group_is_collapsible_and_stages_only_its_own_files(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(directory.path().join("src/tracked.rs"), "old\n").unwrap();
        git(directory.path(), &["add", "-A"]);
        git(directory.path(), &["commit", "-m", "base"]);
        std::fs::write(directory.path().join("src/tracked.rs"), "new\n").unwrap();
        std::fs::write(directory.path().join("src/new.rs"), "added\n").unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.preferences.tree_file_list = true;
                    app.access_mode = AccessMode::ReadWrite;
                    app.load(directory.path().to_owned(), cx);
                    app
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find("tree-tracked").visible());
            assert!(window.find("tree-untracked").visible());
            assert!(window.find("stage-folder:untracked:src").visible());
            window.click("tree-untracked", cx);
            window.render_frame(cx);
            assert!(window.try_find("stage-folder:untracked:src").is_none());
            assert!(window.find("stage-folder:src").visible());
            window.click("tree-untracked", cx);
            window.render_frame(cx);
            window.click("stage-folder:tree-untracked", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            let app = app.read(cx);
            assert!(app.working.error.is_none(), "{:?}", app.working.error);
            assert_eq!(app.working.changes.len(), 2);
            assert!(app.working.changes.iter().any(|c| c.path == std::path::Path::new("src/tracked.rs") && c.area == Area::Unstaged));
            assert!(app.working.changes.iter().any(|c| c.path == std::path::Path::new("src/new.rs") && c.area == Area::Staged));
            assert!(window.try_find("tree-untracked").is_none());
        }).unwrap();
    }

    #[gpui_kit::test]
    fn folder_controls_stage_descendants_and_keep_neighboring_paths_unchanged(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        std::fs::create_dir_all(directory.path().join("src/deep")).unwrap();
        std::fs::create_dir(directory.path().join("src-other")).unwrap();
        for path in ["src/a.txt", "src/deep/b.txt", "src-other/c.txt"] {
            std::fs::write(directory.path().join(path), "base\n").unwrap();
        }
        git(directory.path(), &["add", "-A"]);
        git(directory.path(), &["commit", "-m", "base"]);
        std::fs::write(directory.path().join("src/a.txt"), "staged\n").unwrap();
        git(directory.path(), &["add", "src/a.txt"]);
        std::fs::write(directory.path().join("src/a.txt"), "latest\n").unwrap();
        std::fs::write(directory.path().join("src/deep/b.txt"), "nested\n").unwrap();
        std::fs::write(directory.path().join("src-other/c.txt"), "outside\n").unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.preferences.tree_file_list = true;
                    app.load(directory.path().to_owned(), cx);
                    app
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.try_find("stage-folder:src").is_none());
            app.update(cx, |app, cx| app.set_access_mode(AccessMode::ReadWrite, cx));
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(window.find("stage-folder:src").checked(), Some(false));
            assert_eq!(
                window.find("stage-folder:src").label(),
                Some(format!("{}: src", tr("stage_directory_remaining")).as_str())
            );
            window.click("stage-folder:src", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            let app = app.read(cx);
            assert!(app.working.error.is_none(), "{:?}", app.working.error);
            assert!(
                app.working
                    .changes
                    .iter()
                    .filter(|c| c.path.starts_with("src"))
                    .all(|c| c.area == Area::Staged)
            );
            assert!(
                app.working
                    .changes
                    .iter()
                    .any(|c| c.path.starts_with("src-other") && c.area == Area::Unstaged)
            );
            assert_eq!(window.find("stage-folder:src").checked(), Some(true));
            assert!(
                window
                    .find(format!(
                        "stage-folder:{}",
                        PathBuf::from("src").join("deep").display()
                    ))
                    .visible(),
                "staging must not collapse the folder"
            );
            window.click("stage-folder:src", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(
                app.read(cx).working.error.is_none(),
                "{:?}",
                app.read(cx).working.error
            );
            assert!(
                app.read(cx)
                    .working
                    .changes
                    .iter()
                    .all(|c| c.area == Area::Unstaged)
            );
            assert_eq!(window.find("stage-folder:src").checked(), Some(false));
            // The root uses the same operation, covering every descendant.
            window.click("stage-folder:", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(
                app.read(cx)
                    .working
                    .changes
                    .iter()
                    .all(|c| c.area == Area::Staged)
            )
        });
        assert_eq!(
            std::fs::read_to_string(directory.path().join("src/a.txt")).unwrap(),
            "latest\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("src/deep/b.txt")).unwrap(),
            "nested\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("src-other/c.txt")).unwrap(),
            "outside\n"
        );
    }

    #[gpui_kit::test]
    fn block_writes_preserve_reading_position_and_expanded_context(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        for layout in [Layout::Split, Layout::Inline] {
            for operation in [
                ChangeOperation::Stage(StageSelection::Block(1)),
                ChangeOperation::DiscardBlock(1),
            ] {
                for staged in [false, true] {
                    if staged && matches!(operation, ChangeOperation::DiscardBlock(_)) {
                        continue;
                    }
                    let directory = tempfile::tempdir().unwrap();
                    git(directory.path(), &["init", "-b", "main"]);
                    let base = (1..=60)
                        .map(|i| {
                            format!(
                                "context_{i:02}: {}\n",
                                "long wrapped source text / ".repeat(5)
                            )
                        })
                        .collect::<String>();
                    std::fs::write(directory.path().join("f.txt"), &base).unwrap();
                    git(directory.path(), &["add", "f.txt"]);
                    git(directory.path(), &["commit", "-m", "base"]);
                    let changed = base
                        .replace("context_05", "changed_05")
                        .replace("context_25", "changed_25")
                        .replace("context_55", "changed_55");
                    std::fs::write(directory.path().join("f.txt"), &changed).unwrap();
                    if staged {
                        git(directory.path(), &["add", "f.txt"]);
                    }
                    let (handle, app) = cx.update(|cx| {
                        gpui_kit::open_window(
                            WindowOptions {
                                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                                    Point::default(),
                                    size(px(1200.), px(900.)),
                                ))),
                                ..Default::default()
                            },
                            cx,
                            |window, cx| {
                                cx.new(|cx| {
                                    let mut app = test_app(window, cx);
                                    app.preferences.layout = layout;
                                    app.preferences.soft_wrap = true;
                                    app.access_mode = AccessMode::ReadWrite;
                                    app.load(directory.path().to_owned(), cx);
                                    app
                                })
                            },
                        )
                        .unwrap()
                    });
                    cx.run_until_parked();
                    let mut before = px(0.);
                    cx.update_window(handle, |_, window, cx| {
                        window.render_frame(cx);
                        window.render_frame(cx);
                        app.read(cx).working.diff.clone().update(cx, |diff, cx| {
                            diff.expand_unchanged(cx);
                            diff.scroll_to_line(
                                DiffLinePosition::new("f.txt", DiffSide::Modified, 20),
                                cx,
                            );
                        });
                        window.render_frame(cx);
                        window.render_frame(cx);
                        window.scroll(
                            "diff-body",
                            ScrollDelta::Pixels(point(px(0.), px(-9.5))),
                            cx,
                        );
                        window.render_frame(cx);
                        window.render_frame(cx);
                        before = window
                            .within(("new", 19usize))
                            .find("source")
                            .bounds()
                            .top();
                        window.click(
                            (
                                if matches!(operation, ChangeOperation::Stage(_)) {
                                    "stage-block"
                                } else {
                                    "discard-block"
                                },
                                1usize,
                            ),
                            cx,
                        );
                    })
                    .unwrap();
                    cx.run_until_parked();
                    if matches!(operation, ChangeOperation::DiscardBlock(_)) {
                        cx.update_window(handle, |_, window, cx| {
                            window.render_frame(cx);
                            window.render_frame(cx);
                            window.click("ok", cx);
                        })
                        .unwrap();
                        cx.run_until_parked();
                    }
                    cx.update_window(handle, |_, window, cx| {
                        window.render_frame(cx);
                        window.render_frame(cx);
                        assert!(
                            app.read(cx).working.error.is_none(),
                            "{:?}",
                            app.read(cx).working.error
                        );
                        assert!(
                            !app.read(cx).working.diff.read(cx).has_collapsed_context(),
                            "refresh collapsed the reading context"
                        );
                        let after = window
                            .within(("new", 19usize))
                            .find("source")
                            .bounds()
                            .top();
                        assert!(
                            (before - after).abs() < px(1.),
                            "reading position moved from {before:?} to {after:?}"
                        );
                    })
                    .unwrap();
                }
            }
        }
    }

    #[gpui_kit::test]
    fn block_controls_work_when_one_diff_side_is_empty(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        for layout in [Layout::Split, Layout::Inline] {
            // A pure addition, pure deletion, a new file and a deleted file.
            for (before, after) in [
                (Some("a\nb\n"), Some("a\nnew\nb\n")),
                (Some("a\nold\nb\n"), Some("a\nb\n")),
                (None, Some("new\nfile")),
                (Some("old\nfile"), None),
            ] {
                let directory = tempfile::tempdir().unwrap();
                git(directory.path(), &["init", "-b", "main"]);
                git(directory.path(), &["config", "core.autocrlf", "false"]);
                if let Some(text) = before {
                    std::fs::write(directory.path().join("f.txt"), text).unwrap();
                }
                git(directory.path(), &["add", "-A"]);
                git(directory.path(), &["commit", "--allow-empty", "-m", "base"]);
                if let Some(text) = after {
                    std::fs::write(directory.path().join("f.txt"), text).unwrap();
                } else {
                    std::fs::remove_file(directory.path().join("f.txt")).unwrap();
                }
                let (handle, app) = cx.update(|cx| {
                    gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                        cx.new(|cx| {
                            let mut app = test_app(window, cx);
                            app.preferences.layout = layout;
                            app.access_mode = AccessMode::ReadWrite;
                            app.load(directory.path().to_owned(), cx);
                            app
                        })
                    })
                    .unwrap()
                });
                cx.run_until_parked();
                // Stage and unstage through the same gutter, including its empty side.
                for _ in 0..2 {
                    cx.update_window(handle, |_, window, cx| {
                        window.render_frame(cx);
                        window.render_frame(cx);
                        assert_eq!(window.find_all(("stage-block", 0usize)).len(), 1);
                        window.click(("stage-block", 0usize), cx);
                    })
                    .unwrap();
                    cx.run_until_parked();
                    cx.update(|cx| {
                        assert!(
                            app.read(cx).working.error.is_none(),
                            "{:?}",
                            app.read(cx).working.error
                        )
                    });
                }
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    window.render_frame(cx);
                    assert_eq!(window.find_all(("discard-block", 0usize)).len(), 1);
                    window.click(("discard-block", 0usize), cx);
                })
                .unwrap();
                cx.run_until_parked();
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    window.render_frame(cx);
                    window.click("ok", cx);
                })
                .unwrap();
                cx.run_until_parked();
                cx.update(|cx| {
                    assert!(
                        app.read(cx).working.error.is_none(),
                        "{:?}",
                        app.read(cx).working.error
                    )
                });
                assert_eq!(
                    std::fs::read_to_string(directory.path().join("f.txt"))
                        .ok()
                        .as_deref(),
                    before
                );
                let repo = Repository::open(directory.path()).unwrap();
                assert!(repo.snapshot().unwrap().changes.is_empty());
            }
        }
    }

    #[gpui_kit::test]
    fn isolated_file_selections_load_immediately_but_bursts_wait_for_the_last_file(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        let names = ["go.mod", "go.sum", "main.go"];
        for name in names {
            std::fs::write(directory.path().join(name), format!("{name}\n")).unwrap();
        }
        let (_, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.repository = Some(Repository::open(directory.path()).unwrap());
                    app.working.loaded = true;
                    app.working.changes = names
                        .into_iter()
                        .map(|path| Change {
                            path: path.into(),
                            previous_path: None,
                            status: 'A',
                            area: Area::Untracked,
                        })
                        .collect();
                    app
                })
            })
            .unwrap()
        });
        let select = |index, cx: &mut TestAppContext| {
            cx.update(|cx| app.update(cx, |app, cx| app.select_file(index, cx)));
            cx.run_until_parked();
        };
        let verify = |path: &str, loading: bool, cx: &mut TestAppContext| {
            cx.update(|cx| {
                let files = &app.read(cx).working;
                assert_eq!(files.patch_loading, loading);
                assert_eq!(files.diff.read(cx).files()[0].path().as_str(), path);
            });
        };
        // No clock advancement: the first file is available as soon as I/O finishes.
        select(0, cx);
        verify("go.mod", false, cx);
        select(1, cx);
        verify("go.mod", true, cx);
        cx.executor().advance_clock(FILE_SELECTION_DELAY / 2);
        select(2, cx);
        cx.executor()
            .advance_clock(FILE_SELECTION_DELAY - Duration::from_millis(1));
        cx.run_until_parked();
        verify("go.mod", true, cx);
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        verify("main.go", false, cx);
        // Once the burst settles, another uncached file needs no extra timer.
        select(1, cx);
        verify("go.sum", false, cx);
    }

    #[gpui_kit::test]
    fn titlebar_buttons_exclude_background_hitboxes_and_still_handle_clicks(
        cx: &mut TestAppContext,
    ) {
        use std::{cell::RefCell, rc::Rc};

        struct ToolbarProbe {
            app: Entity<Patchlook>,
            background: Rc<RefCell<Option<Hitbox>>>,
        }
        impl Render for ToolbarProbe {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let background = self.background.clone();
                let toolbar = self
                    .app
                    .update(cx, |app, cx| app.toolbar(cx).into_any_element());
                div()
                    .relative()
                    .size_full()
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                *background.borrow_mut() =
                                    Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(toolbar)
            }
        }

        cx.update(|cx| {
            gpui_kit::init(cx);
            // Test clicks at stable bounds, independently of dialog entrance timing.
            cx.set_reduce_motion(true);
        });
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        std::fs::write(directory.path().join("refresh.txt"), "Refresh me\n").unwrap();
        let repository = Repository::open(directory.path()).unwrap();
        let background = Rc::new(RefCell::new(None));
        let (handle, probe) = cx.update(|cx| {
            gpui_kit::open_window(TitleBar::window_options(), cx, |window, cx| {
                let app = cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.repository = Some(repository);
                    app
                });
                cx.new(|_| ToolbarProbe {
                    app,
                    background: background.clone(),
                })
            })
            .unwrap()
        });
        let app = cx.update(|cx| probe.read(cx).app.clone());
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            let mut controls = vec![
                "open-repository",
                "refresh",
                "access-mode",
                "language",
                "theme",
            ];
            if cfg!(any(target_os = "windows", target_os = "linux")) {
                controls.push("application-menu");
            } else {
                assert!(window.try_find("application-menu").is_none());
            }
            for id in controls {
                let bounds = window.find(id).bounds();
                for position in [bounds.center(), bounds.origin + point(px(1.), px(1.))] {
                    window.simulate_mouse_move(position, cx);
                    window.render_frame(cx);
                    // Windows checks the full hit-test chain, not only hovered items.
                    // This API tests membership in that same chain, so scroll-only
                    // blocking would not incorrectly pass as a native drag exclusion.
                    assert!(
                        !background
                            .borrow()
                            .as_ref()
                            .unwrap()
                            .should_handle_scroll(window),
                        "{id} still exposes its titlebar background to native hit testing"
                    );
                }
            }
            let refresh = window.find("refresh").bounds();
            let language = window.find("language").bounds();
            let blank = point((refresh.right() + language.left()) / 2., refresh.center().y);
            window.simulate_mouse_move(blank, cx);
            window.render_frame(cx);
            assert!(
                background
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .should_handle_scroll(window)
            );

            let next = app.read(cx).preferences.appearance.next();
            window.click("theme", cx);
            assert_eq!(app.read(cx).preferences.appearance, next);
            window.render_frame(cx);
            window.click("language", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("popup-menu").is_some());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("open-repository", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").click(0usize, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("open-repository", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.within("popup-menu").click(1usize, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find("ssh-form").visible());
            window.click("ssh-cancel", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(
                !window.has_active_dialog(cx),
                "SSH cancel must dismiss the dialog before clicking the titlebar"
            );
            window.click("refresh", cx);
            assert!(app.read(cx).loading);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app.read(cx);
            assert!(!app.loading);
            assert!(
                app.working
                    .changes
                    .iter()
                    .any(|change| change.path == std::path::Path::new("refresh.txt"))
            );
        });
    }

    #[gpui_kit::test]
    fn display_changes_from_another_window_survive_repository_preference_writes(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Patchlook::from_preferences(None, Preferences::default(), None, window, cx)
                })
            })
            .unwrap()
        });
        let (other_handle, _) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| test_app(window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(other_handle, |_, window, cx| {
            apply_language(Language::English, cx);
            apply_appearance(Appearance::Dark, window, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(app.read(cx).preferences.language, Language::English);
            assert_eq!(app.read(cx).preferences.appearance, Appearance::Dark);
            app.update(cx, |this, _| {
                this.preferences.tree_file_list = false;
                this.save_preferences();
            });
            let saved: Preferences = serde_json::from_slice(
                &std::fs::read(
                    app.read(cx)
                        .preferences_directory
                        .path()
                        .join("preferences.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(saved.language, Language::English);
            assert_eq!(saved.appearance, Appearance::Dark);
            assert!(!saved.tree_file_list);
            assert!(app.read(cx).load_task.is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn welcome_opens_an_independent_text_comparison(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::text_compare::init(cx);
        });
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| test_app(window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("welcome-compare", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.read(|cx| {
            assert_eq!(cx.windows().len(), 2);
            assert!(app.read(cx).repository.is_none());
            assert!(app.read(cx).load_task.is_none());
            assert!(app.read(cx).preferences.recent_repositories.is_empty());
        });
    }

    #[gpui_kit::test]
    fn application_menu_opens_about_and_dispatches_quit(cx: &mut TestAppContext) {
        use std::{cell::Cell, rc::Rc};

        struct MenuProbe(Entity<Patchlook>);
        impl Render for MenuProbe {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    // The extra test trigger sits outside Patchlook on macOS;
                    // give it the same ancestor action route as the real toolbar.
                    .on_action(cx.listener(|this, action: &About, window, cx| {
                        this.0
                            .update(cx, |app, cx| app.show_about(action, window, cx));
                    }))
                    .child(self.0.clone())
                    .when(
                        !cfg!(any(target_os = "windows", target_os = "linux")),
                        |view| {
                            // Exercise the same trigger on macOS test hosts without
                            // adding it to the macOS application's real toolbar.
                            view.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .child(self.0.read(cx).application_menu(cx)),
                            )
                        },
                    )
            }
        }
        let quits = Rc::new(Cell::new(0));
        let on_quit = quits.clone();
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            cx.on_action(move |_: &Quit, _| on_quit.set(on_quit.get() + 1));
        });
        let (handle, probe) = cx.update(|cx| {
            gpui_kit::open_window(TitleBar::window_options(), cx, |window, cx| {
                let app = cx.new(|cx| test_app(window, cx));
                window.focus(&app.read(cx).focus.clone(), cx);
                cx.new(|_| MenuProbe(app))
            })
            .unwrap()
        });
        let focus = cx.update(|cx| probe.read(cx).0.read(cx).focus.clone());
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("application-menu", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let menu = window.within("popup-menu");
            assert_eq!(menu.find(0usize).label(), Some(tr("new_text_comparison")));
            assert_eq!(menu.find(2usize).label(), Some(tr("about")));
            assert_eq!(menu.find(4usize).label(), Some(tr("quit")));
            assert!(menu.try_find(5usize).is_none());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("popup-menu").is_none());
            assert_eq!(window.focused(cx), Some(focus));
            // The trigger is keyboard reachable; Enter opens its dropdown.
            for _ in 0..16 {
                window.press("tab", cx);
                if window.find("application-menu").focused() == Some(true) {
                    break;
                }
            }
            assert_eq!(window.find("application-menu").focused(), Some(true));
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("popup-menu").is_some());
            window.within("popup-menu").click(2usize, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("about-content").visible());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.click("application-menu", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("up", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(quits.get(), 1);
    }

    #[gpui_kit::test]
    fn about_links_and_license_dialog_work_without_a_repository(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| test_app(window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.focus(&app.read(cx).focus.clone(), cx);
            window.render_frame(cx);
            window.dispatch_action(Box::new(About), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find("about-content").visible());
            window.click("about-github", cx);
        })
        .unwrap();
        assert_eq!(
            cx.opened_url().as_deref(),
            Some(env!("CARGO_PKG_REPOSITORY"))
        );
        cx.update_window(handle, |_, window, cx| window.click("about-issues", cx))
            .unwrap();
        assert_eq!(
            cx.opened_url().as_deref(),
            Some(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues"))
        );
        cx.update_window(handle, |_, window, cx| window.click("about-license", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find("app-license-text").visible());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("about-surface").visible());
            assert!(window.try_find("app-license-text").is_none());
            assert!(window.find("about-content").visible());
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("about-surface").is_none());
            assert_eq!(window.focused(cx), Some(app.read(cx).focus.clone()));
            window.render_frame(cx);
            window.dispatch_action(Box::new(About), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("about-surface").visible());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn about_is_centered_and_keeps_focus_until_each_dismissal_finishes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| test_app(window, cx))
            })
            .unwrap()
        });
        let (enter, exit) = cx.update(|cx| {
            let motion = cx.theme().motion_tokens();
            (motion.duration_normal, motion.duration_fast)
        });
        cx.update_window(handle, |_, window, cx| {
            window.focus(&app.read(cx).focus.clone(), cx);
            window.render_frame(cx);
            assert!(window.try_find("about-surface").is_none());
        })
        .unwrap();
        for dismissal in ["button", "escape", "backdrop"] {
            cx.update_window(handle, |_, window, cx| {
                window.dispatch_action(Box::new(About), cx);
                window.render_frame(cx);
            })
            .unwrap();
            cx.executor().advance_clock(enter);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let bounds = window.find("about-surface").bounds();
                let viewport = window.viewport_size();
                assert!((bounds.center().y - viewport.height / 2.).abs() < px(1.));
                match dismissal {
                    "button" => window.click("about-close", cx),
                    "escape" => window.press("escape", cx),
                    _ => {
                        window.dispatch_event(
                            MouseDownEvent {
                                position: point(px(1.), px(1.)),
                                button: MouseButton::Left,
                                click_count: 1,
                                modifiers: Modifiers::default(),
                                first_mouse: false,
                            }
                            .to_platform_input(),
                            cx,
                        );
                    }
                }
                window.render_frame(cx);
                assert!(window.find("about-surface").visible());
                assert_ne!(window.focused(cx), Some(app.read(cx).focus.clone()));
            })
            .unwrap();
            cx.executor().advance_clock(exit / 2);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.find("about-surface").visible());
            })
            .unwrap();
            cx.executor().advance_clock(exit);
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("about-surface").is_none());
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                assert_eq!(window.focused(cx), Some(app.read(cx).focus.clone()));
            })
            .unwrap();
        }
    }

    fn git(directory: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(directory)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[gpui_kit::test]
    fn open_with_menu_offers_program_selection_and_waits_for_the_visible_file(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        std::fs::write(directory.path().join("file.txt"), "Working file").unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.load(directory.path().to_owned(), cx);
                    app
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(
                app.read(cx).external_file_target().unwrap().change.path,
                PathBuf::from("file.txt")
            );
            window.click("open-with", cx);
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(
                window
                    .find_by_label(tr("system_default_application"))
                    .visible()
            );
            // Recommended apps vary by machine; Up wraps to the final "Other" item.
            window.press("up", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files && !options.multiple);
            None
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.external_files.is_empty());
                app.active_files_mut().patch_loading = true;
                assert!(app.external_file_target().is_none());
                cx.notify();
            });
            window.render_frame(cx);
            window.click("open-with", cx);
            assert!(
                window
                    .try_find_by_label(tr("system_default_application"))
                    .is_none()
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn removing_repositories_persists_the_list_and_clears_their_content(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let repositories = ["alpha", "beta", "gamma"].map(|name| {
            let path = directory.path().join(name);
            std::fs::create_dir(&path).unwrap();
            git(&path, &["init", "-b", "main"]);
            std::fs::write(path.join("file.txt"), name).unwrap();
            git(&path, &["add", "."]);
            git(&path, &["commit", "-qm", "Initial"]);
            std::fs::write(path.join("file.txt"), "Working change").unwrap();
            Repository::open(&path).unwrap()
        });
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    for repo in &repositories {
                        app.preferences.remember_repository(repo.root.clone());
                    }
                    app.load(repositories[1].root.clone(), cx);
                    app
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                assert!(!app.working.changes.is_empty());
                app.switch_view(View::Repositories, cx);
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                assert!(!app.history.commits.is_empty());
                app.history.query = "old search".into();
                app.history_search
                    .update(cx, |input, cx| input.set_value("old search", window, cx));
            });
        })
        .unwrap();

        // Remove a middle entry, then the last entry, then the only remaining entry.
        for (removed, next, remaining) in [(1, Some(2), 2), (2, Some(0), 1), (0, None, 0)] {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                assert!(window.try_find("about").is_none());
                assert!(
                    window.find("add-repository").bounds().right()
                        <= window.find("remove-repository").bounds().left()
                );
                assert_eq!(
                    app.read(cx).repository.as_ref().unwrap().root,
                    repositories[removed].root
                );
                window.click("remove-repository", cx);
                window.render_frame(cx);
                let state = app.read(cx);
                assert!(state.repository.is_none());
                assert!(state.working.changes.is_empty());
                assert!(state.history.commits.is_empty());
                assert!(state.history.query.is_empty());
                assert!(state.history_search.read(cx).value().is_empty());
                assert!(state.branches.is_empty());
                assert_eq!(state.preferences.recent_repositories.len(), remaining);
                // Disabled both while the replacement loads and when nothing is selected.
                window.click("remove-repository", cx);
                assert_eq!(
                    app.read(cx).preferences.recent_repositories.len(),
                    remaining
                );
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                let state = app.read(cx);
                let next_path = next.map(|index| repositories[index].location.clone());
                assert_eq!(
                    state.repository.as_ref().map(|repo| repo.location.clone()),
                    next_path
                );
                assert_eq!(state.preferences.repository, next_path);
                assert_eq!(state.history.commits.len(), usize::from(next.is_some()));
                assert!(state.visible_error().is_none());
                let saved: Preferences = serde_json::from_slice(
                    &std::fs::read(state.preferences_directory.path().join("preferences.json"))
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(saved.repository, next_path);
                assert_eq!(saved.recent_repositories.len(), remaining);
                assert!(
                    !saved
                        .recent_repositories
                        .iter()
                        .any(|entry| entry.path == repositories[removed].location)
                );
            });
            assert!(repositories[removed].root.join(".git").is_dir());
            assert_eq!(
                std::fs::read_to_string(repositories[removed].root.join("file.txt")).unwrap(),
                "Working change"
            );
        }
    }

    #[gpui_kit::test]
    fn tabs_retain_snapshots_and_route_pending_results_to_their_owner(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        std::fs::write(root.join("alpha.rs"), "ALPHA\n").unwrap();
        std::fs::write(root.join("beta.rs"), "BETA\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "Initial"]);
        git(root, &["branch", "older"]);
        std::fs::write(root.join("beta.rs"), "HISTORY\n").unwrap();
        git(root, &["commit", "-qam", "Second"]);
        std::fs::write(root.join("alpha.rs"), "WORKING\n").unwrap();
        let repo = Repository::open(root).unwrap();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.repository = Some(repo.clone());
                    app.branches = repo.branches().unwrap();
                    app.preferences.remember_repository(repo.root.clone());
                    app
                })
            })
            .unwrap()
        });
        // Start both independent loads and leave History hidden before either finishes.
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.ensure_view_loaded(cx);
                app.switch_view(View::Repositories, cx);
                app.switch_view(View::Working, cx);
                assert!(app.working.files_loading && app.history.loading);
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert!(app.working.loaded && app.history.files.loaded);
                assert_eq!(app.working.changes[0].path, PathBuf::from("alpha.rs"));
                assert_eq!(app.history.files.changes[0].path, PathBuf::from("beta.rs"));
                assert_eq!(app.history.files.selected_file, None);
                assert_eq!(app.history.commits.len(), 2);
                app.switch_view(View::History, cx);
                assert!(app.history.files.patch_loading);
                app.switch_view(View::Working, cx);
            })
        });
        cx.run_until_parked();
        cx.executor().advance_clock(FILE_SELECTION_DELAY);
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app.read(cx);
            assert_eq!(
                app.working.diff.read(cx).files()[0].path().as_str(),
                "alpha.rs"
            );
            assert_eq!(
                app.history.files.diff.read(cx).files()[0].path().as_str(),
                "beta.rs"
            );
            assert!(!app.history.files.patch_loading);
        });
        // No Git reads are possible now: revisiting loaded Tabs must still work.
        std::fs::rename(root.join(".git"), root.join("git-hidden")).unwrap();
        let (working_diff, history_diff, revision) = cx.update(|cx| {
            let app = app.read(cx);
            (
                app.working.diff.entity_id(),
                app.history.files.diff.entity_id(),
                app.history.revision.clone(),
            )
        });
        for target in [
            View::History,
            View::Repositories,
            View::Working,
            View::Repositories,
            View::History,
            View::Working,
        ] {
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| app.switch_view(target, cx));
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                let app = app.read(cx);
                assert!(!app.loading && !app.history.loading && !app.working.files_loading);
                assert!(app.visible_error().is_none());
                assert_eq!(app.working.diff.entity_id(), working_diff);
                assert_eq!(app.history.files.diff.entity_id(), history_diff);
                assert_eq!(app.history.revision, revision);
                assert_eq!(app.history.selected_commit, Some(0));
                assert_eq!(app.working.selected_file, Some(0));
                assert_eq!(app.history.files.selected_file, Some(0));
            });
        }
        // Reopening the selected file in Repos reuses the preview, including its identity.
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.switch_view(View::Repositories, cx));
            window.render_frame(cx);
            window.render_frame(cx);
            window.click("tree-file-0", cx);
            window.render_frame(cx);
            assert!(app.read(cx).repository_preview);
            assert!(!app.read(cx).history.files.patch_loading);
            window.click("back-to-files", cx);
            window.render_frame(cx);
            window.click("tree-file-0", cx);
            assert!(app.read(cx).repository_preview);
            assert_eq!(app.read(cx).history.files.diff.entity_id(), history_diff);
        })
        .unwrap();
        std::fs::rename(root.join("git-hidden"), root.join(".git")).unwrap();
        // Changing a browsing branch must not discard or reread working changes.
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                // Set the saved choice in memory so the test never writes real preferences.
                app.preferences
                    .set_viewed_branch(&repo.root, Some("refs/heads/older".into()));
                app.load_history_page(HistoryRequest::Branch(Some("refs/heads/older".into())), cx);
                app.switch_view(View::Working, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app.read(cx);
            assert_eq!(app.working.diff.entity_id(), working_diff);
            assert_eq!(app.history.commits.len(), 1);
            assert_eq!(app.history.commits[0].subject, "Initial");
            assert_eq!(app.history.files.changes.len(), 2);
        });
        // The title bar follows the visible source while the worktree stays on main.
        for view in [View::Repositories, View::History, View::Working] {
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| {
                    app.preferences.tree_file_list = true;
                    app.switch_view(view, cx);
                });
                window.render_frame(cx);
                window.render_frame(cx);
                let label = if view == View::Working {
                    tr("working_branch_tooltip").replace("{branch}", "main")
                } else {
                    tr("viewed_branch_tooltip")
                        .replace("{branch}", "older")
                        .replace("{worktree}", "main")
                };
                assert_eq!(window.find("toolbar-branch").label(), Some(label.as_str()));
                assert_eq!(
                    window.try_find("tree-branch").is_some(),
                    view != View::History
                );
                assert_eq!(
                    window.try_find("history-branch").is_some(),
                    view == View::Repositories
                );
            })
            .unwrap();
        }
        assert_eq!(repo.branch_name().unwrap(), "main");
        // An empty search result is a loaded snapshot too, and survives all Tab changes.
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.history.query = "not present in either commit".into();
                app.load_history_page(HistoryRequest::Search, cx);
            })
        });
        cx.run_until_parked();
        std::fs::rename(root.join(".git"), root.join("git-hidden")).unwrap();
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert!(app.history.files.loaded);
                assert!(app.history.commits.is_empty());
                for view in [
                    View::Repositories,
                    View::History,
                    View::Working,
                    View::History,
                ] {
                    app.switch_view(view, cx);
                    assert!(!app.history.loading);
                }
            })
        });
        cx.run_until_parked();
        cx.update(|cx| assert!(app.read(cx).visible_error().is_none()));
    }

    #[gpui_kit::test]
    fn repository_tree_opens_the_requested_preview_without_revealing_cached_content(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        std::fs::write(root.join("alpha.rs"), "ALPHA_CONTENT\n").unwrap();
        std::fs::write(root.join("beta.rs"), "BETA_CONTENT\n").unwrap();
        std::fs::write(
            root.join("image.png"),
            include_bytes!("../tests/fixtures/images/sample.png"),
        )
        .unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "Preview fixture"]);
        let repo = Repository::open(root).unwrap();
        let commit = repo.history(0, None).unwrap().commits.remove(0);

        // Both a cached text diff and a cached image must stay hidden at this boundary.
        for previous in [0, 2] {
            let changes = repo.commit_changes(&commit).unwrap();
            let stats = repo.file_stats(&changes, Some(&commit)).unwrap();
            let content = FileContent::load(&repo, &changes[previous], Some(&commit)).unwrap();
            let (handle, app) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| {
                        let mut app = test_app(window, cx);
                        app.view = View::Repositories;
                        app.repository = Some(repo.clone());
                        app.history.commits = vec![commit.clone()];
                        app.history.selected_commit = Some(0);
                        app.apply_changes(
                            Source::History,
                            PreparedChanges {
                                progress: Vec::new(),
                                changes,
                                stats,
                                preview: None,
                                preview_index: 0,
                            },
                            cx,
                        );
                        app.history.files.selected_file = Some(previous);
                        app.apply_file_result(Source::History, Ok(content), None, cx);
                        app
                    })
                })
                .unwrap()
            });
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                window.click("tree-file-1", cx);
            })
            .unwrap();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                let app = app.read(cx);
                assert!(app.repository_preview);
                assert_eq!(app.history.files.selected_file, Some(1));
                assert!(app.history.files.patch_loading);
                assert!(app.history.files.diff.read(cx).files().is_empty());
                assert!(app.history.files.image.is_none());
                assert!(window.find_all("source").is_empty());
                assert!(window.try_find("image-fit").is_none());
            })
            .unwrap();
            // The first selection starts I/O without a debounce timer, while
            // the in-flight frame above still hides the previous preview.
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert_eq!(
                    app.read(cx).history.files.diff.read(cx).files()[0]
                        .path()
                        .as_str(),
                    "beta.rs"
                );
                window.click("back-to-files", cx);
                window.render_frame(cx);
                window.click("tree-file-1", cx);
                window.render_frame(cx);
                assert!(!app.read(cx).history.files.patch_loading);
                assert_eq!(
                    app.read(cx).history.files.diff.read(cx).files()[0]
                        .path()
                        .as_str(),
                    "beta.rs"
                );

                // Switching within the preview still preserves its contents while loading.
                app.update(cx, |app, cx| app.select_file(0, cx));
                assert_eq!(
                    app.read(cx).history.files.diff.read(cx).files()[0]
                        .path()
                        .as_str(),
                    "beta.rs"
                );
                window.click("back-to-files", cx);
                window.render_frame(cx);
                window.click("tree-file-0", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                // Reentering the same pending selection must not expose the previous file either.
                assert!(app.read(cx).history.files.patch_loading);
                assert!(app.read(cx).history.files.diff.read(cx).files().is_empty());
                assert!(window.find_all("source").is_empty());
            })
            .unwrap();
            cx.executor().advance_clock(FILE_SELECTION_DELAY);
            cx.run_until_parked();
            cx.update(|cx| {
                assert_eq!(
                    app.read(cx).history.files.diff.read(cx).files()[0]
                        .path()
                        .as_str(),
                    "alpha.rs"
                );
            });
        }
    }

    #[gpui_kit::test]
    fn file_navigation_keeps_header_and_code_until_replacement_is_ready(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        std::fs::write(directory.path().join("next.rs"), "NEXT\n").unwrap();
        let root = directory.path().to_path_buf();
        let (handle, app) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut app = test_app(window, cx);
                    app.repository = Some(Repository::open(&root).unwrap());
                    app.working.loaded = true;
                    app.working.changes = ["current.rs", "next.rs"]
                        .map(|path| Change {
                            path: path.into(),
                            previous_path: None,
                            status: 'A',
                            area: Area::Untracked,
                        })
                        .to_vec();
                    app.working.selected_file = Some(0);
                    app.working.diff.update(cx, |diff, cx| {
                        diff.set_files(
                            DiffFile::parse(
                                "--- /dev/null\n+++ b/current.rs\n@@ -0,0 +1 @@\n+CURRENT\n",
                            )
                            .unwrap(),
                            cx,
                        )
                    });
                    app
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            // The adjacent resize handle must not consume the list scrollbar's clicks.
            app.update(cx, |app, cx| {
                app.active_files_mut()
                    .changes
                    .extend((2..42).map(|index| Change {
                        path: format!("file-{index}.rs").into(),
                        previous_path: None,
                        status: 'A',
                        area: Area::Untracked,
                    }));
                cx.notify();
            });
            window.render_frame(cx);
            window.render_frame(cx);
            let scroll = app.read(cx).active_files().file_scroll.clone();
            // Scrolling reveals the auto-hiding bar before interacting with it.
            scroll.set_offset(point(px(0.), -px(20.)));
            window.render_frame(cx);
            let bounds = scroll.viewport_bounds();
            let position = point(bounds.right() - px(3.), bounds.bottom() - px(8.));
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 1,
                    modifiers: Modifiers::default(),
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 1,
                    modifiers: Modifiers::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            assert!(
                scroll.offset().y < -px(20.),
                "the scrollbar at the pane edge must receive the click"
            );
            let header = window.find("next-file").bounds();
            app.update(cx, |app, cx| app.select_file(1, cx));
            assert!(app.read(cx).active_files().patch_loading);
            assert_eq!(
                app.read(cx).active_files().diff.read(cx).files()[0]
                    .path()
                    .as_str(),
                "current.rs"
            );
            window.render_frame(cx);
            assert_eq!(header, window.find("next-file").bounds());
            assert!(
                window
                    .find_all("source")
                    .iter()
                    .any(|line| line.label() == Some("CURRENT"))
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(!app.read(cx).active_files().patch_loading);
            assert_eq!(
                app.read(cx).active_files().diff.read(cx).files()[0]
                    .path()
                    .as_str(),
                "next.rs"
            );
        });
        // Repeated selection cancels the pending load, preserving the first preview.
        for index in [0, 1] {
            cx.run_until_parked();
            cx.executor().advance_clock(FILE_SELECTION_DELAY / 2);
            cx.update(|cx| app.update(cx, |app, cx| app.select_file(index, cx)));
        }
        cx.run_until_parked();
        cx.executor()
            .advance_clock(FILE_SELECTION_DELAY - Duration::from_millis(1));
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(app.read(cx).active_files().patch_loading);
            assert_eq!(
                app.read(cx).active_files().diff.read(cx).files()[0]
                    .path()
                    .as_str(),
                "next.rs"
            );
        });
        cx.executor().advance_clock(Duration::from_millis(1));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(!app.read(cx).active_files().patch_loading);
            assert_eq!(
                app.read(cx).active_files().diff.read(cx).files()[0]
                    .path()
                    .as_str(),
                "next.rs"
            );
            app.update(cx, |app, cx| app.select_file(1, cx));
            assert!(!app.read(cx).active_files().patch_loading);
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(
                window
                    .find_all("source")
                    .iter()
                    .any(|line| line.label() == Some("NEXT"))
            );
        })
        .unwrap();
        std::fs::write(
            directory.path().join("picture.png"),
            include_bytes!("../tests/fixtures/images/sample.png"),
        )
        .unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.active_files_mut().changes.push(Change {
                    path: "picture.png".into(),
                    previous_path: None,
                    status: 'A',
                    area: Area::Untracked,
                });
                app.select_file(app.active_files().changes.len() - 1, cx);
            });
            window.render_frame(cx);
            assert!(window.try_find("soft-wrap").is_some());
            assert!(app.read(cx).active_files().image.is_none());
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(FILE_SELECTION_DELAY);
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(app.read(cx).active_files().image.is_some());
            assert!(app.read(cx).active_files().diff.read(cx).files().is_empty());
            assert!(window.try_find("soft-wrap").is_none());
            assert!(window.try_find("image-fit").is_some());
            window.click("image-actual", cx);
            assert!(window.find("image-actual").checked().unwrap());
            app.update(cx, |app, cx| app.select_file(1, cx));
            window.render_frame(cx);
            assert!(window.try_find("image-fit").is_some());
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(FILE_SELECTION_DELAY);
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(app.read(cx).active_files().image.is_none());
            assert!(window.try_find("image-fit").is_none());
            assert!(window.try_find("soft-wrap").is_some());
        })
        .unwrap();
        // Repository mode keeps the same commit selected while opening and leaving a file.
        for args in [
            vec!["init", "-b", "main"],
            vec!["add", "--all"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "Repository overview",
                "-m",
                "Full commit message",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(directory.path())
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let repository = Repository::open(directory.path()).unwrap();
        let commits = repository.history(0, None).unwrap().commits;
        let branches = repository.branches().unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.preferences.remember_repository(repository.root.clone());
                app.repository = Some(repository);
                app.branches = branches;
                app.history.commits = commits;
                app.view = View::Repositories;
                app.select_commit(0, cx);
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert_eq!(app.read(cx).active_files().selected_file, None);
            assert!((repository_sidebar_ratio(window) - 0.5).abs() < 0.01);
            let repo_row = window.find(("repository", 0usize)).bounds();
            let commit_row = window.find(("commit", 0usize)).bounds();
            let file_row = window.find("tree-file-0").bounds();
            assert!(repo_row.right() <= commit_row.left());
            assert!(commit_row.right() <= file_row.left());
            // Selecting the message after searching must copy the message, not the input.
            app.update(cx, |app, cx| {
                app.history.search_available = true;
                app.history_search.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.set_value("search text", window, cx);
                });
            });
            window.render_frame(cx);
            window.render_frame(cx);
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(window.focused(cx), Some(app.read(cx).focus.clone()));
            assert_eq!(
                app.read(cx).history_search.read(cx).value().as_str(),
                "search text"
            );
            app.read(cx)
                .history_search
                .clone()
                .update(cx, |input, cx| input.focus(window, cx));
            window.render_frame(cx);
            let message = window.find("commit-message").bounds();
            let position = message.origin + point(px(30.), px(16.));
            window.dispatch_event(
                MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 2,
                    modifiers: Modifiers::default(),
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 2,
                    modifiers: Modifiers::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let selected = gpui_kit::base::TextSelection::selected_text(window, cx);
            assert_eq!(selected, "Repository");
            assert_eq!(window.focused(cx), Some(app.read(cx).focus.clone()));
            cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
            window.dispatch_action(Box::new(component::input::Copy), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "Repository"
            );
            window.click("copy-commit-id", cx);
            assert!(app.read(cx).commit_copied);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                app.read(cx).history.commits[0].id
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(COPY_FEEDBACK_DURATION);
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(!app.read(cx).commit_copied);
            window.click("tree-file-0", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(FILE_SELECTION_DELAY);
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(app.read(cx).active_files().selected_file.is_some());
            window.click("back-to-files", cx);
            assert!(!app.read(cx).repository_preview);
            assert_eq!(app.read(cx).active_files().selected_file, Some(0));
            assert_eq!(app.read(cx).history.selected_commit, Some(0));
            assert!(window.try_find("tree-file-0").is_some());
            app.update(cx, |app, cx| {
                app.view = View::History;
                app.select_file(0, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(FILE_SELECTION_DELAY);
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                let repo = app.repository.as_ref().unwrap();
                let prepared = PreparedChanges::load(
                    repo,
                    app.history.files.changes.clone(),
                    app.current_commit(),
                    Some(0),
                )
                .unwrap();
                app.apply_changes(Source::Working, prepared, cx);
            });
            for view in [View::History, View::Working] {
                app.update(cx, |app, cx| {
                    app.view = view;
                    app.preferences.show_file_stats = false;
                    cx.notify();
                });
                window.render_frame(cx);
                window.render_frame(cx);
                let selected = app.read(cx).active_files().selected_file;
                let diff_path = app.read(cx).active_files().diff.read(cx).files()[0]
                    .path()
                    .clone();
                assert!(window.find_all("file-diff-stats").is_empty());
                window.click("toggle-file-stats", cx);
                window.render_frame(cx);
                assert!(!window.find_all("file-diff-stats").is_empty());
                let total_right = window.find("change-totals").bounds().right();
                for stats in window.find_all("file-diff-stats") {
                    assert!((stats.bounds().right() - total_right).abs() < px(1.));
                }
                window.click("toggle-file-list", cx);
                window.render_frame(cx);
                window.render_frame(cx);
                assert!(window.try_find("tree-file-0").is_some());
                assert!(app.read(cx).preferences.tree_file_list);
                let total_right = window.find("change-totals").bounds().right();
                for stats in window.find_all("file-diff-stats") {
                    assert!((stats.bounds().right() - total_right).abs() < px(1.));
                }
                window.click("toggle-file-list", cx);
                window.render_frame(cx);
                assert!(window.try_find(("file", 0usize)).is_some());
                assert_eq!(app.read(cx).active_files().selected_file, selected);
                assert!(!app.read(cx).active_files().patch_loading);
                assert_eq!(
                    app.read(cx).active_files().diff.read(cx).files()[0].path(),
                    &diff_path
                );
            }
        })
        .unwrap();
        // The footer toggle is available in both history layouts and keeps parsed content intact.
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.switch_view(View::History, cx));
            window.render_frame(cx);
            let diff = app.read(cx).history.files.diff.entity_id();
            let selected = app.read(cx).history.selected_commit;
            assert!(window.find("toggle-commit-message").checked().unwrap());
            assert!(window.try_find("commit-message").is_some());
            window.click("toggle-commit-message", cx);
            window.render_frame(cx);
            assert!(!window.find("toggle-commit-message").checked().unwrap());
            assert!(window.try_find("commit-message").is_none());
            let saved: Preferences = serde_json::from_slice(
                &std::fs::read(
                    app.read(cx)
                        .preferences_directory
                        .path()
                        .join("preferences.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert!(!saved.show_commit_message);
            app.update(cx, |app, cx| app.switch_view(View::Repositories, cx));
            window.render_frame(cx);
            assert!(window.try_find("commit-message").is_none());
            window.click("toggle-commit-message", cx);
            window.render_frame(cx);
            assert!(window.try_find("commit-message").is_some());
            assert_eq!(app.read(cx).history.files.diff.entity_id(), diff);
            assert_eq!(app.read(cx).history.selected_commit, selected);
            assert!(!app.read(cx).history.files.patch_loading);
            app.update(cx, |app, cx| app.switch_view(View::Working, cx));
            window.render_frame(cx);
            assert!(window.try_find("toggle-commit-message").is_none());
            assert!(window.try_find("commit-message").is_none());
            window.click("soft-wrap", cx);
            window.render_frame(cx);
            let saved: Preferences = serde_json::from_slice(
                &std::fs::read(
                    app.read(cx)
                        .preferences_directory
                        .path()
                        .join("preferences.json"),
                )
                .unwrap(),
            )
            .unwrap();
            let current = &app.read(cx).preferences;
            assert_eq!(saved.soft_wrap, current.soft_wrap);
            assert_eq!(saved.tree_file_list, current.tree_file_list);
            assert_eq!(saved.show_file_stats, current.show_file_stats);
            assert_eq!(saved.show_commit_message, current.show_commit_message);
        })
        .unwrap();
        // Drain menu dismissal between languages, as the native event loop does.
        let original_locale = component::locale().to_string();
        let (selected, diff_path) = cx.update(|cx| {
            (
                app.read(cx).active_files().selected_file,
                app.read(cx).active_files().diff.read(cx).files()[0]
                    .path()
                    .clone(),
            )
        });
        for language in Language::ALL {
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| {
                    app.preferences.language = language;
                    apply_language(language, cx);
                    cx.notify();
                });
                window.render_frame(cx);
                assert_eq!(
                    window.find("open-repository").label(),
                    Some(tr("open_repository"))
                );
                assert_eq!(window.find("soft-wrap").label(), Some(tr("soft_wrap")));
                assert_eq!(app.read(cx).active_files().selected_file, selected);
                assert_eq!(
                    app.read(cx).active_files().diff.read(cx).files()[0].path(),
                    &diff_path
                );
                assert!(!app.read(cx).active_files().patch_loading);
                window.click("language", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    window.try_find("popup-menu").is_some(),
                    "Menu did not open for {language:?}"
                );
                for (index, item_language) in Language::ALL.into_iter().enumerate() {
                    let item = window.within("popup-menu").find(index);
                    assert_eq!(item.label(), Some(language_name(item_language)));
                    assert!(item.bounds().right() <= window.viewport_size().width);
                    assert!(item.bounds().bottom() <= window.viewport_size().height);
                }
                window.press("escape", cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
        component::set_locale(&original_locale);
        // Commit changes and their first diff replace the old snapshot together.
        std::fs::write(directory.path().join("later.rs"), "LATER\n").unwrap();
        for args in [
            vec!["add", "later.rs"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "Later commit",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(directory.path())
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let latest = Repository::open(directory.path())
            .unwrap()
            .history(0, None)
            .unwrap()
            .commits
            .remove(0);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.view = View::History;
                app.history.commits.push(latest);
                let files = app
                    .active_files()
                    .changes
                    .iter()
                    .map(|change| change.path.clone())
                    .collect::<Vec<_>>();
                let old_patch = app.active_files_mut().diff.read(cx).files()[0]
                    .path()
                    .clone();
                app.select_commit(1, cx);
                assert_eq!(app.history.selected_commit, Some(0));
                assert_eq!(
                    app.active_files_mut()
                        .changes
                        .iter()
                        .map(|change| change.path.clone())
                        .collect::<Vec<_>>(),
                    files
                );
                assert_eq!(
                    app.active_files_mut().diff.read(cx).files()[0].path(),
                    &old_patch
                );
            });
            window.render_frame(cx);
            assert!(
                window
                    .find_all("source")
                    .iter()
                    .any(|line| line.label() == Some("NEXT"))
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(app.read(cx).history.selected_commit, Some(1));
            assert_eq!(
                app.read(cx).active_files().changes[0].path,
                PathBuf::from("later.rs")
            );
            assert_eq!(
                app.read(cx).active_files().diff.read(cx).files()[0]
                    .path()
                    .as_str(),
                "later.rs"
            );
            assert!(!app.read(cx).active_files().patch_loading);
            app.update(cx, |app, cx| {
                app.load(directory.path().join("missing-repository"), cx)
            });
            assert_eq!(app.read(cx).history.selected_commit, Some(1));
            window.render_frame(cx);
            assert!(
                window
                    .find_all("source")
                    .iter()
                    .any(|line| line.label() == Some("LATER"))
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(app.read(cx).error.is_some());
            assert_eq!(app.read(cx).history.selected_commit, Some(1));
            assert_eq!(
                app.read(cx).active_files().diff.read(cx).files()[0]
                    .path()
                    .as_str(),
                "later.rs"
            );
        });
        // A new window gets its initial ratio from serialized preferences, not old layout state.
        cx.update(|cx| {
            app.update(cx, |app, _| {
                app.view = View::Repositories;
                app.error = None;
                app.preferences.repository_sidebar_ratio = Some(0.35);
                app.preferences =
                    serde_json::from_slice(&serde_json::to_vec(&app.preferences).unwrap()).unwrap();
            })
        });
        let (restored, _) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, _| app.clone()).unwrap()
        });
        cx.update_window(restored, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!((repository_sidebar_ratio(window) - 0.35).abs() < 0.01);
        })
        .unwrap();
        // Tab switches retain separate snapshots, and Repos/History share the same one.
        let (working_diff, history_diff, commit) = cx.update(|cx| {
            let app = app.read(cx);
            (
                app.working.diff.entity_id(),
                app.history.files.diff.entity_id(),
                app.history.selected_commit,
            )
        });
        for target in [
            View::Working,
            View::History,
            View::Repositories,
            View::History,
            View::Working,
        ] {
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |app, cx| app.switch_view(target, cx));
                window.render_frame(cx);
                let app = app.read(cx);
                assert!(!app.loading);
                assert_eq!(app.working.diff.entity_id(), working_diff);
                assert_eq!(app.history.files.diff.entity_id(), history_diff);
                assert_eq!(app.history.selected_commit, commit);
                if target == View::Working {
                    assert!(window.try_find("commit-message").is_none());
                }
            })
            .unwrap();
        }
    }
}
