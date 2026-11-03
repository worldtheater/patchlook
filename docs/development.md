# Development

[Back to Patchlook](../README.md)

## Homepage

The GitHub Pages homepage lives in `website/`: one HTML template, shared CSS,
copy in all nine app languages, and shared menu, theme, platform-switching, and command-copy controls. The Python builder reuses the app
logo and README screenshot and takes download links and SHA-256 checksums from the
same assets in a published GitHub release. The download dialog shows SHA-256, the
hash, and a copy button in one row. Narrow screens truncate the displayed hash;
copying always includes the full value. A missing or malformed asset
digest fails the build, so a new page cannot publish an empty or guessed checksum.
No frontend dependencies are required.

```sh
curl -fsSL https://api.github.com/repos/worldtheater/patchlook/releases/latest -o /tmp/patchlook-release.json
python3 website/build.py --release-json /tmp/patchlook-release.json
python3 -m http.server 4173 --bind 127.0.0.1 --directory dist/site
```

Open `http://127.0.0.1:4173/` (English); use the language menu for the other eight languages. `/zh/` remains the Simplified Chinese address. Check changes with
`python3 -m unittest discover -s website -p 'test_*.py'` and `node --check website/theme.js`.
`website/site.json` owns the repository and canonical site URLs, language routes, and Homebrew tap details. Language names come from the app's locale resources; homepage copy lives in `website/locales/`. Build checks require complete matching translation keys and working links for every language.

For the first deployment, set **Settings → Pages → Source → GitHub Actions**.
`.github/workflows/pages.yml` builds and deploys homepage changes on `main`, manual
runs, and published/edited releases; pull requests only build and validate.
It reads the latest published stable release at build time, so new releases update
the version and platform links without editing page copy. Drafts are never used.

### Search and AI discovery

Each language is a complete static HTML page, including the short platform,
staging, and SSH answers. Titles describe the product; canonical URLs and reciprocal
`hreflang` links identify all nine translations in HTML and `sitemap.xml`. Open Graph
and Twitter metadata reuse the real screenshot. `WebPage` and `SoftwareApplication`
JSON-LD reuse visible copy, requirements, and the published release; do not add
ratings or reviews without actual supporting content.

After deployment, verify the final URLs in Google Search Console and Bing Webmaster
Tools and submit `https://worldtheater.github.io/patchlook/sitemap.xml`. Check indexing,
canonical selection, and structured data against the live pages. The project site
cannot set the host's `https://worldtheater.github.io/robots.txt`: a file under
`/patchlook/` has no robots authority. The builder only emits robots rules when
`site.json` (or `--site-url`) targets a domain root. For this project URL, configure
any crawler restrictions or sitemap declaration in the account-level Pages site,
if one exists; otherwise submit the sitemap directly to the webmaster tools.

AI discovery follows the same crawlable, accurate content. No crawler-only copy,
invented reviews, or special AI instructions are included. `llms.txt` is not required
by Google's AI search guidance. Software markup describes the app but does not
guarantee a rich result: Google's app rich result also requires a genuine rating
or review. Indexing and AI citations require live-site verification and cannot be
established by a local build.

References: [Google AI search guidance](https://developers.google.com/search/docs/appearance/ai-features),
[software application markup](https://developers.google.com/search/docs/appearance/structured-data/software-app),
[localized pages](https://developers.google.com/search/docs/specialty/international/localized-versions),
and [robots.txt placement](https://developers.google.com/crawling/docs/robots-txt/create-robots-txt).

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
cargo test --locked -- --test-threads=1
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Use `python` instead of `python3` on Windows. Git tests use isolated temporary repositories. The test suites cover repository reads, preferences and localization, tree and image behavior, diff layout and scrolling, and packaging failure paths.

### SSH test environment

With Docker running, start an isolated Linux SSH server:

```sh
python3 scripts/ssh_test.py up
```

This creates `patchlook-ssh-test`, bound only to `127.0.0.1:2222`. Its repositories live inside the container; no local repository is mounted. A fixture key and SSH config are generated under ignored `dist/ssh-test/`. The trusted host key is read directly from the container. Your normal SSH config and keys are untouched.

In **Connect via SSH…**, use host `127.0.0.1`, port `2222`, username `review`, and repository `/home/review/repos/demo`. Choose **Password** and use the public fixture password `patchlook-test`. Leave **Advanced options** empty for the normal first-connection flow. Compare the confirmation fingerprint with `ssh-keygen -l -E sha256 -f dist/ssh-test/known_hosts`, then trust it. This records the host in your normal SSH known_hosts. To keep manual testing isolated instead, the optional `dist/ssh-test/config` uses the fixture trust file.

For **SSH key**, choose `dist/ssh-test/id_ed25519`, or `dist/ssh-test/id_ed25519_encrypted` with fixture passphrase `patchlook-test-key`. These are disposable test credentials. The existing config alias also supports CLI key authentication:

```sh
cargo run --locked --release -- --ssh patchlook-test /home/review/repos/demo \
  --ssh-config "$PWD/dist/ssh-test/config"
```

The fixture includes 106 commits, another branch, staged/unstaged changes, renames, deleted and untracked files, binary and image files, and paths containing spaces, quotes, Unicode, and shell metacharacters. `/home/review/repos/unborn` has no commits yet.

The live SSH tests are ignored in ordinary test runs. Run both repository and UI tests explicitly:

```sh
PATCHLOOK_TEST_SSH_CONFIG="$PWD/dist/ssh-test/config" \
  cargo test --locked --test ssh_repository -- --ignored --test-threads=1
cargo build --locked --bin patchlook
PATCHLOOK_TEST_SSH_CONFIG="$PWD/dist/ssh-test/config" \
PATCHLOOK_TEST_SSH_HELPER="$PWD/target/debug/patchlook" \
  cargo test --locked --bin patchlook ssh_ -- --ignored --test-threads=1
```

One test terminates only the fixture user's SSH worker to check disconnect/reconnect behavior; avoid reviewing that fixture in the app while running it. Stop the server with `python3 scripts/ssh_test.py down`; `up` updates the fixture startup script and restarts it with its repository contents preserved. To recreate the fixture from scratch, remove only its container with `docker rm -f patchlook-ssh-test`, then run `up`.

`RepositoryLocation` keeps local paths distinct from SSH host/path/config targets. Local Git calls use direct process arguments. SSH calls quote each argument for the Linux shell and share one framed SSH process, avoiding a new authentication round trip for every read. History uses a separate streaming process so a completed search page can stop `git log` early. A small temporary shell worker is removed when the session ends; it is not an installed service. Remote mutation is rejected in the Git API as well as the UI. Authentication and host-key verification use system OpenSSH. `ssh_auth.rs` serves passwords/passphrases from memory over a capability-protected loopback connection to this same executable in ASKPASS mode, before GPUI starts. No credential is placed in a file, process argument, environment value, or persisted metadata. The helper sends framed prompts through the same capability-protected channel. It supplies credentials only for the expected target prompt. Unknown-host prompts with a SHA-256 fingerprint are routed to the owning window by `ssh_host.rs`; acceptance returns to the same OpenSSH handshake, which persists the key. Declining or destroying the window rejects the request. Other confirmation prompts are rejected. The application forces `StrictHostKeyChecking=ask` for interactive connections, never `accept-new` or `no`; it does not scan and reconnect with a separately accepted key. Existing known_hosts (including configured files) and OpenSSH changed/revoked-key rejection remain authoritative. Tests use isolated trust files, cover both password/key login, and verify refusal, persistence across connections, changed-key blocking, and window cancellation. Connection metadata retains the authentication method so opening a disconnected Recent entry or choosing Reconnect can request a fresh secret. Normal startup restores local repositories automatically but leaves saved SSH repositories disconnected in Repos; an explicit command-line SSH target still opens on launch. A per-window remote-session map owns authenticated Repository handles independently of the current view. Switching repositories reloads their data through the retained session without requesting credentials again. Refresh also reuses it. Disconnect, explicit reconnect, removal, and window teardown close the relevant sessions; switching or a failed load does not close other connected sessions. Interactive multi-factor prompts and server password changes remain unsupported.

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

The patch series starts from the GPUI Kit 0.8.0 source commit `1676826d9d49b90f4d7b6808b69924f58796f6f1`. Its current tip is `907fa984b7a1f316fc767ccf3e25c10b54932518`.

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
| `907fa984` | Apply dialog entrance opacity inside the deferred overlay host so the backdrop and surface fade together. |

When updating GPUI Kit:

1. Start a new patch branch from the target upstream revision.
2. Port only the patches still needed. Upstream commits `e692bcb7` and `cf5b8639` address the two scrolling fixes; check their behavior before carrying those patches forward.
3. Push the patch branch, then update the pinned revision and lockfile in Patchlook.
4. Run the checks above and a release build. Keep the consumer regressions in [diff_layout.rs](../tests/diff_layout.rs) and [scroll_bounce.rs](../tests/scroll_bounce.rs).

For local work on the fork, temporarily replace the `gpui-kit` Git patch with a path to its `crates/kit` directory. Restore the Git revision and lockfile before committing; do not commit machine-specific paths.
