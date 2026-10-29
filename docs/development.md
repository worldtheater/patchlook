# Development

[Back to Patchlook](../README.md)

## Prerequisites

All platforms need Rust 1.95+ and Git on `PATH`. Packaging and packaging tests also need Python 3.11+.

| Platform | Build dependencies |
| --- | --- |
| macOS | macOS 15+, Xcode, and its command-line tools. `xcode-select -p` must point to a valid Xcode developer directory. |
| Windows | The MSVC Rust toolchain, Visual Studio 2022 with **Desktop development with C++** and the Windows SDK, CMake, and Git for Windows. |
| Linux | A C/C++ toolchain and the native libraries below. Running the app needs X11 or Wayland and a Vulkan-capable graphics driver. |

On Ubuntu 24.04:

```sh
sudo apt-get update
sudo apt-get install -y gcc g++ clang cmake pkg-config \
  libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libssl-dev libzstd-dev libvulkan1 libglib2.0-dev
```

## Run and test

```sh
cargo run --locked --release
cargo run --locked --release -- /path/to/repository
```

Quote repository paths that contain spaces. Use Release builds for UI and scrolling reviews so compiler optimizations are enabled.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Use `python` instead of `python3` on Windows. Git tests use isolated temporary repositories. The test suites cover repository reads, preferences and localization, tree and image behavior, diff layout and scrolling, and packaging failure paths.

### Build cache

Keep builds in the repository's default `target/` directory. The `dev` profile uses `debug = "line-tables-only"` and `split-debuginfo = "packed"`; tests inherit these settings. This retains file and line information for backtraces and keeps incremental compilation enabled. Full variable inspection requires a build with more debug information.

Use the commands above consistently: Clippy and tests share the test dependency features, while application previews use Release. GPUI's `test-support` feature belongs to test builds. Changing dependency sources, features, profiles, compiler versions, or target directories can leave additional artifact variants behind.

After a package rename or a dependency-source migration, reclaim obsolete development artifacts with:

```sh
cargo clean --profile dev --dry-run
cargo clean --profile dev
```

This removes `target/debug` and preserves Release artifacts. The next development check or test rebuilds its dependencies, so use it for occasional cleanup rather than before every build.

## Package a local build

```sh
cargo build --locked --release
python3 scripts/package.py
```

The script packages an existing binary and writes its output to `dist/`:

| Platform | Output |
| --- | --- |
| macOS | `Patchlook.app`, ZIP, and a DMG with an Applications shortcut |
| Windows | ZIP containing `patchlook.exe` and the README |
| Linux | tar.gz containing `patchlook` and the README |

The macOS icon is generated from [the app icon source](../assets/app-icon/patchlook-v2.png). Local macOS packages use ad-hoc signing by default. Linux builds link against system libraries and are not universal distribution packages.

Windows builds embed `assets/app-icon/patchlook.ico` into the executable through `build.rs`. Resource ID `1` is required by GPUI for the window and taskbar icon. The checked-in ICO contains 16–256 px variants, so normal builds do not need image conversion tools. After changing the source logo, regenerate it with ImageMagick:

```sh
magick assets/app-icon/patchlook-v2.png -define icon:auto-resize=256,128,64,48,32,24,16 assets/app-icon/patchlook.ico
```

The Windows integration test loads these resources from the actual executable. Resource compilation errors fail the build instead of producing a package without its icon.

For a local signed and notarized macOS build, install a Developer ID Application certificate and store notarization credentials with `xcrun notarytool store-credentials`, then run:

```sh
python3 scripts/package.py \
  --sign-identity "Developer ID Application: Name (TEAMID)" \
  --notary-profile patchlook-notary
```

## Releases

The [Desktop workflow](../.github/workflows/desktop.yml) checks and packages all three platforms on `main` and pull requests. Ordinary builds do not read release credentials.

To prepare a release:

1. Set the version in `Cargo.toml` and update `Cargo.lock`.
2. Commit and push the changes.
3. Push a matching version tag, such as `v0.1.0`.
4. Review the generated Release draft and its packages before publishing it.

For version tags, the macOS job imports the signing certificate into a temporary keychain, signs with Hardened Runtime and a secure timestamp, submits the DMG to Apple, staples and validates notarization tickets, and checks Gatekeeper acceptance. The ZIP contains the stapled app. A Release draft is created or updated only after all platforms pass; published release assets are not replaced.

Configure these repository secrets under **Settings → Secrets and variables → Actions**:

| Secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE_BASE64` | Base64 of the Developer ID Application `.p12`, including its private key |
| `MACOS_CERTIFICATE_PASSWORD` | The `.p12` export password |
| `MACOS_SIGNING_IDENTITY` | Full certificate name: `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple Account email |
| `APPLE_TEAM_ID` | Paid developer team ID |
| `APPLE_APP_SPECIFIC_PASSWORD` | App-specific password for notarization |

Keep certificates, private keys, and passwords out of the repository. Missing credentials or a failed signing, notarization, or ticket check stops the release job.

### Check a notarization timeout

The workflow waits up to 30 minutes for Apple. A timeout stops CI but does not cancel Apple's processing.

In **Actions → Desktop → Run workflow**, select `main` and enter the existing submission ID in `notary_submission_id`. This queries Apple's status and, when processing has finished, its log. It does not rebuild or upload the app. Results appear in the run's logs and Summary. Leave the input empty to run normal build checks.

Check the existing submission before rerunning a failed release. An `Accepted` response confirms that Apple has accepted that submission; the release still needs successful packaging, ticket verification, and artifact upload.

## GPUI Kit patches

Patchlook uses the [GPUI Kit fork](https://github.com/worldtheater/gpui-kit/tree/codex/diff-tool-patches), on the `codex/diff-tool-patches` branch. [Cargo.toml](../Cargo.toml) pins its Git revision through `[patch.crates-io]`; [Cargo.lock](../Cargo.lock) is checked in. Cargo fetches the source automatically, so normal builds need neither a submodule nor a sibling checkout.

The patch series starts from the GPUI Kit 0.8.0 source commit `1676826d9d49b90f4d7b6808b69924f58796f6f1`. Its current tip is `0917a5564505fceacfbbf966d90c0c19a7af9820`.

| Commit | Purpose |
| --- | --- |
| `697e0e7e` | Apply touchpad release velocity only when an edge is already stretched. |
| `085bc94c` | Consume fractional scrolling within bounds to avoid accidental bounce. |
| `634f5b54` | Keep diff panes equally sized, synchronize horizontal scrolling, and constrain text selection to the visible pane. |
| `dcc55bcc` | Add collapsible diff sections and keep change markers aligned with wrapped lines. |
| `9697b217` | Count contiguous changed blocks independently of folding and layout. |
| `e2a084ae` | Localize diff controls in nine languages and rebuild when locale resources change. |
| `a3080ee7` | Defer diff viewport measurement notifications until after drawing so the first layout finishes without further input. |
| `1b0fa905` | Add an application-owned line-action slot with source coordinates and wrap-layout invalidation. |
| `ff3fe330` | Add compact actions inside existing diff gutters, including empty sides of added/deleted rows, without shifting source text. |
| `815e1fc8` | Refresh related patches while retaining visible source lines, wrapped-row offsets and horizontal scrolling. |
| `2a43f3aa` | Let the application soften changed-line and inline-change backgrounds by source position without removing diff semantics. |
| `9bc37a46` | Isolate gutter actions from line selection and let applications disable mouse selection through line numbers. |
| `0917a556` | Add a smaller switch size and expose leading label placement while retaining standard switch behavior. |

When updating GPUI Kit:

1. Start a new patch branch from the target upstream revision.
2. Port only the patches still needed. Upstream commits `e692bcb7` and `cf5b8639` address the two scrolling fixes; check their behavior before carrying those patches forward.
3. Push the patch branch, then update the pinned revision and lockfile in Patchlook.
4. Run the checks above and a release build. Keep the consumer regressions in [diff_layout.rs](../tests/diff_layout.rs) and [scroll_bounce.rs](../tests/scroll_bounce.rs).

For local work on the fork, temporarily replace the `gpui-kit` Git patch with a path to its `crates/kit` directory. Restore the Git revision and lockfile before committing; do not commit machine-specific paths.
