# Builds a release installer of Splashdown for Windows (x64): an NSIS
# setup .exe with the local-model helper inside.
#
#   powershell -ExecutionPolicy Bypass -File scripts\package-win.ps1
#
# Needs the build tools listed at the top of scripts\run-win-app.ps1, plus
# `cargo install tauri-cli --locked`.
#
# Code signing: not set up yet, so Windows SmartScreen warns on first run.
#
# Updates: with the updater signing key (TAURI_SIGNING_PRIVATE_KEY, or the
# file named by UPDATER_KEY; see CONTRIBUTING.md), Tauri signs the installer
# for the updater and this script writes latest.json for the GitHub release.
# Without it, the installer is made without update files.
# Everything to upload ends up in target\release\bundle\release\.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

# llama.cpp's and whisper.cpp's Rust bindings are generated with libclang.
if (-not $env:LIBCLANG_PATH) {
    $llvm = Join-Path $env:ProgramFiles "LLVM\bin"
    if (Test-Path (Join-Path $llvm "libclang.dll")) { $env:LIBCLANG_PATH = $llvm }
}

$triple = ((rustc -vV | Select-String "^host:").ToString() -split " ")[1]
if ($triple -ne "x86_64-pc-windows-msvc") {
    Write-Host "warning: building for $triple; release installers are x64" -ForegroundColor Yellow
}

# The helper that runs the built-in local text model ships next to the app
# (Tauri's "external binary", named for the target).
cargo build --release -p splashdown-llm
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force app\src-tauri\binaries | Out-Null
Copy-Item target\release\splashdown-llm.exe "app\src-tauri\binaries\splashdown-llm-$triple.exe"

# The updater signing key. The key itself never goes into the repository.
$updater = $true
if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
    $keyFile = if ($env:UPDATER_KEY) { $env:UPDATER_KEY } else { Join-Path $HOME ".tauri\splashdown-updater.key" }
    if (Test-Path $keyFile) {
        $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw $keyFile
        if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "" }
    } else {
        Write-Host "warning: no updater signing key; building without update files" -ForegroundColor Yellow
        $updater = $false
    }
}
$configs = @("--config", "app/src-tauri/tauri.release.conf.json")
if (-not $updater) { $configs += @("--config", '{"bundle":{"createUpdaterArtifacts":false}}') }

cargo tauri build --bundles nsis @configs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$bundle = "target\release\bundle"
$setup = Get-ChildItem "$bundle\nsis\*-setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
$version = (Get-Content -Raw app\src-tauri\tauri.conf.json | ConvertFrom-Json).version

# Release files: the installer under a name without the version, so the
# website's link (releases/latest/download/Splashdown-windows-x64-setup.exe)
# never changes. The same file is what the updater downloads.
$release = "$bundle\release"
Remove-Item -Recurse -Force $release -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $release | Out-Null
$name = "Splashdown-windows-x64-setup.exe"
Copy-Item $setup.FullName "$release\$name"
if ($updater) {
    Copy-Item "$($setup.FullName).sig" "$release\$name.sig"
    $latest = [ordered]@{
        version  = $version
        notes    = "See the release notes on GitHub."
        pub_date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
        platforms = [ordered]@{
            "windows-x86_64" = [ordered]@{
                signature = (Get-Content -Raw "$release\$name.sig").Trim()
                url       = "https://github.com/bhowerton/splashdown/releases/download/v$version/$name"
            }
        }
    }
    # The release workflow merges this with the Mac's latest.json.
    # (UTF-8 without a BOM, which Windows PowerShell's Set-Content can't do.)
    $json = $latest | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText((Join-Path (Get-Location) "$release\latest.json"), $json)
}

Write-Host ""
Write-Host "Installer: $($setup.FullName)"
Write-Host "Release files: $release"
