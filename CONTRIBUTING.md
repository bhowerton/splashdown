# Contributing to Splashdown

Thanks for helping. This file covers setting up, the checks a change has to pass, and where things live. For how the code fits together, read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first.

## Setting up

**All platforms**
- Rust stable (pinned by `rust-toolchain.toml`; `rustup` picks it up).
- `cmake`, for local speech-to-text (transcribe.cpp). Without it, build the CLI with `--no-default-features --features full` (everything but local speech-to-text and the natural voice). `--no-default-features` alone builds only the remote control, as the app ships it.

**macOS**
- Xcode 26 or later (the macOS 26 SDK), for the Swift shim that talks to Apple's on-device models. The app itself runs on macOS 12 and later.
- To package releases: `cargo install tauri-cli --locked` and `rustup target add x86_64-apple-darwin`.

**Windows**
- The MSVC build tools, LLVM (libclang, for the llama.cpp and transcribe.cpp bindings) and WebView2 (preinstalled on Windows 11). With winget:
  ```powershell
  winget install Rustlang.Rustup Kitware.CMake LLVM.LLVM Git.Git
  winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```
  On an ARM64 PC, also add the "MSVC ARM64 build tools" component in the Visual Studio Installer.
- The Windows platform layer (`crates/splashdown-platform/src/win/`) is complete, and signed Windows releases ship from it. A few things are macOS only for now: `splashdown://` links, Shortcuts actions, Apple's on-device models, remembering people by voice in meetings, and the computer-use spike.

## Building and running

```sh
cargo build --workspace
cargo test --workspace
scripts/run-mac-app.sh        # macOS: build, bundle, sign and launch the menu bar app
scripts\run-win-app.ps1       # Windows: build and launch the tray app (checks the build tools first)
```

The built-in local text model runs in a separate helper, `splashdown-llm` (see the architecture doc); `cargo build --workspace` builds it next to the app, which is where the app looks for it. The same goes for `splashdown-mcp`, which Settings → Privacy → Coding agents runs to set agents up. Download the text model with `cargo run --release -p splashdown-cli -- models download qwen3-4b` (2.5 GB).

Three binaries ship inside the app, as Tauri's external binaries (`tauri.release.conf.json`; `scripts/run-mac-app.sh` and `scripts/package-win.ps1` build and copy them): `splashdown-llm`, `splashdown-mcp` and the slim `splashdown-cli` (built with `--no-default-features`). On Windows there's a fourth, `splashdown-llm-basic`, the helper for processors without AVX2.

`scripts/run-mac-app.sh` builds a real `.app` bundle so macOS grants Accessibility and Microphone to Splashdown rather than to your terminal. It signs with the first "Apple Development" certificate in your keychain, so permissions survive rebuilds. Without one it signs ad hoc, and macOS asks again after every build. Set `SPLASHDOWN_SIGN_IDENTITY` to choose a certificate. Logs go to `target/macos/splashdown.log`. On Windows, `scripts\run-win-app.ps1` needs no signing (Windows asks for no permissions), passes `.env.local` the same way, and logs to `%LOCALAPPDATA%\Splashdown\logs\splashdown.log`. If PowerShell refuses to run scripts, use `powershell -ExecutionPolicy Bypass -File scripts\run-win-app.ps1`.

**API keys for development.** Copy `.env.local.example` to `.env.local` (gitignored) and fill in what you have. The `splashdown-cli` CLI reads it from the current directory, and `run-mac-app.sh` passes it to the app. Never commit keys; the app itself stores keys in the OS keychain.

### The Notes editor

The Notes window's editor (CodeMirror) is bundled into `app/ui/vendor/editor.js`, which is committed, so building the app needs no JavaScript tools. After changing `app/editor/src/` or its dependencies, rebuild it with [pnpm](https://pnpm.io):

```sh
cd app/editor && pnpm install && pnpm build
```

That also rewrites `app/ui/vendor/editor.LICENSES.txt`, the licenses of what's in the bundle.

## Checks every change must pass

CI runs these (clippy and the tests on both macOS and Windows); please run them before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p splashdown-cli --no-default-features --all-targets --locked -- -D warnings   # the CLI as the app ships it
cargo test --workspace --locked
cargo deny check licenses     # once: cargo install cargo-deny --locked
```

The last one checks that every dependency's license allows MIT/Apache-2.0 distribution (the rules are in `deny.toml`). A new dependency under a license it doesn't list fails CI.

From a Mac you can also check that the shared crates still build for Windows (transcribe.cpp can't be cross-compiled, hence the flags):

```sh
cargo clippy --target x86_64-pc-windows-msvc --no-default-features \
  -p splashdown-core -p splashdown-audio -p splashdown-providers -p splashdown-platform -p splashdown-apple-shim -p splashdown-config \
  -p splashdown-control -p splashdown-mcp -- -D warnings
```

**Tests that need hardware or keys** are marked `#[ignore]` with the reason, and don't run by default:
- `splashdown-apple-shim`: Apple Intelligence turned on, and Apple's speech model downloaded. Run with `cargo test -p splashdown-apple-shim -- --ignored`.
- The evals (`splashdown-cli eval cleanup`, `commands`, `actions`, `compose`, `style`, `window-language` and `stt`; `eval make-audio` makes the audio for `stt`) are commands, not tests. All but `window-language`, which needs no model, call real providers and need keys or local models; see [eval/README.md](eval/README.md).

## Packaging a release

```sh
scripts/package-mac.sh
```

This builds a universal (Apple Silicon + Intel) app with the hardened runtime and makes a DMG in `target/universal-apple-darwin/release/bundle/dmg/`. It signs with a "Developer ID Application" certificate if you have one. Test builds stop there; `NOTARIZE=1 scripts/package-mac.sh` also notarizes the app and the DMG with Apple (a few minutes), using an App Store Connect API key described in `.notarize.env.example`. The app's Info.plist additions and entitlements are in `app/src-tauri/`. It also refreshes `THIRD-PARTY-LICENSES.html` (with `cargo install cargo-about`), which ships inside the app.

**The updater signing key.** Installed copies only accept updates signed with the project's updater key. Its public half is `plugins.updater.pubkey` in `app/src-tauri/tauri.conf.json`; the private half lives outside the repository, at `~/.tauri/splashdown-updater.key` on the maintainer's Mac (set `UPDATER_KEY` to use another path). Never commit it. **Back it up** somewhere safe (a password manager): if it's lost, no existing install can verify a new release, and every user would have to download the next version by hand. Without the key, `package-mac.sh` still makes test builds, just without update files.

### Publishing a release

Releases are built by GitHub Actions (`.github/workflows/release.yml`), so they don't depend on anyone's Mac:

1. Bump `version` in `Cargo.toml` (workspace) and `app/src-tauri/tauri.conf.json`, and commit with a message starting "Release " ("Release 0.2.0"). Push it and a matching tag together, without waiting for anything in between: `git tag -a v0.2.0 -m "Splashdown 0.2.0" && git push origin main v0.2.0`. The workflow stops if the tag and the two versions disagree.
2. Approve the run when GitHub asks (the "release" environment). On each platform it runs clippy and the tests, then builds the notarized Mac DMG or the signed Windows installer from the same release build, so nothing is compiled twice. It merges their `latest.json` files into one and creates a **draft** release with everything attached (about 15 minutes).
3. Read the draft: optionally edit the notes, and replace the placeholder `notes` in `latest.json` (Settings → About shows it). Then **Publish**. It must be a full release, not a pre-release: installed copies read `releases/latest/download/latest.json` (on launch and once a day, unless turned off), and the README's download links point at `releases/latest/download/Splashdown-mac.dmg` and `Splashdown-windows-x64-setup.exe`. Nothing installs until the user clicks.

Running the workflow by hand (Actions → Release → Run workflow) makes the same builds as downloadable artifacts, without a release. For an unsigned Windows test installer, run CI by hand (Actions → CI → Run workflow) with "Also build an unsigned Windows test installer" ticked.

**What runs when.** A push to `main` only checks formatting and licenses (seconds). Pull requests, and CI run by hand, get the full build: clippy and the tests on macOS and Windows. Releases run the same checks inside their own builds. `release-cache.yml` keeps the release builds' caches warm when dependencies change, and weekly; it skips a release's version bump, since the cache from before it still holds every dependency. Locally, `NOTARIZE=1 scripts/package-mac.sh` and `scripts\package-win.ps1` make the same files in `…/bundle/release/`.

**One-time setup.** The repository has an environment named `release` (Settings → Environments) that needs a maintainer's approval for every run and only runs for tags matching `v*` and for `main` (manual runs). Its secrets are never repository secrets, so pull requests and other workflows can't read them. To fill them in, export your Developer ID certificate (Keychain Access → My Certificates → right-click "Developer ID Application: …" → Export, as a `.p12` with a password; that certificate only), then run:

```sh
scripts/set-release-secrets.sh ~/Desktop/developer-id.p12
```

It reads `.notarize.env` and the updater key, checks the `.p12`, uploads everything below with `gh` without printing it, and sets `CI_RELEASES`. Delete the `.p12` afterward.

| Secret | What it is |
|---|---|
| `APPLE_CERTIFICATE` | The Developer ID Application certificate and its private key, exported from Keychain Access as a `.p12`, base64-encoded: `base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | The password chosen when exporting the `.p12` |
| `APPLE_API_KEY` | The App Store Connect API key ID (the same one as in `.notarize.env`) |
| `APPLE_API_ISSUER` | Its issuer ID |
| `APPLE_API_KEY_P8` | The contents of its `AuthKey_….p8` file |
| `TAURI_SIGNING_PRIVATE_KEY` | The contents of `~/.tauri/splashdown-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password (leave empty if it has none) |

The repository **variable** (not a secret) `CI_RELEASES` = `true` is what turns tag releases on. Without it, pushing a tag runs nothing, and releases are made by hand: `NOTARIZE=1 scripts/package-mac.sh`, then `gh release create` with the files from `…/bundle/release/` (that's how v0.1.0 was made). Before the first tag release, test with a manual run (Actions → Release → Run workflow), which builds and signs everything as downloadable artifacts without releasing.

Whoever can run this workflow with these secrets can ship an update to every installed copy, which is why the environment needs a reviewer. Keep your own backup of the updater key: GitHub secrets can't be read back.

**Homebrew.** The cask lives in [bhowerton/homebrew-tap](https://github.com/bhowerton/homebrew-tap) (`Casks/splashdown.rb`). After each release, update its `version` and `sha256` (`shasum -a 256 Splashdown-mac.dmg`); installed copies update themselves, so this only matters for new installs.

**Windows code signing** uses Azure Artifact Signing (formerly Trusted Signing), with a certificate issued to Point Dynamics, LLC: account `pointdynamics` (East US), certificate profile `PointDynamicsPublic`, in Point Dynamics' Azure directory. GitHub Actions signs as the app registration `github-actions-code-signing`, which holds only the "Artifact Signing Certificate Profile Signer" role on that account; its client ID, tenant ID and client secret are the `AZURE_CLIENT_ID`, `AZURE_TENANT_ID` and `AZURE_CLIENT_SECRET` secrets in the `release` environment. `scripts/package-win.ps1` passes Tauri a `signCommand` (`artifact-signing-cli`) when those are set, so the app, the helper and the installer are signed before the update signature is made, and fails if the installer's signature isn't valid and from Point Dynamics. **The client secret expires on 2028-10-30:** before then, create a new one under the app registration's Certificates & secrets and replace `AZURE_CLIENT_SECRET`. The identity validation is valid until 2029-02-01 and is renewed in the portal.

## Changing the AI prompts

Each prompt lives next to the code that uses it, in `crates/splashdown-core/src/` unless noted, and most have an eval:

| Prompt | Where | Eval |
|---|---|---|
| The AI pass: cleanup, spoken instructions, transforms of selected text; and the sanity guard that checks its answer | `prompt.rs`, `guard.rs` | `eval cleanup` |
| The built-in transforms' instructions, such as the meeting summary | `crates/splashdown-config/src/transforms.rs` | `eval cleanup` (the `transform-*` and `meeting-summary-*` cases) |
| Spoken commands: which action a command names | `command.rs` | `eval commands` |
| Voice actions: planning a "do" into one action | `action/planner.rs` | `eval actions` |
| Compose: drafts, changes to them, and writing in the user's voice | `compose.rs` | `eval compose`, `eval style` |
| Your writing style: the profile written from the user's edits to drafts | `style.rs` (`PROFILE_SYSTEM`) | `eval style` (the `edits-*` cases) |
| Translation: the translate key, ⌃⌥T and a preset's "Translate into"; and the length check that catches a reply that isn't a translation | `translate_instruction` in `crates/splashdown-config/src/presets.rs`, `guard.rs` | `eval cleanup` (the `translate-*` cases) |
| Translating selected text that can't be edited, answered in the Ask panel | `show_translation` in `app/src-tauri/src/session/edits.rs` | none yet |
| Ask: answers in the panel, and spoken summaries | `ask.rs` | none yet; try it by hand with `splashdown-cli ask "…"` |

Small wording changes can move results a lot, so any prompt or guard change needs an eval run, before and after, on the cloud default and the built-in local model:

```sh
cargo run --release -p splashdown-cli -- eval cleanup --ai openrouter
cargo run --release -p splashdown-cli -- eval cleanup --ai local
cargo run --release -p splashdown-cli -- eval cleanup --ai apple        # on a Mac with Apple Intelligence
```

The interpreter's prompt is shared by commands and voice actions, so a change to `command.rs` needs both `eval commands` and `eval actions`. `splashdown-cli command "…"` (add `--actions` for voice actions) shows what one command turns into. Include the scores in the pull request, and add a case for the behavior you're fixing (see below).

## How to add…

**A setting**
1. A field in the right section's struct in `crates/splashdown-config/src/settings/` (`general.rs` for most; the opening comment of `settings.rs` maps them), as an `Option` documented with what unset means, and an accessor that applies the default (`compose_style()` is `compose_style.unwrap_or(true)`). Unset stays out of `config.toml`, so the default can change later.
2. If the session needs it, a field in `Engine` (`app/src-tauri/src/engine.rs`), filled in `engine::build`. Saving rebuilds the engine, so the session sees the change.
3. A control in `app/ui/index.html`, in its tab, marked `data-setting` so a change saves the whole form.
4. A line in `collect()` in `app/ui/settings/main.js` that reads the control (save the default as `null`), and a line in the tab's render function (`renderGeneral` in `settings/general.js`, `renderPrivacy` in `settings/privacy.js`, …) that shows it.
5. If it should show something other than the default when the page is opened as a plain file, the sample state in `app/ui/settings/mock.js`.

**A Tauri command**
1. A `#[tauri::command]` function in the module that owns the feature (it can take `State<'_, AppState>` and `AppHandle`), returning `Result<T, String>` with a message the user can read. Make it `async` and move slow work off the main thread.
2. Its name in `invoke_handler` in `app/src-tauri/src/main.rs`.
3. A call from the page: `invoke("my_command", { someArg })` (camelCase in JavaScript, snake_case in Rust).
4. A `case` in the page's `mockInvoke`, if it has one (`settings/mock.js`, `onboarding.js`, `notes/preview.js`).

**A voice action**
1. A `Spec` in `CATALOG` in `crates/splashdown-core/src/action/catalog.rs`, with its parameters and what each takes, and a variant of `Action` there (with its `id` and `fields`); then its arm in `check` in `action/check.rs`, where the planner's answer is turned into an `Action`. Parameters are checked there; nothing in the catalog may send, delete or pay.
2. What it does: an arm in `carry_out` in `app/src-tauri/src/voice_actions.rs`. It runs off the session thread, so it may ask the user first (`prompt::ask`), as Shortcuts and calendar events do.
3. Its checkbox in Settings → Actions: an `<input type="checkbox" class="switch" data-action="<id>" data-setting>` in `app/ui/index.html`, and its line in `help.rs` ("What can I say?").
4. Cases in `eval/actions.toml`, including requests that must do nothing, then a run of `eval actions` and `eval commands` (an edit must never become an action).

**A Shortcuts action** (macOS)
1. An `AppIntent` in `crates/splashdown-apple-shim/swift/Sources/SplashdownAppleShim/`: `Intents.swift`, or `Notes.swift` for one about notes. Its `perform()` sends a request with `Splashdown.send(["cmd": "<name>", …])` and returns the answer. For Siri and Spotlight, add a phrase to `SplashdownShortcuts`; every phrase names the app.
2. A matching variant of `Intent` in `app/src-tauri/src/intents.rs` (`cmd` is its name in snake_case) and its arm in `answer`, returning a `Reply`. Where the control channel already has the request, send it through the session (`state.session.control`). An action that reads notes goes through `readable`, so Settings → Privacy → Shortcuts applies. Add the JSON the Swift side sends to the test `intents_read_as_swift_sends_them`.
3. Rebuild with `scripts/run-mac-app.sh`. The shim's `build.rs` extracts the actions' metadata into `target/appintents/Metadata.appintents`, which the script (and `scripts/package-mac.sh`) puts in the app bundle; without it, Shortcuts doesn't know the action exists.

**An eval case**
Add a `[[case]]` to the right file in `eval/` with a new `id`; the file's header says what the fields mean. Run just that case with `--filter <id>` on the cloud default and the built-in model, then the whole set, and put the scores in the pull request. When you compare models, add a row to the results in [eval/README.md](eval/README.md).

## Prices

Cost estimates for OpenAI and Anthropic come from the list in `crates/splashdown-providers/src/pricing.rs`. When a provider changes its prices or a new default model is added, update the table and `PRICES_AS_OF` together. OpenRouter reports exact costs and needs no entry.

## Code style

- Match the code around you. Comments are short and explain *why*; every Rust file starts with a `//!` summary of what it owns.
- Keep OS code in `splashdown-platform` (or `splashdown-apple-shim`), and settings decisions in `splashdown-config`. `splashdown-core` stays free of OS calls so it can be unit-tested.
- Every `unsafe` block gets a `// SAFETY:` comment stating why it's sound; clippy enforces this.
- User-facing text is plain and short, and names the fix ("OpenRouter rejected the key — check it in Settings → AI Models").
- Use US spelling everywhere: UI text, code, comments and docs ("color", "center", "canceled", "recognize"). Names defined elsewhere keep their own spelling, such as MCP's `notifications/cancelled`.
- The UI in `app/ui` has no build step: plain classic scripts sharing `common.js`. Keep it that way.
- Never log what the user said or typed. Logs may contain timings, provider names and error messages only.

## Pull requests

- One topic per pull request, with a description of what changed and how you tested it (which apps you dictated into, which providers).
- For UI changes, include a screenshot.
- By contributing, you agree that your work is licensed under the project's dual MIT / Apache-2.0 license (see below).

## License

Splashdown is licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion is dual-licensed as above, without any additional terms or conditions.
