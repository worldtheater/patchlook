use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    pub location: RepositoryLocation,
    pub initialized: bool,
}

impl Repository {
    /// Discover direct children from the index, including uninitialized gitlinks.
    /// Nested modules are discovered when their own repository is opened.
    pub fn submodules(&self) -> Result<Vec<Submodule>> {
        let entries = output(self.command().args(["ls-files", "--stage", "-z"]), false)?;
        let mut paths = BTreeSet::new();
        for entry in entries.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            let tab = entry
                .iter()
                .position(|b| *b == b'\t')
                .context("Invalid index entry")?;
            if entry.starts_with(b"160000 ") {
                let relative = path(&entry[tab + 1..])?;
                ensure!(
                    relative
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_))),
                    "Invalid submodule path"
                );
                paths.insert(relative);
            }
        }
        paths
            .into_iter()
            .map(|relative| {
                let location = self.location.join(&relative)?;
                let initialized = self.has_repository_directory(location.path())?;
                Ok(Submodule {
                    location,
                    initialized,
                })
            })
            .collect()
    }

    fn has_repository_directory(&self, directory: &Path) -> Result<bool> {
        // Inspect the child's own .git; ordinary discovery in an empty child
        // directory would walk upwards and accidentally reopen its parent.
        // Resolve relative to the child: Git's gitfile reader expects '/' in
        // paths, while Path::join inserts '\\' on Windows.
        let result = command(directory)
            .remote(self.remote.clone())
            .args(["rev-parse", "--resolve-git-dir", ".git"])
            .output()?;
        Ok(result.status.success())
    }

    pub fn ensure_submodule_directory(&self, location: &RepositoryLocation) -> Result<()> {
        ensure!(
            self.has_repository_directory(location.path())?,
            "Submodule is not initialized"
        );
        ensure!(
            self.location == *location,
            "Submodule path resolves to another repository"
        );
        Ok(())
    }

    /// Navigate within a repository family using the existing SSH session.
    /// The caller supplies a known repository root, never a file or empty directory.
    pub fn open_related(&self, location: RepositoryLocation) -> Result<Self> {
        match (&self.location, &location) {
            (RepositoryLocation::Local(_), RepositoryLocation::Local(_)) => {}
            (
                RepositoryLocation::Ssh { options: a, .. },
                RepositoryLocation::Ssh { options: b, .. },
            ) if a == b => {}
            _ => bail!("Cannot reuse a connection for another SSH server"),
        }
        let repo = Self::open_with_session(location.clone(), self.remote.clone())?;
        ensure!(
            repo.location == location,
            "Submodule is not initialized or resolves to another repository"
        );
        Ok(repo)
    }

    pub fn shares_connection(&self, other: &Self) -> bool {
        match (&self.remote, &other.remote) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl RepositoryLocation {
    pub fn join(&self, relative: &Path) -> Result<Self> {
        Ok(match self {
            Self::Local(root) => Self::Local(root.join(relative)),
            Self::Ssh { options, path } => Self::Ssh {
                options: options.clone(),
                // Remote paths are POSIX even on Windows clients.
                path: format!(
                    "{}/{}",
                    path.trim_end_matches('/'),
                    relative
                        .to_str()
                        .context("Remote repository path must be UTF-8")?
                ),
            },
        })
    }
}
