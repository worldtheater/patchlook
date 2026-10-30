//! Session-only undo records for the exact index entries or file a write touches.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    sync::Arc,
};

pub enum WriteScope {
    Index(Vec<PathBuf>),
    Worktree(PathBuf),
}

impl WriteScope {
    pub fn index(changes: &[Change]) -> Self {
        Self::Index(
            changes
                .iter()
                .flat_map(|change| {
                    std::iter::once(change.path.clone()).chain(change.previous_path.clone())
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        )
    }

    fn paths(&self) -> &[PathBuf] {
        match self {
            Self::Index(paths) => paths,
            Self::Worktree(path) => std::slice::from_ref(path),
        }
    }
}

#[derive(PartialEq)]
struct FileVersion {
    bytes: Vec<u8>,
    permissions: fs::Permissions,
}

#[derive(PartialEq)]
struct Snapshot {
    index: Vec<u8>,
    intent_to_add: BTreeSet<PathBuf>,
    file: Option<FileVersion>,
}

impl Snapshot {
    fn read(repo: &Repository, scope: &WriteScope) -> Result<Self> {
        ensure!(!scope.paths().is_empty(), "No files selected");
        let index = output(
            command(&repo.root)
                .args(["ls-files", "--stage", "-z", "--"])
                .args(scope.paths()),
            false,
        )?;
        // Only empty-blob entries can be intent-to-add. For those candidates,
        // compare Git's two index views to distinguish the flag from a real
        // empty file. Ordinary nonempty entries need no extra Git reads.
        let candidates = index
            .split(|byte| *byte == 0)
            .filter(|entry| {
                entry.split(|byte| *byte == b' ').nth(1) == Some(repo.empty_blob.as_slice())
            })
            .map(|entry| {
                let tab = entry
                    .iter()
                    .position(|byte| *byte == b'\t')
                    .context("Invalid index entry")?;
                path(&entry[tab + 1..])
            })
            .collect::<Result<Vec<_>>>()?;
        let added = |visibility| -> Result<BTreeSet<PathBuf>> {
            output(
                command(&repo.root)
                    .args([
                        "diff",
                        "--cached",
                        "--name-only",
                        "-z",
                        "--no-renames",
                        "--diff-filter=A",
                        visibility,
                        "--",
                    ])
                    .args(&candidates),
                false,
            )?
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(path)
            .collect()
        };
        let intent_to_add = if candidates.is_empty() {
            BTreeSet::new()
        } else {
            added("--ita-visible-in-index")?
                .difference(&added("--ita-invisible-in-index")?)
                .cloned()
                .collect()
        };
        let file = if let WriteScope::Worktree(path) = scope {
            let path = repo.root.join(path);
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    ensure!(metadata.is_file(), "The path is no longer a regular file");
                    Some(FileVersion {
                        bytes: fs::read(path)?,
                        permissions: metadata.permissions(),
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            }
        } else {
            None
        };
        Ok(Self {
            index,
            intent_to_add,
            file,
        })
    }
}

struct WriteRecord {
    root: PathBuf,
    head: Vec<u8>,
    scope: WriteScope,
    before: Snapshot,
    after: Snapshot,
}

impl WriteRecord {
    fn restore(&self, repo: &Repository, mode: AccessMode, undo: bool) -> Result<()> {
        ensure!(
            mode == AccessMode::ReadWrite,
            "Repository is in read-only mode"
        );
        ensure!(repo.root == self.root, "The repository changed");
        ensure!(
            head(repo)? == self.head,
            "HEAD changed; cannot restore this operation"
        );
        let (expected, target) = if undo {
            (&self.after, &self.before)
        } else {
            (&self.before, &self.after)
        };
        ensure!(
            Snapshot::read(repo, &self.scope)? == *expected,
            "The affected contents changed outside this operation; nothing was restored"
        );
        match &self.scope {
            WriteScope::Index(paths) => {
                // Mode zero removes the entry; any correctly formatted object ID works.
                // Reusing an existing ID supports both SHA-1 and SHA-256 repositories.
                let source = if self.before.index.is_empty() {
                    &self.after.index
                } else {
                    &self.before.index
                };
                let oid = source
                    .split(|b| *b == b' ')
                    .nth(1)
                    .context("Missing index object")?;
                let mut input = Vec::new();
                for path in paths {
                    input.extend_from_slice(b"0 ");
                    input.extend_from_slice(oid);
                    input.push(b'\t');
                    input.extend_from_slice(path.as_os_str().as_encoded_bytes());
                    input.push(0);
                }
                let mut intent_groups: BTreeMap<&[u8], Vec<PathBuf>> = BTreeMap::new();
                for entry in target
                    .index
                    .split(|byte| *byte == 0)
                    .filter(|entry| !entry.is_empty())
                {
                    let tab = entry
                        .iter()
                        .position(|byte| *byte == b'\t')
                        .context("Invalid index entry")?;
                    let name = path(&entry[tab + 1..])?;
                    if target.intent_to_add.contains(&name) {
                        let mode = entry.split(|byte| *byte == b' ').next().unwrap();
                        let metadata = fs::symlink_metadata(repo.root.join(&name))
                            .context("Intent-to-add file no longer exists; nothing was restored")?;
                        ensure!(
                            match mode {
                                b"100644" | b"100755" => metadata.is_file(),
                                b"120000" => metadata.file_type().is_symlink(),
                                _ => false,
                            },
                            "Intent-to-add file type changed; nothing was restored"
                        );
                        intent_groups.entry(mode).or_default().push(name);
                    } else {
                        input.extend_from_slice(entry);
                        input.push(0);
                    }
                }
                let mut child = command(&repo.root)
                    .args(["update-index", "-z", "--index-info"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;
                let written = child
                    .stdin
                    .take()
                    .context("Missing Git input")?
                    .write_all(&input);
                let result = child.wait_with_output()?;
                ensure!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr).trim()
                );
                written?;
                for (mode, paths) in intent_groups {
                    let mut cmd = command(&repo.root);
                    cmd.args(["add", "--intent-to-add", "--force"]);
                    match mode {
                        b"100755" => {
                            cmd.arg("--chmod=+x");
                        }
                        b"100644" => {
                            cmd.arg("--chmod=-x");
                        }
                        _ => {}
                    }
                    output(cmd.arg("--").args(paths), false)?;
                }
            }
            WriteScope::Worktree(path) => {
                let path = repo.root.join(path);
                match &target.file {
                    Some(file) => {
                        let parent = path.parent().context("Missing parent directory")?;
                        fs::create_dir_all(parent)?;
                        let mut saved = tempfile::NamedTempFile::new_in(parent)?;
                        saved.write_all(&file.bytes)?;
                        saved.as_file().set_permissions(file.permissions.clone())?;
                        saved.persist(&path).map_err(|error| error.error)?;
                    }
                    None => fs::remove_file(path)?,
                }
            }
        }
        Ok(())
    }
}

fn head(repo: &Repository) -> Result<Vec<u8>> {
    output(
        command(&repo.root).args(["rev-parse", "--verify", "--quiet", "HEAD"]),
        true,
    )
}

/// Owned by one application window. Switching repositories drops both stacks.
#[derive(Clone, Default)]
pub struct WriteHistory {
    undo: Vec<Arc<WriteRecord>>,
    redo: Vec<Arc<WriteRecord>>,
}

impl WriteHistory {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn record(
        &mut self,
        repo: &Repository,
        mode: AccessMode,
        scope: WriteScope,
        write: impl FnOnce() -> Result<()>,
    ) -> Result<Vec<PathBuf>> {
        ensure!(
            mode == AccessMode::ReadWrite,
            "Repository is in read-only mode"
        );
        let head = head(repo)?;
        let before = Snapshot::read(repo, &scope)?;
        write()?;
        let after = Snapshot::read(repo, &scope)
            .context("Changes were saved, but the undo record could not be completed")?;
        let paths = scope.paths().to_vec();
        if before != after {
            self.undo.push(Arc::new(WriteRecord {
                root: repo.root.clone(),
                head,
                scope,
                before,
                after,
            }));
            self.redo.clear();
        }
        Ok(paths)
    }

    pub fn undo(&mut self, repo: &Repository, mode: AccessMode) -> Result<Vec<PathBuf>> {
        let record = self.undo.last().context("Nothing to undo")?;
        record.restore(repo, mode, true)?;
        let paths = record.scope.paths().to_vec();
        self.redo.push(self.undo.pop().unwrap());
        Ok(paths)
    }

    pub fn redo(&mut self, repo: &Repository, mode: AccessMode) -> Result<Vec<PathBuf>> {
        let record = self.redo.last().context("Nothing to redo")?;
        record.restore(repo, mode, false)?;
        let paths = record.scope.paths().to_vec();
        self.undo.push(self.redo.pop().unwrap());
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{commit, fixture, run};

    fn index(repo: &Repository, name: &str) -> Vec<u8> {
        output(
            command(&repo.root).args(["show", &format!(":{name}")]),
            false,
        )
        .unwrap()
    }

    fn change(repo: &Repository, name: &str) -> Change {
        merge_working_changes(&repo.snapshot().unwrap().changes)
            .into_iter()
            .find(|change| change.path == Path::new(name))
            .unwrap()
    }

    fn stage(history: &mut WriteHistory, repo: &Repository, files: &[&str], staged: bool) {
        let changes = files
            .iter()
            .map(|name| change(repo, name))
            .collect::<Vec<_>>();
        history
            .record(
                repo,
                AccessMode::ReadWrite,
                WriteScope::index(&changes),
                || repo.set_files_staged(AccessMode::ReadWrite, &changes, staged),
            )
            .unwrap();
    }

    fn discard(history: &mut WriteHistory, repo: &Repository, name: &str, block: usize) {
        let change = change(repo, name);
        let review = repo.review_patch(&change).unwrap();
        history
            .record(
                repo,
                AccessMode::ReadWrite,
                WriteScope::Worktree(change.path.clone()),
                || repo.discard_review_block(AccessMode::ReadWrite, &review, block),
            )
            .unwrap();
    }

    #[test]
    fn staged_files_and_blocks_round_trip_without_touching_other_paths_or_worktree() {
        let (_directory, repo) = fixture();
        for name in ["a.txt", "b.txt", "other.txt"] {
            fs::write(repo.root.join(name), "a\nb\nc\nd\n").unwrap();
        }
        commit(&repo, "base");
        for name in ["a.txt", "b.txt"] {
            fs::write(repo.root.join(name), "a\nB\nc\nD\n").unwrap();
        }
        let mut history = WriteHistory::default();
        let change = change(&repo, "a.txt");
        let review = repo.review_patch(&change).unwrap();
        history
            .record(
                &repo,
                AccessMode::ReadWrite,
                WriteScope::index(&[change]),
                || repo.set_review_block(AccessMode::ReadWrite, &review, 0),
            )
            .unwrap();
        assert_eq!(index(&repo, "a.txt"), b"a\nB\nc\nd\n");
        stage(&mut history, &repo, &["a.txt", "b.txt"], true);
        fs::write(repo.root.join("other.txt"), "external staged change\n").unwrap();
        run(&repo.root, &["add", "other.txt"]);
        fs::write(repo.root.join("a.txt"), "later worktree edit\n").unwrap();
        history.undo(&repo, AccessMode::ReadWrite).unwrap();
        assert_eq!(index(&repo, "a.txt"), b"a\nB\nc\nd\n");
        assert_eq!(index(&repo, "b.txt"), b"a\nb\nc\nd\n");
        history.undo(&repo, AccessMode::ReadWrite).unwrap();
        assert_eq!(index(&repo, "a.txt"), b"a\nb\nc\nd\n");
        assert!(!history.can_undo());
        history.redo(&repo, AccessMode::ReadWrite).unwrap();
        history.redo(&repo, AccessMode::ReadWrite).unwrap();
        assert_eq!(index(&repo, "a.txt"), b"a\nB\nc\nD\n");
        assert_eq!(index(&repo, "other.txt"), b"external staged change\n");
        assert_eq!(
            fs::read(repo.root.join("a.txt")).unwrap(),
            b"later worktree edit\n"
        );
        assert!(!history.can_redo());
    }

    #[test]
    fn discard_undo_restores_exact_bytes_permissions_and_deleted_new_files() {
        for newline in ["\n", "\r\n"] {
            let (_directory, repo) = fixture();
            run(&repo.root, &["config", "core.autocrlf", "false"]);
            let base = ["a", "b", "c", "d"].join(newline);
            let edited = ["a", "B", "c", "D"].join(newline);
            fs::write(repo.root.join("f.txt"), &base).unwrap();
            commit(&repo, "base");
            fs::write(repo.root.join("f.txt"), &edited).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(repo.root.join("f.txt"), fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
            let permissions = fs::metadata(repo.root.join("f.txt")).unwrap().permissions();
            let mut history = WriteHistory::default();
            discard(&mut history, &repo, "f.txt", 0);
            let discarded = fs::read(repo.root.join("f.txt")).unwrap();
            assert_ne!(discarded, edited.as_bytes());
            history.undo(&repo, AccessMode::ReadWrite).unwrap();
            assert_eq!(
                fs::read(repo.root.join("f.txt")).unwrap(),
                edited.as_bytes()
            );
            assert_eq!(
                fs::metadata(repo.root.join("f.txt")).unwrap().permissions(),
                permissions
            );
            history.redo(&repo, AccessMode::ReadWrite).unwrap();
            assert_eq!(fs::read(repo.root.join("f.txt")).unwrap(), discarded);
            assert_eq!(index(&repo, "f.txt"), base.as_bytes());

            fs::create_dir(repo.root.join("new")).unwrap();
            let new_path = "new/[literal].txt";
            fs::write(repo.root.join(new_path), &edited).unwrap();
            discard(&mut history, &repo, new_path, 0);
            assert!(!repo.root.join(new_path).exists());
            history.undo(&repo, AccessMode::ReadWrite).unwrap();
            assert_eq!(
                fs::read(repo.root.join(new_path)).unwrap(),
                edited.as_bytes()
            );
            history.redo(&repo, AccessMode::ReadWrite).unwrap();
            assert!(!repo.root.join(new_path).exists());

            fs::remove_file(repo.root.join("f.txt")).unwrap();
            discard(&mut history, &repo, "f.txt", 0);
            assert_eq!(fs::read(repo.root.join("f.txt")).unwrap(), base.as_bytes());
            history.undo(&repo, AccessMode::ReadWrite).unwrap();
            assert!(!repo.root.join("f.txt").exists());
            history.redo(&repo, AccessMode::ReadWrite).unwrap();
            assert_eq!(fs::read(repo.root.join("f.txt")).unwrap(), base.as_bytes());
        }
    }

    #[test]
    fn undo_rejects_external_changes_and_readonly_and_new_writes_clear_redo() {
        let (_directory, repo) = fixture();
        fs::write(repo.root.join("f.txt"), "old\n").unwrap();
        commit(&repo, "base");
        fs::write(repo.root.join("f.txt"), "new\n").unwrap();
        let mut history = WriteHistory::default();
        discard(&mut history, &repo, "f.txt", 0);
        assert!(history.undo(&repo, AccessMode::ReadOnly).is_err());
        assert!(history.can_undo());
        fs::write(repo.root.join("f.txt"), "external\n").unwrap();
        assert!(history.undo(&repo, AccessMode::ReadWrite).is_err());
        assert_eq!(fs::read(repo.root.join("f.txt")).unwrap(), b"external\n");
        fs::write(repo.root.join("f.txt"), "old\n").unwrap();
        history.undo(&repo, AccessMode::ReadWrite).unwrap();
        assert!(history.can_redo());
        stage(&mut history, &repo, &["f.txt"], true);
        assert!(!history.can_redo());
        fs::write(repo.root.join("f.txt"), "external\n").unwrap();
        run(&repo.root, &["add", "f.txt"]);
        assert!(history.undo(&repo, AccessMode::ReadWrite).is_err());
        assert_eq!(index(&repo, "f.txt"), b"external\n");
        fs::write(repo.root.join("f.txt"), "new\n").unwrap();
        run(&repo.root, &["add", "f.txt"]);
        commit(&repo, "external commit");
        assert!(history.undo(&repo, AccessMode::ReadWrite).is_err());
        assert_eq!(index(&repo, "f.txt"), b"new\n");
    }

    #[test]
    fn newborn_repository_and_staged_rename_round_trip() {
        let (_directory, repo) = fixture();
        let mut history = WriteHistory::default();
        fs::write(repo.root.join("old.txt"), "new\n").unwrap();
        stage(&mut history, &repo, &["old.txt"], true);
        history.undo(&repo, AccessMode::ReadWrite).unwrap();
        assert!(
            repo.snapshot()
                .unwrap()
                .changes
                .iter()
                .all(|c| c.area == Area::Untracked)
        );
        history.redo(&repo, AccessMode::ReadWrite).unwrap();
        commit(&repo, "base");
        run(&repo.root, &["mv", "old.txt", "renamed.txt"]);
        history = WriteHistory::default();
        stage(&mut history, &repo, &["renamed.txt"], false);
        assert_eq!(index(&repo, "old.txt"), b"new\n");
        history.undo(&repo, AccessMode::ReadWrite).unwrap();
        assert_eq!(index(&repo, "renamed.txt"), b"new\n");
        assert!(repo.root.join("renamed.txt").exists());
        history.redo(&repo, AccessMode::ReadWrite).unwrap();
        assert_eq!(index(&repo, "old.txt"), b"new\n");
    }
}
