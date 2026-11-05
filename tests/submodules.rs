use patchlook::{
    git::{AccessMode, Repository, RepositoryLocation, StageSelection, WriteHistory, WriteScope},
    preferences::Preferences,
};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Submodule Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.test")
        .env("GIT_COMMITTER_NAME", "Submodule Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.test")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

fn init(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["config", "core.autocrlf", "false"]);
    fs::write(root.join("file.txt"), "base\n").unwrap();
    git(root, &["add", "file.txt"]);
    git(root, &["commit", "-m", "base"]);
}

fn fixture() -> (tempfile::TempDir, Repository) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let parent = temp.path().join("parent");
    init(&source);
    init(&parent);
    git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            source.to_str().unwrap(),
            "deps/子模块 [one]",
        ],
    );
    git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            source.to_str().unwrap(),
            "missing",
        ],
    );
    git(&parent, &["commit", "-am", "add modules"]);
    git(&parent, &["submodule", "deinit", "-f", "--", "missing"]);
    (temp, Repository::open(&parent).unwrap())
}

#[test]
fn discovery_and_navigation_keep_uninitialized_and_nested_modules_distinct() {
    let (_temp, parent) = fixture();
    let modules = parent.submodules().unwrap();
    assert_eq!(modules.len(), 2);
    let ready = modules.iter().find(|m| m.initialized).unwrap();
    let missing = modules.iter().find(|m| !m.initialized).unwrap();
    assert!(parent.open_related(missing.location.clone()).is_err());
    // Ordinary Git discovery would resolve the empty directory to the parent.
    let discovered = Repository::open(missing.location.path()).unwrap();
    assert_eq!(discovered.root, parent.root);
    assert!(
        discovered
            .ensure_submodule_directory(&missing.location)
            .is_err()
    );
    let child = parent.open_related(ready.location.clone()).unwrap();
    assert_eq!(child.location, ready.location);
    assert_ne!(child.root, parent.root);
    let nested = child.root.join("nested");
    init(&nested); // Also cover old-style embedded .git directories.
    let nested_head = git(&nested, &["rev-parse", "HEAD"]);
    git(
        &child.root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &nested_head,
            "nested",
        ],
    );
    let nested_modules = child.submodules().unwrap();
    assert_eq!(nested_modules.len(), 1);
    assert!(nested_modules[0].initialized);
    assert_eq!(parent.submodules().unwrap().len(), 2);
    assert_eq!(
        child
            .open_related(nested_modules[0].location.clone())
            .unwrap()
            .root,
        nested
    );
    assert_eq!(
        child.open_related(parent.location.clone()).unwrap().root,
        parent.root
    );
}

#[test]
fn child_staging_commit_and_parent_pointer_undo_are_independent() {
    let (_temp, parent) = fixture();
    let child = parent
        .open_related(
            parent
                .submodules()
                .unwrap()
                .into_iter()
                .find(|m| m.initialized)
                .unwrap()
                .location,
        )
        .unwrap();
    let relative = child.root.strip_prefix(&parent.root).unwrap();
    let before = git(&parent.root, &["ls-files", "--stage"]);
    fs::write(child.root.join("file.txt"), "selected child change\n").unwrap();
    let change = child.working_changes().unwrap().remove(0);
    let mut child_history = WriteHistory::default();
    child_history
        .record(
            &child,
            AccessMode::ReadWrite,
            WriteScope::Index(vec![change.path.clone()]),
            || child.set_staged(AccessMode::ReadWrite, &change, StageSelection::File, None),
        )
        .unwrap();
    assert_eq!(git(&parent.root, &["ls-files", "--stage"]), before);
    assert_eq!(
        git(&child.root, &["show", ":file.txt"]),
        "selected child change"
    );
    child_history.undo(&child, AccessMode::ReadWrite).unwrap();
    assert_eq!(git(&child.root, &["show", ":file.txt"]), "base");
    assert_eq!(
        fs::read_to_string(child.root.join("file.txt")).unwrap(),
        "selected child change\n"
    );
    child_history.redo(&child, AccessMode::ReadWrite).unwrap();
    assert_eq!(git(&parent.root, &["ls-files", "--stage"]), before);
    git(
        &child.root,
        &["-c", "commit.gpgsign=false", "commit", "-m", "child change"],
    );
    assert_eq!(
        git(&child.root, &["show", "HEAD:file.txt"]),
        "selected child change"
    );
    let change = parent
        .working_changes()
        .unwrap()
        .into_iter()
        .find(|c| c.path == relative)
        .unwrap();
    let mut parent_history = WriteHistory::default();
    parent_history
        .record(
            &parent,
            AccessMode::ReadWrite,
            WriteScope::Index(vec![relative.to_owned()]),
            || parent.set_staged(AccessMode::ReadWrite, &change, StageSelection::File, None),
        )
        .unwrap();
    let staged = git(&parent.root, &["ls-files", "--stage"]);
    assert_ne!(staged, before);
    parent_history.undo(&parent, AccessMode::ReadWrite).unwrap();
    assert_eq!(git(&parent.root, &["ls-files", "--stage"]), before);
    assert_eq!(
        git(&child.root, &["show", "HEAD:file.txt"]),
        "selected child change"
    );
    parent_history.redo(&parent, AccessMode::ReadWrite).unwrap();
    assert_eq!(git(&parent.root, &["ls-files", "--stage"]), staged);
}

#[test]
fn saved_children_keep_independent_branches_and_refresh_without_duplicates() {
    let (_temp, parent) = fixture();
    let mut preferences = Preferences::default();
    preferences.remember_repository(parent.location.clone());
    let modules = parent.submodules().unwrap();
    preferences.remember_submodules(&parent.location, &modules);
    let child = &modules.iter().find(|m| m.initialized).unwrap().location;
    preferences.remember_repository(child.clone());
    preferences.set_viewed_branch(child, Some("refs/heads/child".into()));
    preferences.set_viewed_branch(&parent.location, Some("refs/heads/parent".into()));
    preferences.remember_submodules(&parent.location, &modules);
    let mut restored: Preferences =
        serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
    assert_eq!(restored.recent_repositories.len(), 3);
    assert_eq!(restored.repository_family(child), parent.location);
    assert_eq!(restored.viewed_branch(child), Some("refs/heads/child"));
    assert_eq!(
        restored.viewed_branch(&parent.location),
        Some("refs/heads/parent")
    );
    let nested: RepositoryLocation = child.join(Path::new("nested")).unwrap();
    restored.remember_submodules(
        child,
        &[patchlook::git::Submodule {
            location: nested.clone(),
            initialized: true,
        }],
    );
    assert_eq!(restored.repository_family(&nested), parent.location);
    restored.remember_submodules(&parent.location, &[]);
    assert_eq!(restored.recent_repositories.len(), 1);
    assert_eq!(restored.recent_repositories[0].path, parent.location);
    assert!(restored.repository.is_none());
}
