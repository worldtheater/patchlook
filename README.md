<p align="center">
  <img src="https://raw.githubusercontent.com/worldtheater/patchlook/main/assets/app-icon/patchlook-v2.png" alt="Patchlook logo" width="112">
</p>

<h1 align="center">Patchlook</h1>

<p align="center">English · <a href="README.zh-CN.md">简体中文</a></p>

<p align="center">A native Git viewer for macOS, Windows, and Linux. Read-only by default.</p>

<p align="center">
  <a href="https://github.com/worldtheater/patchlook/releases">Releases</a> ·
  <a href="#build-from-source">Build from source</a> ·
  <a href="https://github.com/worldtheater/patchlook/issues">Report an issue</a>
</p>

Inspect local changes, explore commit history, and compare file versions. Patchlook starts in read-only mode; enable read/write mode when you want to stage changes.

![Patchlook reviewing its own commit history in light and dark themes](docs/images/patchlook-light-dark.webp)

## Features

- **Review changes** — inspect staged, unstaged, and untracked files, with added and deleted line counts.
- **Stage selectively** — in read/write mode, stage or unstage whole files and contiguous change blocks. Discard an unwanted block after confirmation.
- **Explore history** — browse branches and commits, or search the full branch history by message or SHA prefix.
- **Compare code** — switch between side-by-side and inline diffs, with syntax highlighting and changes highlighted within each line.
- **Stay focused** — collapse unchanged sections, wrap long lines, or scroll both versions together.
- **Compare images** — view versions side by side, fit them to the window, or zoom and pan to inspect details.
- **Open in another app** — choose a system-recommended application or another program from the file header. Opens the latest working-tree file, or a temporary copy of the version before deletion for deleted files.
- **Make it yours** — light, dark, and system themes, plus nine interface languages.

## Install

Browse [Releases](https://github.com/worldtheater/patchlook/releases) for published builds, or [build from source](#build-from-source).

| Platform | Requirements | Installation |
| --- | --- | --- |
| macOS | macOS 15+, Apple Silicon | Open the DMG and drag **Patchlook** into **Applications**. |
| Windows | Windows 10+, x64 | Extract the ZIP and run `patchlook.exe`. |
| Linux | x86_64, X11 or Wayland, Vulkan-capable graphics | Extract the tar.gz and run `./patchlook`. |

[Git](https://git-scm.com/downloads) must be installed and available on `PATH`. macOS release builds are signed and notarized. Linux packages are built on Ubuntu 24.04 and require compatible system libraries.

## Getting started

1. Choose **Open repository** and select a local Git repository.
2. Use **Changes** to review your working tree and staging area, or **History** to inspect commits.
3. Use **Repos** to switch repositories or browse another branch.

Refresh after making changes in another tool. Branch browsing uses local Git references; fetch updates with your usual Git client.

Text diffs require UTF-8. Supported images are shown as previews; other binary files are listed without a content diff.

### Stage changes

Turn off the **Read-only** switch in the title bar, then open **Changes**. The indicator is green while read-only is on and orange when writing is enabled. Check a file to stage it, or use the outlined square button in the right-hand line-number gutter to stage a complete contiguous change block. Uncheck a staged file or block to unstage it. Each file appears once; partially staged files show their block progress. Staged blocks keep a softer diff background in place. If the index contains a third version of the same block, separate labelled comparisons show HEAD → Index and Index → Working tree.

The tree separates tracked changes and untracked files into collapsible groups. Folder and group buttons include only their own descendants. Click an empty or partially checked folder to stage its remaining changes; click a fully checked folder to unstage its changes.

These controls update the real Git index. Binary files, symlinks, submodule revisions, and metadata-only changes can be handled as whole files; unresolved conflicts must be resolved and staged with another Git tool. If a text file changes outside Patchlook, refresh before staging a block.

In an unstaged diff, the `»` discard button in the left-hand gutter **discards the entire block** after confirmation, restoring its content from the staging area. Other changes and the index stay untouched. Discarding a new file’s block deletes that file. Staging and unstaging alone never change working-tree contents.

Turning **Read-only** back on leaves staged changes intact. Each new application window starts read-only. To review and commit everything currently staged, use your terminal:

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
