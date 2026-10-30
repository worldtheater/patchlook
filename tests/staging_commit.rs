//! Verify the committed tree, not just the index shown by the application.
use patchlook::git::{
    AccessMode, Area, Change, Repository, WriteHistory, WriteScope, merge_working_changes,
};
use std::{collections::BTreeSet, fs, path::Path, process::Command};

struct Fixture {
    _directory: tempfile::TempDir,
    repo: Repository,
}

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

impl Fixture {
    fn new() -> Self {
        Self::with_object_format("sha1")
    }

    fn with_object_format(object_format: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(
            root,
            &[
                "init",
                "-b",
                "main",
                &format!("--object-format={object_format}"),
            ],
        );
        for (key, value) in [
            ("user.name", "Staging Commit Test"),
            ("user.email", "staging@example.test"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.hooksPath", ".git/no-hooks"),
        ] {
            git(root, &["config", key, value]);
        }
        let repo = Repository::open(root).unwrap();
        Self {
            _directory: directory,
            repo,
        }
    }

    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.repo.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn read(&self, path: &str) -> Vec<u8> {
        fs::read(self.repo.root.join(path)).unwrap()
    }

    fn seed(&self, files: &[(&str, &[u8])]) {
        for (path, bytes) in files {
            self.write(path, bytes);
        }
        git(&self.repo.root, &["add", "--all"]);
        git(&self.repo.root, &["commit", "-m", "base"]);
    }

    fn change(&self, path: &str) -> Change {
        merge_working_changes(&self.repo.snapshot().unwrap().changes)
            .into_iter()
            .find(|change| change.path == Path::new(path))
            .unwrap()
    }

    fn block(&self, path: &str, block: usize) {
        let review = self.repo.review_patch(&self.change(path)).unwrap();
        self.repo
            .set_review_block(AccessMode::ReadWrite, &review, block)
            .unwrap();
    }

    fn commit_exact(&self, files: &[(&str, &[u8])]) {
        // Deliberately no `add`, `-a`, or file arguments at the commit boundary.
        git(&self.repo.root, &["commit", "-m", "selected by Patchlook"]);
        let paths = git(
            &self.repo.root,
            &["ls-tree", "-r", "--name-only", "-z", "HEAD"],
        );
        let actual = paths
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8(path.to_vec()).unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual,
            files.iter().map(|(path, _)| path.to_string()).collect()
        );
        for (path, bytes) in files {
            assert_eq!(
                git(&self.repo.root, &["show", &format!("HEAD:{path}")]),
                *bytes,
                "committed {path}"
            );
        }
        assert!(git(&self.repo.root, &["diff", "--cached", "--name-only"]).is_empty());
    }
}

#[test]
fn commit_contains_only_selected_blocks_and_preexisting_staged_content() {
    let f = Fixture::new();
    let base = b"one\nkeep\ntwo\nkeep\nthree\n";
    let working = b"ONE\nkeep\nTWO\nkeep\nTHREE\n";
    f.seed(&[
        ("f.txt", base),
        ("existing.txt", b"old\n"),
        ("untouched.txt", b"old\n"),
    ]);
    f.write("f.txt", working);
    f.write("existing.txt", "already staged\n");
    git(&f.repo.root, &["add", "existing.txt"]);
    f.write("untouched.txt", "keep in working tree\n");
    f.write("untracked.txt", "do not include\n");
    f.block("f.txt", 0);
    f.block("f.txt", 2);
    assert_eq!(f.read("f.txt"), working);
    f.commit_exact(&[
        ("f.txt", b"ONE\nkeep\ntwo\nkeep\nTHREE\n"),
        ("existing.txt", b"already staged\n"),
        ("untouched.txt", b"old\n"),
    ]);
    assert_eq!(f.read("f.txt"), working);
    assert_eq!(f.read("untouched.txt"), b"keep in working tree\n");
    assert_eq!(f.change("f.txt").area, Area::Unstaged);
    assert_eq!(f.change("untracked.txt").area, Area::Untracked);
}

#[test]
fn unstage_excludes_the_block_from_the_real_commit() {
    let f = Fixture::new();
    f.seed(&[("f.txt", b"one\nkeep\ntwo\n")]);
    f.write("f.txt", "ONE\nkeep\nTWO\n");
    f.repo
        .set_files_staged(AccessMode::ReadWrite, &[f.change("f.txt")], true)
        .unwrap();
    f.block("f.txt", 0);
    f.commit_exact(&[("f.txt", b"one\nkeep\nTWO\n")]);
    assert_eq!(f.read("f.txt"), b"ONE\nkeep\nTWO\n");
}

#[test]
fn third_version_keeps_existing_index_content_in_the_commit() {
    let f = Fixture::new();
    f.seed(&[("f.txt", b"a=1\nkeep\nkeep\nb=1\n")]);
    f.write("f.txt", "a=2\nkeep\nkeep\nb=1\n");
    git(&f.repo.root, &["add", "f.txt"]);
    f.write("f.txt", "a=3\nkeep\nkeep\nb=3\n");
    let review = f.repo.review_patch(&f.change("f.txt")).unwrap();
    assert!(review.separate_versions());
    let part = (0..review.patches().count())
        .find(|&i| review.part_area(i) == Some(Area::Unstaged))
        .unwrap();
    let block = review.block_target(part, false, 4, false).unwrap();
    f.repo
        .set_review_block(AccessMode::ReadWrite, &review, block)
        .unwrap();
    f.commit_exact(&[("f.txt", b"a=2\nkeep\nkeep\nb=3\n")]);
    assert_eq!(f.read("f.txt"), b"a=3\nkeep\nkeep\nb=3\n");
}

#[test]
fn folder_commit_handles_add_delete_rename_binary_and_unselected_neighbors() {
    let f = Fixture::new();
    f.seed(&[
        ("src/change.txt", b"old"),
        ("src/leave.txt", b"old"),
        ("src/delete.txt", b"delete"),
        ("src/old.txt", b"move"),
        ("src/binary", &[0, 1]),
        ("src-other/neighbor.txt", b"old"),
    ]);
    f.write("src/change.txt", "new");
    f.write("src/leave.txt", "unselected");
    fs::remove_file(f.repo.root.join("src/delete.txt")).unwrap();
    fs::rename(
        f.repo.root.join("src/old.txt"),
        f.repo.root.join("src/new.txt"),
    )
    .unwrap();
    f.write("src/binary", [0, 2, 255]);
    f.write("src/中文 [literal].txt", "new file");
    f.write("src-other/neighbor.txt", "unselected neighbor");
    f.write("src-other/new.txt", "untracked neighbor");
    let changes = merge_working_changes(&f.repo.snapshot().unwrap().changes)
        .into_iter()
        .filter(|change| change.path.starts_with("src"))
        .collect::<Vec<_>>();
    f.repo
        .set_files_staged(AccessMode::ReadWrite, &changes, true)
        .unwrap();
    f.repo
        .set_files_staged(AccessMode::ReadWrite, &[f.change("src/leave.txt")], false)
        .unwrap();
    f.commit_exact(&[
        ("src/change.txt", b"new"),
        ("src/leave.txt", b"old"),
        ("src/new.txt", b"move"),
        ("src/binary", &[0, 2, 255]),
        ("src/中文 [literal].txt", b"new file"),
        ("src-other/neighbor.txt", b"old"),
    ]);
    assert_eq!(f.read("src/leave.txt"), b"unselected");
    assert_eq!(f.read("src-other/neighbor.txt"), b"unselected neighbor");
}

#[test]
fn discard_and_stage_undo_redo_commit_the_final_selection() {
    let f = Fixture::new();
    f.seed(&[("f.txt", b"one\nkeep\ntwo\n")]);
    f.write("f.txt", "ONE\nkeep\nTWO\n");
    let mut history = WriteHistory::default();
    let review = f.repo.review_patch(&f.change("f.txt")).unwrap();
    history
        .record(
            &f.repo,
            AccessMode::ReadWrite,
            WriteScope::Worktree("f.txt".into()),
            || {
                f.repo
                    .discard_review_block(AccessMode::ReadWrite, &review, 0)
            },
        )
        .unwrap();
    let change = f.change("f.txt");
    let review = f.repo.review_patch(&change).unwrap();
    history
        .record(
            &f.repo,
            AccessMode::ReadWrite,
            WriteScope::index(&[change]),
            || f.repo.set_review_block(AccessMode::ReadWrite, &review, 0),
        )
        .unwrap();
    history.undo(&f.repo, AccessMode::ReadWrite).unwrap();
    history.undo(&f.repo, AccessMode::ReadWrite).unwrap();
    assert_eq!(f.read("f.txt"), b"ONE\nkeep\nTWO\n");
    history.redo(&f.repo, AccessMode::ReadWrite).unwrap();
    history.redo(&f.repo, AccessMode::ReadWrite).unwrap();
    f.commit_exact(&[("f.txt", b"one\nkeep\nTWO\n")]);
    assert_eq!(f.read("f.txt"), b"one\nkeep\nTWO\n");
    assert!(history.undo(&f.repo, AccessMode::ReadWrite).is_err());
}

#[test]
fn restored_untracked_file_can_be_committed_without_other_untracked_files() {
    let f = Fixture::new();
    f.write("new.txt", "recover without final newline");
    f.write("leave.txt", "do not commit");
    let mut history = WriteHistory::default();
    let review = f.repo.review_patch(&f.change("new.txt")).unwrap();
    history
        .record(
            &f.repo,
            AccessMode::ReadWrite,
            WriteScope::Worktree("new.txt".into()),
            || {
                f.repo
                    .discard_review_block(AccessMode::ReadWrite, &review, 0)
            },
        )
        .unwrap();
    history.undo(&f.repo, AccessMode::ReadWrite).unwrap();
    f.block("new.txt", 0);
    f.commit_exact(&[("new.txt", b"recover without final newline")]);
    assert_eq!(f.read("leave.txt"), b"do not commit");
}

#[test]
fn partial_commits_respect_crlf_conversion_and_missing_final_newline() {
    for autocrlf in ["false", "true"] {
        let f = Fixture::new();
        git(&f.repo.root, &["config", "core.autocrlf", autocrlf]);
        f.seed(&[("[literal].txt", b"one\r\nkeep\r\ntwo")]);
        f.write("[literal].txt", "ONE\r\nkeep\r\nTWO");
        f.block("[literal].txt", 0);
        let expected: &[u8] = if autocrlf == "true" {
            b"ONE\nkeep\ntwo"
        } else {
            b"ONE\r\nkeep\r\ntwo"
        };
        f.commit_exact(&[("[literal].txt", expected)]);
        assert_eq!(f.read("[literal].txt"), b"ONE\r\nkeep\r\nTWO");
    }
}

#[test]
fn undo_staging_an_intent_to_add_file_does_not_commit_an_empty_file() {
    for object_format in ["sha1", "sha256"] {
        for initial_commit in [false, true] {
            for contents in ["not selected after undo", ""] {
                let f = Fixture::with_object_format(object_format);
                if initial_commit {
                    f.seed(&[("keep.txt", b"base")]);
                }
                // A real empty file must remain staged when the same snapshot also
                // contains an intent-to-add entry with an identical blob ID.
                f.write("empty.txt", "");
                git(&f.repo.root, &["add", "empty.txt"]);
                f.write("new.txt", contents);
                git(&f.repo.root, &["add", "-N", "--chmod=+x", "new.txt"]);
                let before = git(&f.repo.root, &["status", "--porcelain=v1", "-z"]);
                let before_index = git(&f.repo.root, &["ls-files", "--stage", "-z"]);
                let changes = [f.change("new.txt"), f.change("empty.txt")];
                let mut history = WriteHistory::default();
                history
                    .record(
                        &f.repo,
                        AccessMode::ReadWrite,
                        WriteScope::index(&changes),
                        || {
                            f.repo
                                .set_files_staged(AccessMode::ReadWrite, &changes, true)
                        },
                    )
                    .unwrap();
                history.undo(&f.repo, AccessMode::ReadWrite).unwrap();
                assert_eq!(
                    git(&f.repo.root, &["status", "--porcelain=v1", "-z"]),
                    before
                );
                assert_eq!(
                    git(&f.repo.root, &["ls-files", "--stage", "-z"]),
                    before_index
                );
                history.redo(&f.repo, AccessMode::ReadWrite).unwrap();
                let staged = git(&f.repo.root, &["ls-files", "--stage", "-z"]);
                fs::remove_file(f.repo.root.join("new.txt")).unwrap();
                assert!(history.undo(&f.repo, AccessMode::ReadWrite).is_err());
                assert_eq!(git(&f.repo.root, &["ls-files", "--stage", "-z"]), staged);
                f.write("new.txt", contents);
                history.undo(&f.repo, AccessMode::ReadWrite).unwrap();
                f.write("keep.txt", "selected");
                f.repo
                    .set_files_staged(AccessMode::ReadWrite, &[f.change("keep.txt")], true)
                    .unwrap();
                f.commit_exact(&[("keep.txt", b"selected"), ("empty.txt", b"")]);
                assert_eq!(f.read("new.txt"), contents.as_bytes());
            }
        }
    }
}
