<p align="center">
  <img src="https://raw.githubusercontent.com/worldtheater/patchlook/main/assets/app-icon/patchlook-v2.png" alt="Patchlook logo" width="112">
</p>

<h1 align="center">Patchlook</h1>

<p align="center">English · <a href="README.zh-CN.md">简体中文</a></p>

<p align="center">A native Git viewer for macOS, Windows, and Linux. Read-only by default.</p>

<p align="center">
  <a href="https://worldtheater.github.io/patchlook/">Website</a> ·
  <a href="https://github.com/worldtheater/patchlook/releases">Releases</a> ·
  <a href="#build-from-source">Build from source</a> ·
  <a href="https://github.com/worldtheater/patchlook/issues">Report an issue</a>
</p>

Review changes, explore history, and compare file versions in local repositories or on Linux servers over SSH. Enable read/write mode to stage changes in local repositories.

![Patchlook reviewing its own commit history in light and dark themes](docs/images/patchlook-light-dark.webp)

## Features

- **Review changes** — inspect staged, unstaged, and untracked files, with added and deleted line counts.
- **Stage selectively** — in read/write mode, stage or unstage whole files and contiguous change blocks. Discard an unwanted block after confirmation.
- **Explore history** — browse branches and commits, or search the full branch history by message or SHA prefix.
- **Browse over SSH** — inspect changes and history in a Linux server's repository without cloning it locally. Remote repositories stay read-only.
- **Compare code** — switch between side-by-side and inline diffs, with syntax highlighting and changes highlighted within each line.
- **Stay focused** — collapse unchanged sections, wrap long lines, or scroll both versions together.
- **Compare images** — view versions side by side, fit them to the window, or zoom and pan to inspect details.
- **Open in another app** — open the latest local file in a system-recommended app or another program. Deleted files open as a temporary copy of their previous version.
- **Make it yours** — light, dark, and system themes, plus nine interface languages.

## Install

On macOS (Apple Silicon, macOS 15+), install with [Homebrew](https://github.com/worldtheater/homebrew-tap):

```sh
brew install --cask worldtheater/tap/patchlook
```

Git is installed automatically. Upgrade with `brew update && brew upgrade --cask patchlook`. If you previously installed the app manually, quit it and move the existing `Patchlook.app` out of Applications first; your preferences are kept.

Browse [Releases](https://github.com/worldtheater/patchlook/releases) for published builds, or [build from source](#build-from-source).

| Platform | Requirements | Installation |
| --- | --- | --- |
| macOS | macOS 15+, Apple Silicon | Open the DMG and drag **Patchlook** into **Applications**. |
| Windows | Windows 10+, x64 | Extract the ZIP and run `patchlook.exe`. |
| Linux | x86_64, X11 or Wayland, Vulkan-capable graphics | Extract the tar.gz and run `./patchlook`. |

Local repositories require [Git](https://git-scm.com/downloads) on `PATH`. SSH connections require OpenSSH (`ssh` on `PATH`); the Linux server needs Git, a POSIX shell, and standard command-line tools. The server does not need Patchlook installed.

macOS release builds are signed and notarized. Linux packages are built on Ubuntu 24.04 and require compatible system libraries.

## Getting started

1. Choose **Open repository** for a local repository, or [connect via SSH](#connect-to-a-linux-server) for a remote one.
2. Use **Changes** to review your working tree and staging area, or **History** to inspect commits.
3. Use **Repos** to switch repositories or browse another branch.

Refresh after making changes in another tool. Branch browsing uses the opened repository's Git references; fetch updates with your usual Git client.

Text diffs require UTF-8. Supported images are shown as previews; other binary files are listed without a content diff.

### Connect to a Linux server

Available in **v0.1.6+**. Remote repositories are always read-only.

1. Open the repository menu or the **+** menu above Recent and choose **Connect via SSH…**. Enter the host, port, username, and repository's absolute Linux path.
2. Choose **Password** or **SSH key**. Leave the key file empty to use default keys or your SSH agent.
3. Verify the server fingerprint against a trusted source on first connection, then choose **Trust and connect**. The confirmed key is remembered for later connections.

Passwords and key passphrases are not saved. Switching repositories keeps the connection; use the remote entry's menu to disconnect or reconnect. Closing the window ends its connections. SSH config files are optional and belong under **Advanced options**.

### Stage changes

In a local repository, click the lock icon in the title bar to enter **Read/write** mode, then open **Changes**. Hover over the lock to see the current mode. A closed lock and green indicator mean read-only; an open lock and orange indicator mean read/write. Check a file to stage it, or use the outlined square button in the right-hand line-number gutter to stage a complete contiguous change block. Uncheck a staged file or block to unstage it. Each file appears once; partially staged files show their block progress. Staged blocks keep a softer diff background in place. If the index contains a third version of the same block, separate labelled comparisons show HEAD → Index and Index → Working tree.

The tree separates tracked changes and untracked files into collapsible groups. Folder and group buttons include only their own descendants. Click an empty or partially checked folder to stage its remaining changes; click a fully checked folder to unstage its changes.

These controls update the real Git index. Binary files, symlinks, submodule revisions, and metadata-only changes can be handled as whole files; unresolved conflicts must be resolved and staged with another Git tool. If a text file changes outside Patchlook, refresh before staging a block.

In an unstaged diff, the `»` discard button in the left-hand gutter **discards the entire block** after confirmation, restoring its content from the staging area. Other changes and the index stay untouched. Discarding a new file’s block deletes that file. Staging and unstaging alone never change working-tree contents.

The **Undo** and **Redo** buttons beside the filename restore staging and discard operations made in this window, including a new file deleted by discard. Each click is one step, even for a folder, and the history spans files in the current repository. Switching repositories or closing the window clears it. Recovery stops if HEAD or the affected contents have changed externally; unrelated changes are left alone.

Switching back to **Read-only** leaves staged changes intact. Each new application window starts read-only. To review and commit everything currently staged, use your terminal:

```sh
git diff --cached
git commit -m "Your commit message"
```

Use ordinary `git commit` without `-a` or file arguments to commit exactly the staged content. Patchlook does not create commits, switch branches, or push changes.

## Keyboard shortcuts

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Open repository | `⌘ O` | `Ctrl O` |
| Refresh | `⌘ R` / `F5` | `Ctrl R` / `F5` |
| Copy selection | `⌘ C` | `Ctrl C` |
| Select all in the current diff pane | `⌘ A` | `Ctrl A` |
| Navigate the file tree | Arrow keys | Arrow keys |
| Quit | `⌘ Q` | `Ctrl Q` |

## Build from source

Install Rust 1.95+, Git, and the [platform build dependencies](https://github.com/worldtheater/patchlook/blob/main/docs/development.md#prerequisites), then run:

```sh
git clone https://github.com/worldtheater/patchlook.git
cd patchlook
cargo run --locked --release
```

See the [development guide](https://github.com/worldtheater/patchlook/blob/main/docs/development.md) for testing, packaging, releases, and GPUI Kit patches.

Built with [Rust](https://www.rust-lang.org/) and [GPUI Kit](https://github.com/longbridge/gpui-kit).

Licensed under the [MIT License](LICENSE).
