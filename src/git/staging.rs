use super::*;
use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

/// Write access is deliberately session-only: every new window starts read-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    #[default]
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageSelection {
    File,
    Block(usize),
}

#[derive(Debug)]
pub(super) struct PatchLine {
    pub(super) kind: u8,
    pub(super) text: String,
}

/// A complete-context patch, with source coordinates shared by both diff layouts.
#[derive(Debug)]
pub struct StagingPatch {
    pub(super) raw: String,
    pub(super) lines: Vec<PatchLine>,
    positions: HashMap<(bool, usize), usize>,
    pub(super) blocks: Vec<Range<usize>>,
    mode: String,
}

impl StagingPatch {
    /// Metadata-only, binary, symlink, submodule and conflict diffs have no block actions.
    pub fn parse(raw: String) -> Option<Self> {
        let mut lines: Vec<PatchLine> = Vec::new();
        let mut positions = HashMap::new();
        let mut mode = "100644".to_owned();
        let mut in_hunk = false;
        let (mut old, mut new) = (1, 1);
        for line in raw.split_inclusive('\n') {
            if line.starts_with("@@ ") {
                // Repository::patch supplies the entire file in one hunk.
                if in_hunk {
                    return None;
                }
                let mut parts = line.split_whitespace();
                parts.next()?;
                let old_start = parts.next()?.strip_prefix('-')?.split(',').next()?;
                let new_start = parts.next()?.strip_prefix('+')?.split(',').next()?;
                if !matches!(old_start, "0" | "1") || !matches!(new_start, "0" | "1") {
                    return None;
                }
                in_hunk = true;
                continue;
            }
            if !in_hunk {
                if let Some(value) = line
                    .strip_prefix("new file mode ")
                    .or_else(|| line.strip_prefix("deleted file mode "))
                    .or_else(|| line.strip_prefix("old mode "))
                    .or_else(|| line.strip_prefix("new mode "))
                {
                    mode = value.trim().to_owned();
                    if !matches!(mode.as_str(), "100644" | "100755") {
                        return None;
                    }
                } else if line.starts_with("index ")
                    && let Some(value) = line.split_whitespace().nth(2)
                {
                    mode = value.to_owned();
                }
                continue;
            }
            if line.starts_with("\\ No newline at end of file") {
                let previous = lines.last_mut()?;
                previous.text = previous.text.strip_suffix('\n')?.to_owned();
                continue;
            }
            let kind = *line.as_bytes().first()?;
            if !matches!(kind, b' ' | b'+' | b'-') {
                return None;
            }
            let index = lines.len();
            if kind != b'+' {
                if kind == b'-' {
                    positions.insert((true, old), index);
                }
                old += 1;
            }
            if kind != b'-' {
                if kind == b'+' {
                    positions.insert((false, new), index);
                }
                new += 1;
            }
            lines.push(PatchLine {
                kind,
                text: line[1..].to_owned(),
            });
        }
        if !in_hunk || !matches!(mode.as_str(), "100644" | "100755") {
            return None;
        }
        let mut blocks: Vec<Range<usize>> = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if line.kind == b' ' {
                continue;
            }
            if let Some(last) = blocks.last_mut().filter(|last| last.end == index) {
                last.end += 1;
            } else {
                blocks.push(index..index + 1);
            }
        }
        Some(Self {
            raw,
            lines,
            positions,
            blocks,
            mode,
        })
    }

    /// One anchor per contiguous change block, on the preferred side if it exists.
    /// A missing side uses the opposite source row (pure additions/deletions).
    pub fn block_target(
        &self,
        original: bool,
        line: usize,
        prefer_original: bool,
    ) -> Option<usize> {
        let index = *self.positions.get(&(original, line))?;
        self.blocks.iter().position(|range| {
            let kind = if prefer_original { b'-' } else { b'+' };
            let anchor = range
                .clone()
                .find(|&i| self.lines[i].kind == kind)
                .unwrap_or(range.start);
            anchor == index
        })
    }

    pub(super) fn block_for_line(&self, original: bool, line: usize) -> Option<usize> {
        let index = *self.positions.get(&(original, line))?;
        self.blocks.iter().position(|range| range.contains(&index))
    }

    fn selected(&self, selection: StageSelection) -> Result<Range<usize>> {
        match selection {
            StageSelection::File => Ok(0..self.lines.len()),
            StageSelection::Block(index) => self
                .blocks
                .get(index)
                .cloned()
                .context("Unknown change block"),
        }
    }

    fn partial_patch(
        &self,
        change: &Change,
        selection: StageSelection,
        reverse: bool,
    ) -> Result<String> {
        let selected = self.selected(selection)?;
        let mut body = String::new();
        let (mut old_count, mut new_count) = (0, 0);
        for (index, line) in self.lines.iter().enumerate() {
            let kind = match (reverse, selected.contains(&index), line.kind) {
                (_, _, b' ') => b' ',
                (false, true, kind) => kind,
                (false, false, b'-') | (true, false, b'+') => b' ',
                (true, true, b'+') => b'-',
                (true, true, b'-') => b'+',
                _ => continue,
            };
            old_count += usize::from(kind != b'+');
            new_count += usize::from(kind != b'-');
            body.push(kind as char);
            body.push_str(&line.text);
            if !line.text.ends_with('\n') {
                body.push_str("\n\\ No newline at end of file\n");
            }
        }
        let base_missing = if reverse {
            change.status == 'D'
        } else {
            change.area == Area::Untracked || change.status == 'A'
        };
        let all_selected = self
            .lines
            .iter()
            .enumerate()
            .all(|(i, line)| line.kind == b' ' || selected.contains(&i));
        let result_missing = all_selected
            && if reverse {
                change.status == 'A' || change.area == Area::Untracked
            } else {
                change.status == 'D'
            };
        // For a staged rename, undo content at its current index path, preserving the rename.
        let path = if reverse {
            &change.path
        } else {
            change.previous_path.as_ref().unwrap_or(&change.path)
        };
        let old_path = quoted_path("a/", path)?;
        let new_path = quoted_path("b/", path)?;
        let mut patch = format!("diff --git {old_path} {new_path}\n");
        if base_missing {
            patch.push_str(&format!("new file mode {}\n", self.mode));
        }
        if result_missing {
            patch.push_str(&format!("deleted file mode {}\n", self.mode));
        }
        patch.push_str(&format!(
            "--- {}\n+++ {}\n@@ -{},{} +{},{} @@\n{body}",
            if base_missing { "/dev/null" } else { &old_path },
            if result_missing {
                "/dev/null"
            } else {
                &new_path
            },
            usize::from(old_count != 0),
            old_count,
            usize::from(new_count != 0),
            new_count
        ));
        Ok(patch)
    }
}

pub(super) fn quoted_path(prefix: &str, path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .context("Partial staging requires a UTF-8 path")?;
    #[cfg(windows)]
    let path = path.replace('\\', "/");
    let mut quoted = String::from("\"");
    for byte in prefix.bytes().chain(path.bytes()) {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            32..=126 => quoted.push(byte as char),
            _ => quoted.push_str(&format!("\\{byte:03o}")),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

impl Repository {
    /// Only changes the index. Partial requests are checked against the displayed patch.
    pub fn set_staged(
        &self,
        mode: AccessMode,
        change: &Change,
        selection: StageSelection,
        displayed: Option<&StagingPatch>,
    ) -> Result<()> {
        if selection == StageSelection::File {
            return self.set_files_staged(
                mode,
                std::slice::from_ref(change),
                change.area != Area::Staged,
            );
        }
        ensure!(
            mode == AccessMode::ReadWrite,
            "Repository is in read-only mode"
        );
        ensure!(
            change.area != Area::Conflict,
            "Resolve merge conflicts before staging in Patchlook"
        );
        ensure!(
            self.snapshot()?.changes.contains(change),
            "File status changed; refresh before staging"
        );
        let displayed = displayed.context("No text patch available for partial staging")?;
        ensure!(
            self.patch(change, None)? == displayed.raw,
            "File contents changed; refresh before staging"
        );
        let patch = displayed.partial_patch(change, selection, change.area == Area::Staged)?;
        self.apply_patch(&patch, true)
    }

    /// Stages or unstages exactly the supplied file entries in one Git operation.
    /// Already-satisfied entries are skipped, including staged halves of mixed files.
    pub fn set_files_staged(
        &self,
        mode: AccessMode,
        changes: &[Change],
        staged: bool,
    ) -> Result<()> {
        ensure!(
            mode == AccessMode::ReadWrite,
            "Repository is in read-only mode"
        );
        ensure!(
            changes.iter().all(|change| change.area != Area::Conflict),
            "Resolve merge conflicts before staging in Patchlook"
        );
        let current = self.snapshot()?.changes;
        let merged = merge_working_changes(&current);
        ensure!(
            changes
                .iter()
                .all(|change| current.contains(change) || merged.contains(change)),
            "File status changed; refresh before staging"
        );
        let selected = current.iter().filter(|entry| {
            changes
                .iter()
                .any(|c| c == *entry || (c.area == Area::Partial && c.path == entry.path))
        });
        let mut paths = BTreeSet::new();
        for change in selected.filter(|change| (change.area != Area::Staged) == staged) {
            if change.status == 'R'
                && let Some(previous) = &change.previous_path
            {
                paths.insert(previous);
            }
            paths.insert(&change.path);
        }
        if paths.is_empty() {
            return Ok(());
        }
        let mut cmd = command(&self.root);
        if staged {
            cmd.args(["add", "-A", "--"]);
        } else if self.has_head()? {
            cmd.args(["reset", "-q", "HEAD", "--"]);
        } else {
            cmd.args(["rm", "--cached", "-f", "--"]);
        }
        cmd.args(paths);
        output(&mut cmd, false)?;
        Ok(())
    }

    /// Restores one unstaged block from the index, leaving the index untouched.
    pub fn discard_block(
        &self,
        mode: AccessMode,
        change: &Change,
        block: usize,
        displayed: &StagingPatch,
    ) -> Result<()> {
        ensure!(
            mode == AccessMode::ReadWrite,
            "Repository is in read-only mode"
        );
        ensure!(
            matches!(change.area, Area::Unstaged | Area::Untracked),
            "Only unstaged changes can be discarded"
        );
        ensure!(
            self.snapshot()?.changes.contains(change),
            "File status changed; refresh before discarding"
        );
        ensure!(
            self.patch(change, None)? == displayed.raw,
            "File contents changed; refresh before discarding"
        );
        let patch = displayed.partial_patch(change, StageSelection::Block(block), true)?;
        self.apply_patch(&patch, false)
    }

    fn apply_patch(&self, patch: &str, cached: bool) -> Result<()> {
        let mut cmd = command(&self.root);
        cmd.arg("apply");
        if cached {
            cmd.arg("--cached");
        }
        let mut child = cmd
            .args(["--whitespace=nowarn", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let written = child
            .stdin
            .take()
            .context("Missing Git input")?
            .write_all(patch.as_bytes());
        let result = child.wait_with_output()?;
        ensure!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
        written?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{commit, fixture, run};
    use super::*;
    use std::fs;

    fn change(repo: &Repository, name: &str, area: Area) -> Change {
        repo.snapshot()
            .unwrap()
            .changes
            .into_iter()
            .find(|c| c.path == Path::new(name) && c.area == area)
            .unwrap()
    }

    fn apply(repo: &Repository, change: &Change, selection: StageSelection) {
        let patch = StagingPatch::parse(repo.patch(change, None).unwrap());
        repo.set_staged(AccessMode::ReadWrite, change, selection, patch.as_ref())
            .unwrap();
    }

    #[test]
    fn partial_staging_and_unstaging_preserve_worktree_and_other_index_changes() {
        for newline in ["\n", "\r\n"] {
            let (_temp, repo) = fixture();
            let base = ["a", "b", "c", "d", ""].join(newline);
            let working = ["a", "B", "c", "D", ""].join(newline);
            fs::write(repo.root.join("file.txt"), &base).unwrap();
            fs::write(repo.root.join("other.txt"), "base").unwrap();
            commit(&repo, "base");
            fs::write(repo.root.join("other.txt"), "already staged").unwrap();
            run(&repo.root, &["add", "other.txt"]);
            fs::write(repo.root.join("file.txt"), &working).unwrap();
            apply(
                &repo,
                &change(&repo, "file.txt", Area::Unstaged),
                StageSelection::Block(0),
            );
            assert_eq!(
                repo.index_file(Path::new("file.txt")).unwrap(),
                ["a", "B", "c", "d", ""].join(newline).as_bytes()
            );
            apply(
                &repo,
                &change(&repo, "file.txt", Area::Staged),
                StageSelection::Block(0),
            );
            assert_eq!(
                repo.index_file(Path::new("file.txt")).unwrap(),
                base.as_bytes()
            );
            assert_eq!(
                fs::read(repo.root.join("file.txt")).unwrap(),
                working.as_bytes()
            );
            assert_eq!(
                repo.index_file(Path::new("other.txt")).unwrap(),
                b"already staged"
            );
        }
    }

    #[test]
    fn block_anchors_keep_replacements_together_and_handle_missing_sides() {
        let (_temp, repo) = fixture();
        fs::write(repo.root.join("f"), "keep\nold\nend\ndeleted\nlast\n").unwrap();
        commit(&repo, "base");
        fs::write(repo.root.join("f"), "keep\nnew\nmore\nend\nlast\nadded\n").unwrap();
        let c = change(&repo, "f", Area::Unstaged);
        let patch = StagingPatch::parse(repo.patch(&c, None).unwrap()).unwrap();
        assert_eq!(patch.block_target(true, 2, true), Some(0));
        assert_eq!(patch.block_target(true, 2, false), None);
        assert_eq!(patch.block_target(false, 2, false), Some(0));
        assert_eq!(patch.block_target(false, 3, false), None);
        assert_eq!(patch.block_target(true, 4, false), Some(1));
        assert_eq!(patch.block_target(false, 6, true), Some(2));
        apply(&repo, &c, StageSelection::Block(0));
        assert_eq!(
            repo.index_file(Path::new("f")).unwrap(),
            b"keep\nnew\nmore\nend\ndeleted\nlast\n"
        );
    }

    #[test]
    fn new_files_unborn_head_and_missing_final_newline_round_trip() {
        let (_temp, repo) = fixture();
        let name = if cfg!(windows) {
            "space 中文.txt"
        } else {
            "space\t中文\n.txt"
        };
        fs::write(repo.root.join(name), "one\ntwo").unwrap();
        let untracked = change(&repo, name, Area::Untracked);
        apply(&repo, &untracked, StageSelection::Block(0));
        assert_eq!(repo.index_file(Path::new(name)).unwrap(), b"one\ntwo");
        apply(
            &repo,
            &change(&repo, name, Area::Staged),
            StageSelection::Block(0),
        );
        assert!(repo.index_file(Path::new(name)).is_err());
        apply(
            &repo,
            &change(&repo, name, Area::Untracked),
            StageSelection::File,
        );
        assert_eq!(repo.index_file(Path::new(name)).unwrap(), b"one\ntwo");
        apply(
            &repo,
            &change(&repo, name, Area::Staged),
            StageSelection::File,
        );
        assert!(repo.index_file(Path::new(name)).is_err());
        assert_eq!(fs::read(repo.root.join(name)).unwrap(), b"one\ntwo");
    }

    #[test]
    fn deletion_partial_restore_and_binary_whole_file_round_trip() {
        let (_temp, repo) = fixture();
        fs::write(repo.root.join("f"), "one\ntwo\n").unwrap();
        fs::write(repo.root.join("binary"), [0, 1, 2]).unwrap();
        commit(&repo, "base");
        fs::remove_file(repo.root.join("f")).unwrap();
        apply(
            &repo,
            &change(&repo, "f", Area::Unstaged),
            StageSelection::File,
        );
        let deleted = change(&repo, "f", Area::Staged);
        apply(&repo, &deleted, StageSelection::Block(0));
        assert_eq!(repo.index_file(Path::new("f")).unwrap(), b"one\ntwo\n");
        assert!(!repo.root.join("f").exists());
        apply(
            &repo,
            &change(&repo, "f", Area::Unstaged),
            StageSelection::Block(0),
        );
        assert!(repo.index_file(Path::new("f")).is_err());
        fs::write(repo.root.join("binary"), [0, 3, 4]).unwrap();
        let binary = change(&repo, "binary", Area::Unstaged);
        assert!(StagingPatch::parse(repo.patch(&binary, None).unwrap()).is_none());
        apply(&repo, &binary, StageSelection::File);
        apply(
            &repo,
            &change(&repo, "binary", Area::Staged),
            StageSelection::File,
        );
        assert_eq!(repo.index_file(Path::new("binary")).unwrap(), [0, 1, 2]);
        assert_eq!(fs::read(repo.root.join("binary")).unwrap(), [0, 3, 4]);
    }

    #[test]
    fn read_only_and_stale_partial_requests_cannot_modify_index() {
        let (_temp, repo) = fixture();
        fs::write(repo.root.join("f"), "base\n").unwrap();
        commit(&repo, "base");
        fs::write(repo.root.join("f"), "changed\n").unwrap();
        let change = change(&repo, "f", Area::Unstaged);
        let patch = StagingPatch::parse(repo.patch(&change, None).unwrap()).unwrap();
        assert!(
            repo.set_staged(AccessMode::ReadOnly, &change, StageSelection::File, None)
                .is_err()
        );
        assert!(
            repo.set_staged(
                AccessMode::ReadOnly,
                &change,
                StageSelection::Block(0),
                Some(&patch)
            )
            .is_err()
        );
        fs::write(repo.root.join("f"), "newer\n").unwrap();
        assert!(
            repo.set_staged(
                AccessMode::ReadWrite,
                &change,
                StageSelection::Block(0),
                Some(&patch)
            )
            .is_err()
        );
        assert_eq!(repo.index_file(Path::new("f")).unwrap(), b"base\n");
    }

    #[test]
    fn renamed_files_keep_paths_and_modes_when_only_content_is_unstaged() {
        let (_temp, repo) = fixture();
        let base = "unchanged\n".repeat(20);
        fs::write(repo.root.join("old.txt"), &base).unwrap();
        commit(&repo, "base");
        run(&repo.root, &["mv", "old.txt", "new.txt"]);
        fs::write(repo.root.join("new.txt"), format!("{base}extra\n")).unwrap();
        run(&repo.root, &["add", "new.txt"]);
        let rename = change(&repo, "new.txt", Area::Staged);
        assert_eq!(rename.status, 'R');
        apply(&repo, &rename, StageSelection::Block(0));
        assert_eq!(
            repo.index_file(Path::new("new.txt")).unwrap(),
            base.as_bytes()
        );
        assert!(repo.index_file(Path::new("old.txt")).is_err());
        apply(
            &repo,
            &change(&repo, "new.txt", Area::Staged),
            StageSelection::File,
        );
        assert_eq!(
            repo.index_file(Path::new("old.txt")).unwrap(),
            base.as_bytes()
        );
        assert!(repo.index_file(Path::new("new.txt")).is_err());
        assert!(!repo.root.join("old.txt").exists());
        assert_eq!(
            fs::read_to_string(repo.root.join("new.txt")).unwrap(),
            format!("{base}extra\n")
        );
    }

    #[test]
    fn partial_staging_respects_git_line_ending_conversion_and_literal_paths() {
        let (_temp, repo) = fixture();
        run(&repo.root, &["config", "core.autocrlf", "true"]);
        let name = "[literal] 中文.txt";
        fs::write(repo.root.join(name), "base\r\nend\r\n").unwrap();
        commit(&repo, "base");
        let working = "base\r\nadded\r\nend\r\n";
        fs::write(repo.root.join(name), working).unwrap();
        apply(
            &repo,
            &change(&repo, name, Area::Unstaged),
            StageSelection::Block(0),
        );
        assert_eq!(
            repo.index_file(Path::new(name)).unwrap(),
            b"base\nadded\nend\n"
        );
        apply(
            &repo,
            &change(&repo, name, Area::Staged),
            StageSelection::Block(0),
        );
        assert_eq!(repo.index_file(Path::new(name)).unwrap(), b"base\nend\n");
        assert_eq!(fs::read(repo.root.join(name)).unwrap(), working.as_bytes());
    }

    #[test]
    fn discard_restores_one_block_preserving_index_other_edits_and_line_endings() {
        for (autocrlf, newline) in [(false, "\n"), (false, "\r\n"), (true, "\r\n")] {
            let (_temp, repo) = fixture();
            if autocrlf {
                run(&repo.root, &["config", "core.autocrlf", "true"]);
            }
            let path = "[literal] 中文.txt";
            fs::write(
                repo.root.join(path),
                ["a", "b", "c", "d", "last"].join(newline),
            )
            .unwrap();
            commit(&repo, "base");
            fs::write(
                repo.root.join(path),
                ["staged", "b", "c", "d", "last"].join(newline),
            )
            .unwrap();
            run(&repo.root, &["add", "--", path]);
            fs::write(
                repo.root.join(path),
                ["staged", "B", "extra", "c", "D", "last"].join(newline),
            )
            .unwrap();
            let index = fs::read(repo.root.join(".git/index")).unwrap();
            let c = change(&repo, path, Area::Unstaged);
            let patch = StagingPatch::parse(repo.patch(&c, None).unwrap()).unwrap();
            repo.discard_block(AccessMode::ReadWrite, &c, 0, &patch)
                .unwrap();
            assert_eq!(
                fs::read_to_string(repo.root.join(path)).unwrap(),
                ["staged", "b", "c", "D", "last"].join(newline)
            );
            assert_eq!(fs::read(repo.root.join(".git/index")).unwrap(), index);
        }
    }

    #[test]
    fn discard_added_and_deleted_files_and_reject_stale_or_readonly_requests() {
        let (_temp, repo) = fixture();
        fs::write(repo.root.join("f"), "one\ntwo").unwrap();
        commit(&repo, "base");
        fs::remove_file(repo.root.join("f")).unwrap();
        let c = change(&repo, "f", Area::Unstaged);
        let patch = StagingPatch::parse(repo.patch(&c, None).unwrap()).unwrap();
        assert!(
            repo.discard_block(AccessMode::ReadOnly, &c, 0, &patch)
                .is_err()
        );
        repo.discard_block(AccessMode::ReadWrite, &c, 0, &patch)
            .unwrap();
        assert_eq!(fs::read(repo.root.join("f")).unwrap(), b"one\ntwo");
        fs::write(repo.root.join("new"), "new\nfile").unwrap();
        let c = change(&repo, "new", Area::Untracked);
        let patch = StagingPatch::parse(repo.patch(&c, None).unwrap()).unwrap();
        fs::write(repo.root.join("new"), "edited since confirmation").unwrap();
        assert!(
            repo.discard_block(AccessMode::ReadWrite, &c, 0, &patch)
                .is_err()
        );
        assert_eq!(
            fs::read(repo.root.join("new")).unwrap(),
            b"edited since confirmation"
        );
        let patch = StagingPatch::parse(repo.patch(&c, None).unwrap()).unwrap();
        repo.discard_block(AccessMode::ReadWrite, &c, 0, &patch)
            .unwrap();
        assert!(!repo.root.join("new").exists());
        assert!(repo.snapshot().unwrap().changes.is_empty());
    }

    #[test]
    fn folder_batch_handles_mixed_files_renames_and_deletions_without_touching_neighbors() {
        let (_temp, repo) = fixture();
        fs::create_dir_all(repo.root.join("src/deep")).unwrap();
        fs::create_dir_all(repo.root.join("src-other")).unwrap();
        for name in [
            "src/a.txt",
            "src/deep/b.txt",
            "src/deleted.txt",
            "src/move.txt",
            "src-other/outside.txt",
        ] {
            fs::write(repo.root.join(name), "base\n").unwrap();
        }
        commit(&repo, "base");
        fs::write(repo.root.join("src/a.txt"), "already staged\n").unwrap();
        run(&repo.root, &["add", "src/a.txt"]);
        fs::write(repo.root.join("src/a.txt"), "latest worktree\n").unwrap();
        fs::write(repo.root.join("src/deep/b.txt"), "nested edit\n").unwrap();
        fs::write(repo.root.join("src/new.bin"), b"new\0binary").unwrap();
        fs::remove_file(repo.root.join("src/deleted.txt")).unwrap();
        run(&repo.root, &["mv", "src/move.txt", "src/moved.txt"]);
        fs::write(repo.root.join("src-other/outside.txt"), "outside staged\n").unwrap();
        run(&repo.root, &["add", "src-other/outside.txt"]);
        fs::write(
            repo.root.join("src-other/outside.txt"),
            "outside unstaged\n",
        )
        .unwrap();
        let folder_changes = || {
            repo.snapshot()
                .unwrap()
                .changes
                .into_iter()
                .filter(|c| c.path.starts_with("src"))
                .collect::<Vec<_>>()
        };
        let changes = folder_changes();
        assert!(
            repo.set_files_staged(AccessMode::ReadOnly, &changes, true)
                .is_err()
        );
        repo.set_files_staged(AccessMode::ReadWrite, &changes, true)
            .unwrap();
        assert!(folder_changes().iter().all(|c| c.area == Area::Staged));
        assert_eq!(
            repo.index_file(Path::new("src/a.txt")).unwrap(),
            b"latest worktree\n"
        );
        assert_eq!(
            repo.index_file(Path::new("src/new.bin")).unwrap(),
            b"new\0binary"
        );
        repo.set_files_staged(AccessMode::ReadWrite, &folder_changes(), false)
            .unwrap();
        assert!(folder_changes().iter().all(|c| c.area != Area::Staged));
        assert_eq!(repo.index_file(Path::new("src/a.txt")).unwrap(), b"base\n");
        assert_eq!(
            repo.index_file(Path::new("src/move.txt")).unwrap(),
            b"base\n"
        );
        assert_eq!(
            repo.index_file(Path::new("src/deleted.txt")).unwrap(),
            b"base\n"
        );
        assert_eq!(
            repo.index_file(Path::new("src-other/outside.txt")).unwrap(),
            b"outside staged\n"
        );
        assert_eq!(
            fs::read(repo.root.join("src/a.txt")).unwrap(),
            b"latest worktree\n"
        );
        assert_eq!(
            fs::read(repo.root.join("src/moved.txt")).unwrap(),
            b"base\n"
        );
        assert_eq!(
            fs::read(repo.root.join("src-other/outside.txt")).unwrap(),
            b"outside unstaged\n"
        );
        assert!(!repo.root.join("src/deleted.txt").exists());
    }

    #[test]
    fn folder_batch_supports_unborn_head_and_rejects_stale_groups_before_writing() {
        let (_temp, repo) = fixture();
        fs::create_dir(repo.root.join("folder")).unwrap();
        fs::write(repo.root.join("folder/a.txt"), "a").unwrap();
        fs::write(repo.root.join("folder/b.txt"), "b").unwrap();
        let changes = repo.snapshot().unwrap().changes;
        run(&repo.root, &["add", "folder/a.txt"]);
        assert!(
            repo.set_files_staged(AccessMode::ReadWrite, &changes, true)
                .is_err()
        );
        assert!(repo.index_file(Path::new("folder/b.txt")).is_err());
        repo.set_files_staged(
            AccessMode::ReadWrite,
            &repo.snapshot().unwrap().changes,
            true,
        )
        .unwrap();
        assert_eq!(repo.index_file(Path::new("folder/b.txt")).unwrap(), b"b");
        repo.set_files_staged(
            AccessMode::ReadWrite,
            &repo.snapshot().unwrap().changes,
            false,
        )
        .unwrap();
        assert!(
            repo.snapshot()
                .unwrap()
                .changes
                .iter()
                .all(|c| c.area == Area::Untracked)
        );
        assert_eq!(fs::read(repo.root.join("folder/a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(repo.root.join("folder/b.txt")).unwrap(), b"b");
    }
}
