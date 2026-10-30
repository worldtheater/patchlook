//! Working review keeps content differences separate from index inclusion.
use super::staging::quoted_path;
use super::*;
use std::sync::Arc;

/// One file in the working list, even when both index and worktree differ.
pub fn merge_working_changes(changes: &[Change]) -> Vec<Change> {
    let mut result: Vec<Change> = Vec::new();
    for change in changes {
        if let Some(file) = result.iter_mut().find(|file| file.path == change.path) {
            file.area = Area::Partial;
            if file.previous_path.is_none() {
                file.previous_path = change.previous_path.clone();
            }
            if change.status == 'D' {
                file.status = 'D';
            } else if file.status == 'D' && change.area == Area::Untracked {
                // A staged deletion followed by recreating the path is still tracked by HEAD.
                file.status = 'M';
            }
        } else {
            result.push(change.clone());
        }
    }
    result
}

struct VersionPatch {
    change: Change,
    raw: String,
    text: Option<Arc<StagingPatch>>,
}

struct BlockAction {
    version: usize,
    block: usize,
}

struct ReviewPart {
    raw: String,
    text: Option<StagingPatch>,
    blocks: Vec<usize>,
    area: Option<Area>,
}

/// Normally HEAD → worktree, with index inclusion attached to each block.
/// When index contains a third version, two explicitly labelled comparisons preserve it.
pub struct ReviewPatch {
    versions: Vec<VersionPatch>,
    parts: Vec<ReviewPart>,
    actions: Vec<BlockAction>,
}

impl Repository {
    pub fn review_patch(&self, change: &Change) -> Result<ReviewPatch> {
        self.review_patch_from_changes(change, &self.working_changes()?)
    }

    /// Reuse the status already read for this refresh; write validation always reads afresh.
    pub fn review_patch_from_changes(
        &self,
        change: &Change,
        changes: &[Change],
    ) -> Result<ReviewPatch> {
        let entries = changes
            .iter()
            .filter(|entry| entry.path == change.path)
            .cloned()
            .collect::<Vec<_>>();
        ensure!(
            !entries.is_empty(),
            "File status changed; refresh before reviewing"
        );
        let versions = entries
            .into_iter()
            .map(|entry| {
                let raw = self.patch(&entry, None)?;
                let text = (entry.area != Area::Conflict)
                    .then(|| StagingPatch::parse(raw.clone()))
                    .flatten()
                    .map(Arc::new);
                Ok(VersionPatch {
                    change: entry,
                    raw,
                    text,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut review = ReviewPatch {
            versions,
            parts: Vec::new(),
            actions: Vec::new(),
        };
        if review.versions.len() == 1 {
            review.push_version(0)?;
            return Ok(review);
        }
        // HEAD includes the original rename path. An unborn repository has no base tree;
        // its index/worktree comparisons are retained explicitly below.
        if self.has_head()? {
            let combined = self.patch(
                &Change {
                    area: Area::Partial,
                    ..change.clone()
                },
                None,
            )?;
            if let Some(text) = StagingPatch::parse(combined.clone()) {
                let sources = review
                    .versions
                    .iter()
                    .map(|version| {
                        version.text.as_ref().map(|patch| {
                            signatures(patch, version.change.area == Area::Staged)
                                .into_iter()
                                .enumerate()
                                .map(|(i, key)| (key, i))
                                .collect::<HashMap<_, _>>()
                        })
                    })
                    .collect::<Vec<_>>();
                let original = signatures(&text, true);
                let modified = signatures(&text, false);
                let mut actions = Vec::new();
                for block in 0..text.blocks.len() {
                    let matches = review
                        .versions
                        .iter()
                        .enumerate()
                        .filter_map(|(version, source)| {
                            let key = if source.change.area == Area::Staged {
                                &original[block]
                            } else {
                                &modified[block]
                            };
                            sources[version]
                                .as_ref()?
                                .get(key)
                                .map(|&block| BlockAction { version, block })
                        })
                        .collect::<Vec<_>>();
                    if matches.len() != 1 {
                        break;
                    }
                    actions.extend(matches);
                }
                let underlying = review
                    .versions
                    .iter()
                    .map(|v| v.text.as_ref().map_or(0, |p| p.blocks.len()))
                    .sum::<usize>();
                let unique = actions
                    .iter()
                    .map(|a| (a.version, a.block))
                    .collect::<std::collections::BTreeSet<_>>();
                if actions.len() == text.blocks.len()
                    && unique.len() == underlying
                    && review.versions.iter().all(|v| v.text.is_some())
                {
                    review.parts.push(ReviewPart {
                        raw: combined,
                        text: Some(text),
                        blocks: (0..actions.len()).collect(),
                        area: None,
                    });
                    review.actions = actions;
                    return Ok(review);
                }
            }
        }
        for version in 0..review.versions.len() {
            review.push_version(version)?;
        }
        Ok(review)
    }

    pub fn set_review_block(
        &self,
        mode: AccessMode,
        review: &ReviewPatch,
        block: usize,
    ) -> Result<()> {
        ensure!(
            mode == AccessMode::ReadWrite,
            "Repository is in read-only mode"
        );
        review.validate(self)?;
        let action = review.actions.get(block).context("Unknown review block")?;
        let version = &review.versions[action.version];
        let patch = version
            .text
            .as_ref()
            .context("No text patch")?
            .partial_patch(
                &version.change,
                StageSelection::Block(action.block),
                version.change.area == Area::Staged,
            )?;
        self.apply_patch(&patch, true)
    }

    pub fn discard_review_block(
        &self,
        mode: AccessMode,
        review: &ReviewPatch,
        block: usize,
    ) -> Result<()> {
        ensure!(
            mode == AccessMode::ReadWrite,
            "Repository is in read-only mode"
        );
        review.validate(self)?;
        let action = review.actions.get(block).context("Unknown review block")?;
        let version = &review.versions[action.version];
        ensure!(
            matches!(version.change.area, Area::Unstaged | Area::Untracked),
            "Only unstaged changes can be discarded"
        );
        let patch = version
            .text
            .as_ref()
            .context("No text patch")?
            .partial_patch(&version.change, StageSelection::Block(action.block), true)?;
        self.apply_patch(&patch, false)
    }
}

impl ReviewPatch {
    fn push_version(&mut self, version: usize) -> Result<()> {
        let source = &self.versions[version];
        let split = self.versions.len() > 1;
        let raw = if split {
            display_patch(&source.raw, &source.change, version)?
        } else {
            source.raw.clone()
        };
        let text = StagingPatch::parse(raw.clone());
        let blocks = (0..source.text.as_ref().map_or(0, |p| p.blocks.len()))
            .map(|block| {
                let index = self.actions.len();
                self.actions.push(BlockAction { version, block });
                index
            })
            .collect();
        self.parts.push(ReviewPart {
            raw,
            text,
            blocks,
            area: split.then_some(source.change.area),
        });
        Ok(())
    }

    pub fn patches(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().map(|part| part.raw.as_str())
    }
    /// Count the underlying index/worktree changes, including third versions,
    /// from the text already parsed for review. Other file types still use numstat.
    pub fn text_stats(&self) -> Option<DiffStats> {
        let mut stats = DiffStats::default();
        for version in &self.versions {
            for line in &version.text.as_ref()?.lines {
                match line.kind {
                    b'+' => stats.additions += 1,
                    b'-' => stats.deletions += 1,
                    _ => {}
                }
            }
        }
        Some(stats)
    }
    pub fn separate_versions(&self) -> bool {
        self.parts.len() > 1
    }
    pub fn part_area(&self, part: usize) -> Option<Area> {
        self.parts.get(part).and_then(|part| part.area)
    }
    pub fn progress(&self) -> (usize, usize) {
        (
            (0..self.actions.len())
                .filter(|&block| self.is_staged(block))
                .count(),
            self.actions.len(),
        )
    }
    pub fn is_staged(&self, block: usize) -> bool {
        self.actions
            .get(block)
            .is_some_and(|action| self.versions[action.version].change.area == Area::Staged)
    }
    pub fn block_target(
        &self,
        part: usize,
        original: bool,
        line: usize,
        prefer_original: bool,
    ) -> Option<usize> {
        let part = self.parts.get(part)?;
        let local = part
            .text
            .as_ref()?
            .block_target(original, line, prefer_original)?;
        part.blocks.get(local).copied()
    }
    pub fn line_staged(&self, part: usize, original: bool, line: usize) -> bool {
        self.parts
            .get(part)
            .and_then(|part| {
                let local = part.text.as_ref()?.block_for_line(original, line)?;
                part.blocks.get(local)
            })
            .is_some_and(|&block| self.is_staged(block))
    }
    fn validate(&self, repo: &Repository) -> Result<()> {
        let current = repo
            .working_changes()?
            .into_iter()
            .filter(|entry| entry.path == self.versions[0].change.path)
            .collect::<Vec<_>>();
        ensure!(
            current
                == self
                    .versions
                    .iter()
                    .map(|v| v.change.clone())
                    .collect::<Vec<_>>(),
            "File status changed; refresh before staging"
        );
        for version in &self.versions {
            ensure!(
                repo.patch(&version.change, None)? == version.raw,
                "File contents changed; refresh before staging"
            );
        }
        Ok(())
    }
}

/// Coordinates on the unchanged version distinguish repeated, identical edits.
type BlockSignature<'a> = (usize, Vec<(u8, &'a str)>);

fn signatures(patch: &StagingPatch, original: bool) -> Vec<BlockSignature<'_>> {
    let omitted = if original { b'+' } else { b'-' };
    let (mut cursor, mut line_number) = (0, 0);
    patch
        .blocks
        .iter()
        .map(|range| {
            line_number += patch.lines[cursor..range.start]
                .iter()
                .filter(|line| line.kind != omitted)
                .count();
            let start = line_number;
            let lines = patch.lines[range.clone()]
                .iter()
                .map(|line| (line.kind, line.text.as_str()))
                .collect();
            line_number += patch.lines[range.clone()]
                .iter()
                .filter(|line| line.kind != omitted)
                .count();
            cursor = range.end;
            (start, lines)
        })
        .collect()
}

/// Distinct identities for two comparisons of the same path, used only by the viewer.
/// Git mutations always use the original patch and literal repository path.
fn display_patch(raw: &str, change: &Change, version: usize) -> Result<String> {
    let path = PathBuf::from(format!("version-{version}")).join(&change.path);
    let old = quoted_path("a/", &path)?;
    let new = quoted_path("b/", &path)?;
    let mut result = String::new();
    let mut body = false;
    for line in raw.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            body = true;
        }
        if body {
            result.push_str(line);
        } else if line.starts_with("diff --git ") {
            result.push_str(&format!("diff --git {old} {new}\n"));
        } else if line.starts_with("--- ") && !line.starts_with("--- /dev/null") {
            result.push_str(&format!("--- {old}\n"));
        } else if line.starts_with("+++ ") && !line.starts_with("+++ /dev/null") {
            result.push_str(&format!("+++ {new}\n"));
        } else {
            result.push_str(line);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{commit, fixture, run};

    fn review(repo: &Repository) -> ReviewPatch {
        let raw_changes = repo.working_changes().unwrap();
        let changes = merge_working_changes(&raw_changes);
        assert_eq!(changes.len(), 1);
        let review = repo.review_patch(&changes[0]).unwrap();
        assert_eq!(
            review.text_stats(),
            Some(repo.change_stats(&raw_changes, None).unwrap())
        );
        review
    }

    #[test]
    fn staging_keeps_all_blocks_and_unstaging_restores_only_the_selected_block() {
        let (directory, repo) = fixture();
        std::fs::write(directory.path().join("f"), "a\nb\nc\nd\ne\n").unwrap();
        commit(&repo, "base");
        let working = "a\nB\nc\nD\ne\n";
        std::fs::write(directory.path().join("f"), working).unwrap();
        let first = review(&repo);
        let raw = first.patches().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(first.progress(), (0, 2));
        repo.set_review_block(AccessMode::ReadWrite, &first, 0)
            .unwrap();
        let partial = review(&repo);
        assert!(!partial.separate_versions());
        assert_eq!(partial.progress(), (1, 2));
        assert_eq!(partial.patches().collect::<Vec<_>>(), raw);
        assert!(partial.line_staged(0, false, 2));
        assert!(!partial.line_staged(0, false, 4));
        repo.set_review_block(AccessMode::ReadWrite, &partial, 1)
            .unwrap();
        let all = review(&repo);
        assert_eq!(all.progress(), (2, 2));
        assert_eq!(all.patches().collect::<Vec<_>>(), raw);
        repo.set_review_block(AccessMode::ReadWrite, &all, 0)
            .unwrap();
        let remaining = review(&repo);
        assert_eq!(remaining.progress(), (1, 2));
        assert!(!remaining.is_staged(0));
        assert!(remaining.is_staged(1));
        assert_eq!(repo.index_file(Path::new("f")).unwrap(), b"a\nb\nc\nD\ne\n");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("f")).unwrap(),
            working
        );
    }

    #[test]
    fn third_versions_and_index_only_changes_are_not_hidden() {
        for working in ["a\nTHIRD\nc\n", "a\nb\nc\n"] {
            let (directory, repo) = fixture();
            std::fs::write(directory.path().join("f"), "a\nb\nc\n").unwrap();
            commit(&repo, "base");
            std::fs::write(directory.path().join("f"), "a\nINDEX\nc\n").unwrap();
            run(directory.path(), &["add", "f"]);
            std::fs::write(directory.path().join("f"), working).unwrap();
            let current = review(&repo);
            assert!(current.separate_versions());
            assert_eq!(current.progress(), (1, 2));
            assert_eq!(current.part_area(0), Some(Area::Staged));
            assert_eq!(current.part_area(1), Some(Area::Unstaged));
            for raw in current.patches() {
                let files = gpui_kit::component::diff::DiffFile::parse(raw).unwrap();
                assert_eq!(files.len(), 1);
                assert!(files[0].change_block_count() > 0);
            }
            repo.discard_review_block(AccessMode::ReadWrite, &current, 1)
                .unwrap();
            assert_eq!(repo.index_file(Path::new("f")).unwrap(), b"a\nINDEX\nc\n");
            assert_eq!(
                std::fs::read(directory.path().join("f")).unwrap(),
                b"a\nINDEX\nc\n"
            );
            assert_eq!(review(&repo).progress(), (1, 1));
        }
    }

    #[test]
    fn review_rejects_changes_to_either_version_and_read_only_writes() {
        let (directory, repo) = fixture();
        std::fs::write(directory.path().join("f"), "a\nb\nc\nd\n").unwrap();
        commit(&repo, "base");
        std::fs::write(directory.path().join("f"), "a\nB\nc\nd\n").unwrap();
        run(directory.path(), &["add", "f"]);
        std::fs::write(directory.path().join("f"), "a\nB\nc\nD\n").unwrap();
        let displayed = review(&repo);
        assert!(
            repo.set_review_block(AccessMode::ReadOnly, &displayed, 1)
                .is_err()
        );
        std::fs::write(directory.path().join("f"), "a\nCHANGED\nc\nD\n").unwrap();
        assert!(
            repo.set_review_block(AccessMode::ReadWrite, &displayed, 0)
                .is_err()
        );
        assert_eq!(repo.index_file(Path::new("f")).unwrap(), b"a\nB\nc\nd\n");
    }

    #[test]
    fn repeated_edits_use_source_coordinates_not_only_text() {
        let (directory, repo) = fixture();
        std::fs::write(directory.path().join("f"), "before\nx\nmiddle\nx\nafter\n").unwrap();
        commit(&repo, "base");
        std::fs::write(directory.path().join("f"), "before\ny\nmiddle\ny\nafter\n").unwrap();
        repo.set_review_block(AccessMode::ReadWrite, &review(&repo), 1)
            .unwrap();
        let current = review(&repo);
        assert!(!current.separate_versions());
        assert_eq!(current.progress(), (1, 2));
        assert!(!current.is_staged(0));
        assert!(current.is_staged(1));
    }

    #[test]
    fn recreated_staged_deletion_keeps_the_latest_file_and_can_stage_or_unstage() {
        let (directory, repo) = fixture();
        std::fs::write(directory.path().join("f"), "base\n").unwrap();
        commit(&repo, "base");
        run(directory.path(), &["rm", "f"]);
        std::fs::write(directory.path().join("f"), "recreated\n").unwrap();
        let entries = merge_working_changes(&repo.snapshot().unwrap().changes);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].area, Area::Partial);
        assert_eq!(entries[0].status, 'M');
        assert!(repo.review_patch(&entries[0]).unwrap().separate_versions());
        repo.set_staged(
            AccessMode::ReadWrite,
            &entries[0],
            StageSelection::File,
            None,
        )
        .unwrap();
        assert_eq!(repo.index_file(Path::new("f")).unwrap(), b"recreated\n");
        let staged = merge_working_changes(&repo.snapshot().unwrap().changes).remove(0);
        repo.set_staged(AccessMode::ReadWrite, &staged, StageSelection::File, None)
            .unwrap();
        assert_eq!(repo.index_file(Path::new("f")).unwrap(), b"base\n");
        assert_eq!(
            std::fs::read(directory.path().join("f")).unwrap(),
            b"recreated\n"
        );
    }

    #[test]
    fn unborn_partial_file_and_deleting_a_staged_addition_keep_index_contents_available() {
        let (directory, repo) = fixture();
        std::fs::write(directory.path().join("f"), "staged\n").unwrap();
        run(directory.path(), &["add", "f"]);
        std::fs::write(directory.path().join("f"), "latest\n").unwrap();
        assert!(review(&repo).separate_versions());
        let partial = merge_working_changes(&repo.snapshot().unwrap().changes).remove(0);
        repo.set_staged(AccessMode::ReadWrite, &partial, StageSelection::File, None)
            .unwrap();
        assert_eq!(repo.index_file(Path::new("f")).unwrap(), b"latest\n");
        std::fs::remove_file(directory.path().join("f")).unwrap();
        let deleted = merge_working_changes(&repo.snapshot().unwrap().changes).remove(0);
        assert_eq!(deleted.status, 'D');
        assert_eq!(
            repo.file_versions(&deleted, None)
                .unwrap()
                .original
                .unwrap(),
            b"latest\n"
        );
        let current = review(&repo);
        assert_eq!(current.progress(), (1, 2));
        repo.discard_review_block(AccessMode::ReadWrite, &current, 1)
            .unwrap();
        assert_eq!(
            std::fs::read(directory.path().join("f")).unwrap(),
            b"latest\n"
        );
    }
}
