//! Run against `python3 scripts/ssh_test.py up`; never writes the remote fixture.
use patchlook::git::{
    AccessMode, Area, Repository, RepositoryLocation, StageSelection, WriteHistory, WriteScope,
    merge_working_changes,
};
use patchlook::{
    git::{SshAuthentication, SshOptions},
    ssh_auth::SshSecret,
};
use std::{path::Path, process::Command};

#[test]
#[ignore = "requires the isolated SSH fixture with submodules"]
fn remote_submodules_share_authentication_and_remain_read_only() {
    let target = RepositoryLocation::Ssh {
        path: "/home/review/repos/modules".into(),
        options: SshOptions {
            host: "patchlook-test".into(),
            config_file: Some(
                std::env::var_os("PATCHLOOK_TEST_SSH_CONFIG")
                    .unwrap()
                    .into(),
            ),
            ..Default::default()
        },
    };
    let parent = Repository::open_location(target).unwrap();
    let modules = parent.submodules().unwrap();
    assert_eq!(modules.len(), 2);
    let ready = modules.iter().find(|m| m.initialized).unwrap();
    let missing = modules.iter().find(|m| !m.initialized).unwrap();
    assert!(parent.open_related(missing.location.clone()).is_err());
    let child = parent.open_related(ready.location.clone()).unwrap();
    assert!(child.shares_connection(&parent));
    assert_eq!(
        child.history(0, None).unwrap().commits[0].subject,
        "Child history"
    );
    let change = child.working_changes().unwrap().remove(0);
    assert_eq!(change.path, Path::new("new.txt"));
    assert!(
        child
            .set_staged(AccessMode::ReadWrite, &change, StageSelection::File, None)
            .is_err()
    );
    let returned = child.open_related(parent.location.clone()).unwrap();
    assert!(returned.shares_connection(&parent));
    parent.disconnect();
    assert!(child.branch_name().is_err());
    assert!(returned.branch_name().is_err());
}

#[test]
#[ignore = "requires the isolated SSH fixture"]
fn disconnect_invalidates_all_repository_clones_and_reconnect_uses_a_new_session() {
    use patchlook::git::SshConnectionControl;
    let target = RepositoryLocation::Ssh {
        path: "/home/review/repos/demo".into(),
        options: SshOptions {
            host: "patchlook-test".into(),
            config_file: Some(
                std::env::var_os("PATCHLOOK_TEST_SSH_CONFIG")
                    .unwrap()
                    .into(),
            ),
            ..Default::default()
        },
    };
    let control = SshConnectionControl::default();
    let repo = Repository::open_with_control(target.clone(), None, None, control.clone()).unwrap();
    let other_read = repo.clone();
    assert_eq!(repo.branch_name().unwrap(), "main");
    assert!(!other_read.history(0, None).unwrap().commits.is_empty());
    repo.disconnect();
    assert!(control.is_disconnected());
    assert!(other_read.branch_name().is_err());
    assert!(other_read.history(0, None).is_err());
    let reconnected = Repository::open_location(target).unwrap();
    assert_eq!(reconnected.branch_name().unwrap(), "main");
    reconnected.disconnect();
}

#[test]
#[ignore = "requires the isolated SSH fixture"]
fn host_confirmation_remembers_only_approved_keys_and_rejects_changes() {
    use patchlook::ssh_auth::SshHostVerification;
    use std::sync::{Arc, Mutex};
    let state = std::path::PathBuf::from(std::env::var_os("PATCHLOOK_TEST_SSH_CONFIG").unwrap())
        .parent()
        .unwrap()
        .to_owned();
    let fingerprint = Command::new("ssh-keygen")
        .args(["-l", "-E", "sha256", "-f"])
        .arg(state.join("known_hosts"))
        .output()
        .unwrap();
    assert!(fingerprint.status.success());
    let fingerprint = String::from_utf8(fingerprint.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    for authentication in [SshAuthentication::Password, SshAuthentication::Key] {
        let temporary = tempfile::tempdir().unwrap();
        let known = temporary.path().join("known_hosts");
        let config = temporary.path().join("config");
        // Only isolate trust storage. Host, port, username and authentication
        // still come from the same separate fields as an ordinary connection.
        std::fs::write(&config, format!("Host *\n UserKnownHostsFile \"{}\"\n GlobalKnownHostsFile \"{}\"\n HostKeyAlgorithms ssh-ed25519\n HashKnownHosts yes\n StrictHostKeyChecking no\n", known.display(), temporary.path().join("global_hosts").display())).unwrap();
        let target = RepositoryLocation::Ssh {
            path: "/home/review/repos/demo".into(),
            options: SshOptions {
                host: "127.0.0.1".into(),
                port: Some(2222),
                username: Some("review".into()),
                authentication,
                identity_file: (authentication == SshAuthentication::Key)
                    .then(|| state.join("id_ed25519")),
                config_file: Some(config),
            },
        };
        let secret = || {
            (authentication == SshAuthentication::Password).then(|| {
                SshSecret::new(
                    "patchlook-test".into(),
                    env!("CARGO_BIN_EXE_patchlook").into(),
                )
                .unwrap()
            })
        };
        let requests = Arc::new(Mutex::new(Vec::new()));
        let declined = requests.clone();
        let verifier =
            SshHostVerification::new(env!("CARGO_BIN_EXE_patchlook").into(), move |key| {
                declined.lock().unwrap().push(key);
                false
            });
        assert!(
            Repository::open_with_verification(target.clone(), secret(), Some(verifier)).is_err()
        );
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert!(!known.exists() || std::fs::read(&known).unwrap().is_empty());
        let accepted = requests.clone();
        let verifier =
            SshHostVerification::new(env!("CARGO_BIN_EXE_patchlook").into(), move |key| {
                accepted.lock().unwrap().push(key);
                true
            });
        let repo =
            Repository::open_with_verification(target.clone(), secret(), Some(verifier)).unwrap();
        assert_eq!(repo.branch_name().unwrap(), "main");
        assert!(!repo.history(0, None).unwrap().commits.is_empty());
        assert_eq!(requests.lock().unwrap().len(), 2);
        let confirmed = requests.lock().unwrap()[1].clone();
        assert!(confirmed.host.contains("2222"));
        assert_eq!(confirmed.algorithm, "ED25519");
        assert_eq!(confirmed.fingerprint, fingerprint);
        let saved = std::fs::read(&known).unwrap();
        assert!(!saved.is_empty());
        drop(repo);
        let verifier = SshHostVerification::new(env!("CARGO_BIN_EXE_patchlook").into(), |_| {
            panic!("remembered hosts must not prompt again")
        });
        assert_eq!(
            Repository::open_with_verification(target.clone(), secret(), Some(verifier))
                .unwrap()
                .branch_name()
                .unwrap(),
            "main"
        );
        assert_eq!(std::fs::read(&known).unwrap(), saved);
        // Seed an unrelated key for the same host to simulate server-key change,
        // without touching the fixture server or the user's known_hosts.
        let replacement = temporary.path().join("different-key");
        assert!(
            Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(&replacement)
                .status()
                .unwrap()
                .success()
        );
        let changed = format!(
            "[127.0.0.1]:2222 {}",
            std::fs::read_to_string(replacement.with_extension("pub")).unwrap()
        );
        std::fs::write(&known, &changed).unwrap();
        let verifier = SshHostVerification::new(env!("CARGO_BIN_EXE_patchlook").into(), |_| {
            panic!("changed keys must not offer a trust override")
        });
        let error =
            Repository::open_with_verification(target, secret(), Some(verifier)).unwrap_err();
        assert!(format!("{error:#}").contains("REMOTE HOST IDENTIFICATION HAS CHANGED"));
        assert_eq!(std::fs::read_to_string(&known).unwrap(), changed);
    }
}

#[test]
#[ignore = "requires the isolated SSH fixture"]
fn password_and_explicit_keys_authenticate_without_persisting_secrets() {
    let config = std::path::PathBuf::from(std::env::var_os("PATCHLOOK_TEST_SSH_CONFIG").unwrap());
    let state = config.parent().unwrap();
    let target = |authentication, identity_file| RepositoryLocation::Ssh {
        path: "/home/review/repos/demo".into(),
        options: SshOptions {
            host: "127.0.0.1".into(),
            port: Some(2222),
            username: Some("review".into()),
            authentication,
            identity_file,
            config_file: Some(config.clone()),
        },
    };
    let secret =
        |value: &str| SshSecret::new(value.into(), env!("CARGO_BIN_EXE_patchlook").into()).unwrap();
    for (authentication, identity, value) in [
        (SshAuthentication::Password, None, Some("patchlook-test")),
        (SshAuthentication::Key, Some(state.join("id_ed25519")), None),
        (
            SshAuthentication::KeyWithPassphrase,
            Some(state.join("id_ed25519_encrypted")),
            Some("patchlook-test-key"),
        ),
    ] {
        let target = target(authentication, identity);
        let repo = Repository::open_authenticated(target.clone(), value.map(secret)).unwrap();
        assert_eq!(repo.branch_name().unwrap(), "main");
        // History creates a separate SSH process; it must use the same in-memory credential.
        assert!(!repo.history(0, None).unwrap().commits.is_empty());
        let mut preferences = patchlook::preferences::Preferences::default();
        preferences.remember_repository(repo.location.clone());
        let saved = serde_json::to_string(&preferences).unwrap();
        if let Some(value) = value {
            assert!(!saved.contains(value));
            assert!(!format!("{repo:?}").contains(value));
            assert!(Repository::open_location(target).is_err());
        }
    }
    let target = target(SshAuthentication::Password, None);
    let error = Repository::open_authenticated(target.clone(), Some(secret("wrong-test-password")))
        .unwrap_err();
    assert!(!format!("{error:#}").contains("wrong-test-password"));
    assert!(format!("{error:#}").contains("Permission denied"));
    // Missing trust must fail even when a correct password is supplied.
    let temporary = tempfile::tempdir().unwrap();
    let untrusted = temporary.path().join("config");
    std::fs::write(
        &untrusted,
        format!(
            "Host *\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile {}\n",
            temporary.path().join("empty-hosts").display(),
            temporary.path().join("empty-global").display()
        ),
    )
    .unwrap();
    let RepositoryLocation::Ssh { mut options, path } = target else {
        unreachable!()
    };
    options.config_file = Some(untrusted);
    let error = Repository::open_authenticated(
        RepositoryLocation::Ssh { options, path },
        Some(secret("patchlook-test")),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("Host key verification failed"));
}

fn location(path: &str) -> RepositoryLocation {
    RepositoryLocation::Ssh {
        path: path.into(),
        options: patchlook::git::SshOptions {
            host: "patchlook-test".into(),
            config_file: Some(
                std::env::var_os("PATCHLOOK_TEST_SSH_CONFIG")
                    .expect("Set PATCHLOOK_TEST_SSH_CONFIG to the fixture config")
                    .into(),
            ),
            ..Default::default()
        },
    }
}

#[test]
#[ignore = "requires the isolated SSH fixture"]
fn ssh_reads_worktree_history_images_and_rejects_all_writes() {
    let repo = Repository::open_location(location("/home/review/repos/demo")).unwrap();
    assert!(repo.is_remote());
    assert_eq!(repo.branch_name().unwrap(), "main");
    assert!(
        repo.branches()
            .unwrap()
            .iter()
            .any(|b| b.name == "feature/demo")
    );
    let before = repo.working_changes().unwrap();
    assert!(before.iter().any(|c| c.area == Area::Untracked));
    assert!(before.iter().any(|c| c.status == 'D'));
    let changes = merge_working_changes(&before);
    let partial = changes
        .iter()
        .find(|c| c.path == Path::new("partial.txt"))
        .unwrap();
    assert_eq!(partial.area, Area::Partial);
    let review = repo.review_patch(partial).unwrap();
    assert_eq!(review.progress(), (1, 2));
    assert!(!review.separate_versions());
    for change in &changes {
        assert!(repo.patch(change, None).is_ok(), "{:?}", change.path);
    }
    let odd = Path::new("$(touch SHOULD_NOT_EXIST).txt");
    assert_eq!(
        repo.worktree_file(odd).unwrap().unwrap(),
        b"literal shell characters\n"
    );
    assert!(
        repo.worktree_file(Path::new("SHOULD_NOT_EXIST"))
            .unwrap()
            .is_none()
    );
    let image = changes
        .iter()
        .find(|c| c.path == Path::new("preview.png"))
        .unwrap();
    assert!(
        repo.file_versions(image, None)
            .unwrap()
            .modified
            .unwrap()
            .starts_with(b"\x89PNG")
    );
    let stats = repo.file_stats(&before, None).unwrap();
    assert_eq!(stats.iter().filter(|s| s.is_none()).count(), 2);
    let page = repo.history(0, None).unwrap();
    assert_eq!(page.commits.len(), 100);
    assert!(page.has_more);
    let next = repo.history(100, page.revision.as_deref()).unwrap();
    assert_eq!(next.commits.len(), 6);
    let found = repo.history_filtered(0, None, "remote-review").unwrap();
    assert_eq!(found.commits.len(), 100);
    assert!(found.has_more);
    let hash = &page.commits[0].id[..10];
    assert_eq!(
        repo.history_filtered(0, None, hash).unwrap().commits[0].id,
        page.commits[0].id
    );
    let commit = next.commits.last().unwrap();
    let files = repo.commit_changes(commit).unwrap();
    assert!(!files.is_empty());
    assert!(
        repo.patch(&files[0], Some(commit))
            .unwrap()
            .contains("diff --git")
    );
    assert!(
        repo.set_staged(AccessMode::ReadWrite, partial, StageSelection::File, None)
            .is_err()
    );
    assert!(
        repo.set_files_staged(AccessMode::ReadWrite, &changes, true)
            .is_err()
    );
    assert!(
        repo.set_review_block(AccessMode::ReadWrite, &review, 1)
            .is_err()
    );
    assert!(
        repo.discard_review_block(AccessMode::ReadWrite, &review, 1)
            .is_err()
    );
    let mut history = WriteHistory::default();
    assert!(
        history
            .record(
                &repo,
                AccessMode::ReadWrite,
                WriteScope::index(&changes),
                || panic!("remote write must not run")
            )
            .is_err()
    );
    assert_eq!(repo.working_changes().unwrap(), before);
    let unborn = Repository::open_location(location("/home/review/repos/unborn")).unwrap();
    assert!(unborn.history(0, None).unwrap().commits.is_empty());
    assert_eq!(unborn.working_changes().unwrap().len(), 1);
}

#[test]
#[ignore = "requires the isolated SSH fixture"]
fn disconnected_session_reports_error_and_can_reconnect() {
    let target = location("/home/review/repos/demo");
    let repo = Repository::open_location(target.clone()).unwrap();
    let RepositoryLocation::Ssh { options, .. } = target else {
        unreachable!()
    };
    let result = Command::new("ssh")
        .arg("-F")
        .arg(options.config_file.unwrap())
        .arg(options.host)
        .arg("pkill -u review -f '^sh -c # Runs inside one SSH session' || true")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(repo.branch_name().is_err());
    assert_eq!(repo.branch_name().unwrap(), "main");
}
