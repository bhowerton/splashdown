# Contributing to Splashdown

Thanks for helping. This file covers setting up, the checks a change has to pass, and where things live. For how the code fits together, read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first; for what the app should do, [docs/SPEC.md](docs/SPEC.md).

## Setting up

**All platforms**
- Rust stable (pinned by `rust-toolchain.toml`; `rustup` picks it up).
- `cmake`, for local speech-to-text (transcribe.cpp). Without it, build the CLI with `--no-default-features`.

**macOS**
- Xcode 26 or later (the macOS 26 SDK), for the Swift shim that talks to Apple's on-device models. The app itself runs on macOS 13 and later.
- To package releases: `cargo install tauri-cli --locked` and `rustup target add x86_64-apple-darwin`.

**Windows**
- The MSVC build tools, LLVM (libclang, for the llama.cpp and transcribe.cpp bindings) and WebView2 (preinstalled on Windows 11). With winget:
  ```powershell
  winget install Rustlang.Rustup Kitware.CMake LLVM.LLVM Git.Git
  winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```
  On an ARM64 PC, also add the "MSVC ARM64 build tools" component in the Visual Studio Installer.
- The Windows platform layer (`crates/splashdown-platform/src/win/`) is complete but has barely run on real Windows; see the M3 section of [docs/ROADMAP.md](docs/ROADMAP.md).

## Building and running

```sh
cargo build --workspace
cargo test --workspace
scripts/run-mac-app.sh        # macOS: build, bundle, sign and launch the menu bar app
scripts\run-win-app.ps1       # Windows: build and launch the tray app (checks the build tools first)
```

The built-in local text model runs in a separate helper, `splashdown-llm` (see the architecture doc); `cargo build --workspace` builds it next to the app, which is where the app looks for it. Download its model with `cargo run --release -p splashdown-cli -- models download qwen3-4b` (2.5 GB).

`scripts/run-mac-app.sh` builds a real `.app` bundle so macOS grants Accessibility and Microphone to Splashdown rather than to your terminal. It signs with the first "Apple Development" certificate in your keychain, so permissions survive rebuilds. Without one it signs ad hoc, and macOS asks again after every build. Set `SPLASHDOWN_SIGN_IDENTITY` to choose a certificate. Logs go to `target/macos/splashdown.log`. On Windows, `scripts\run-win-app.ps1` needs no signing (Windows asks for no permissions), passes `.env.local` the same way, and logs to `%LOCALAPPDATA%\Splashdown\logs\splashdown.log`. If PowerShell refuses to run scripts, use `powershell -ExecutionPolicy Bypass -File scripts\run-win-app.ps1`.

**API keys for development.** Copy `.env.local.example` to `.env.local` (gitignored) and fill in what you have. The `splashdown-cli` CLI reads it from the current directory, and `run-mac-app.sh` passes it to the app. Never commit keys; the app itself stores keys in the OS keychain.

## Checks every change must pass

CI runs these on macOS and Windows; please run them before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

From a Mac you can also check that the shared crates still build for Windows (transcribe.cpp can't be cross-compiled, hence the flags):

```sh
cargo clippy --target x86_64-pc-windows-msvc --no-default-features \
  -p splashdown-core -p splashdown-audio -p splashdown-providers -p splashdown-platform -p splashdown-apple-shim -p splashdown-config \
  -p splashdown-control -p splashdown-mcp -- -D warnings
```

**Tests that need hardware or keys** are marked `#[ignore]` with the reason, and don't run by default:
- `splashdown-apple-shim`: Apple Intelligence turned on, and Apple's speech model downloaded. Run with `cargo test -p splashdown-apple-shim -- --ignored`.
- The evals (`splashdown-cli eval cleanup`, `splashdown-cli eval stt`) call real providers and need keys or local models; see [eval/README.md](eval/README.md).

## Packaging a release

```sh
scripts/package-mac.sh
```

This builds a universal (Apple Silicon + Intel) app with the hardened runtime and makes a DMG in `target/universal-apple-darwin/release/bundle/dmg/`. It signs with a "Developer ID Application" certificate if you have one. Test builds stop there; `NOTARIZE=1 scripts/package-mac.sh` also notarizes the app and the DMG with Apple (a few minutes), using an App Store Connect API key described in `.notarize.env.example`. The app's Info.plist additions and entitlements are in `app/src-tauri/`. It also refreshes `THIRD-PARTY-LICENSES.html` (with `cargo install cargo-about`), which ships inside the app.

**The updater signing key.** Installed copies only accept updates signed with the project's updater key. Its public half is `plugins.updater.pubkey` in `app/src-tauri/tauri.conf.json`; the private half lives outside the repository, at `~/.tauri/splashdown-updater.key` on the maintainer's Mac (set `UPDATER_KEY` to use another path). Never commit it. **Back it up** somewhere safe (a password manager): if it's lost, no existing install can verify a new release, and every user would have to download the next version by hand. Without the key, `package-mac.sh` still makes test builds, just without update files.

### Publishing a release

Releases are built by GitHub Actions (`.github/workflows/release.yml`), so they don't depend on anyone's Mac:

1. Bump `version` in `Cargo.toml` (workspace) and `app/src-tauri/tauri.conf.json`, commit, and push a matching tag: `git tag v0.2.0 && git push origin v0.2.0`. The workflow stops if the tag and the two versions disagree.
2. Approve the run when GitHub asks (the "release" environment). It builds the notarized Mac DMG and the Windows installer, merges their `latest.json` files into one, and creates a **draft** release with everything attached.
3. Read the draft: optionally edit the notes, and replace the placeholder `notes` in `latest.json` (Settings → About shows it). Then **Publish**. It must be a full release, not a pre-release: installed copies read `releases/latest/download/latest.json` (on launch and once a day, unless turned off), and the README's download links point at `releases/latest/download/Splashdown-mac.dmg` and `Splashdown-windows-x64-setup.exe`. Nothing installs until the user clicks.

Running the workflow by hand (Actions → Release → Run workflow) makes the same builds as downloadable artifacts, without a release. Every push to `main` also leaves an unsigned Windows test installer on its CI run (`windows-installer`). Locally, `NOTARIZE=1 scripts/package-mac.sh` and `scripts\package-win.ps1` make the same files in `…/bundle/release/`.

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

Prompts live in `crates/splashdown-core/src/prompt.rs` and the sanity guard in `guard.rs`. Small wording changes can move results a lot, so any prompt or guard change needs an eval run, before and after:

```sh
cargo run --release -p splashdown-cli -- eval cleanup --ai openrouter
cargo run --release -p splashdown-cli -- eval cleanup --ai apple        # on a Mac with Apple Intelligence
```

Include the scores in the pull request, and add a case to `eval/cleanup.toml` for the behavior you're fixing.

## Prices

Cost estimates for OpenAI and Anthropic come from the list in `crates/splashdown-providers/src/pricing.rs`. When a provider changes its prices or a new default model is added, update the table and `PRICES_AS_OF` together. OpenRouter reports exact costs and needs no entry.

## Code style

- Match the code around you. Comments are short and explain *why*; every file starts with a `//!` summary of what it owns.
- Keep OS code in `splashdown-platform` (or `splashdown-apple-shim`), and settings decisions in `splashdown-config`. `splashdown-core` stays free of OS calls so it can be unit-tested.
- Every `unsafe` block gets a `// SAFETY:` comment stating why it's sound; clippy enforces this.
- User-facing text is plain and short, and names the fix ("OpenRouter rejected the key — check it in Settings → AI Providers").
- Use US spelling everywhere: UI text, code, comments and docs ("color", "center", "canceled", "recognize"). Names defined elsewhere keep their own spelling, such as MCP's `notifications/cancelled`.
- The UI in `app/ui` has no build step: plain classic scripts sharing `common.js`. Keep it that way.
- Never log what the user said or typed. Logs may contain timings, provider names and error messages only.

## Pull requests

- One topic per pull request, with a description of what changed and how you tested it (which apps you dictated into, which providers).
- For UI changes, include a screenshot.
- By contributing, you agree that your work is licensed under the project's dual MIT / Apache-2.0 license (see below).

## License

Splashdown is licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion is dual-licensed as above, without any additional terms or conditions.
