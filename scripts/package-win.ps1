# Builds a release installer of Splashdown for Windows (x64): an NSIS
# setup .exe with the local-model helper inside.
#
#   powershell -ExecutionPolicy Bypass -File scripts\package-win.ps1
#
# Needs the build tools listed at the top of scripts\run-win-app.ps1, plus
# `cargo install tauri-cli --locked`.
#
# Code signing (Authenticode): with Azure Artifact Signing credentials in
# AZURE_CLIENT_ID, AZURE_CLIENT_SECRET and AZURE_TENANT_ID (release builds in
# CI), the app, the local-model helper and the installer are signed as
# Point Dynamics, LLC. Needs `cargo install artifact-signing-cli`. Without
# them the build is unsigned and SmartScreen warns on first run.
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
# (Tauri's "external binary", named for the target), in two builds, and the
# app picks one when it starts (local_llm.rs):
# - splashdown-llm: for processors with AVX2 (nearly every PC since 2013),
#   which llama.cpp leans on, with its Vulkan backend for the graphics chip
#   when the Vulkan SDK is installed (VULKAN_SDK; CI installs it);
# - splashdown-llm-basic: for any x64 processor, processor only.
New-Item -ItemType Directory -Force app\src-tauri\binaries | Out-Null
cargo build --release -p splashdown-llm
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Copy-Item target\release\splashdown-llm.exe "app\src-tauri\binaries\splashdown-llm-basic-$triple.exe"
$llmFeatures = @()
if ($env:VULKAN_SDK) {
    # MSBuild in Visual Studio 2026 runs the steps of llama.cpp's shader
    # generator (a CMake sub-project) side by side, installing it before
    # it's built, so the Vulkan build uses Ninja, which Visual Studio's
    # developer shell puts on the path along with its compiler.
    if (-not (Get-Command ninja -ErrorAction SilentlyContinue) -or -not (Get-Command cl -ErrorAction SilentlyContinue)) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
        $vs = if (Test-Path $vswhere) {
            & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        }
        if ($vs) {
            Import-Module (Join-Path $vs "Common7\Tools\Microsoft.VisualStudio.DevShell.dll")
            Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments "-arch=x64 -host_arch=x64" | Out-Null
        }
    }
    if (Get-Command ninja -ErrorAction SilentlyContinue) {
        $llmFeatures = @("--features", "vulkan")
        $env:CMAKE_GENERATOR = "Ninja"
    } else {
        Write-Host "warning: no Ninja (Visual Studio's C++ CMake tools); the helper won't use the graphics chip" -ForegroundColor Yellow
    }
} else {
    Write-Host "warning: no Vulkan SDK (VULKAN_SDK); the helper won't use the graphics chip" -ForegroundColor Yellow
}
# Its own target folder (different compiler flags would rebuild everything),
# and a short one: llama.cpp's Vulkan build nests a CMake project deep
# enough under target\ (274 characters) to pass Windows' 260-character
# limit, which some build tools still keep to.
$llmTarget = if ($env:SPLASHDOWN_LLM_TARGET) { $env:SPLASHDOWN_LLM_TARGET } else { Join-Path $env:SystemDrive "\sdllm" }
$env:RUSTFLAGS = "-C target-feature=+avx,+avx2,+fma,+f16c,+bmi2"
cargo build --release -p splashdown-llm @llmFeatures --target-dir $llmTarget
$built = $LASTEXITCODE
Remove-Item Env:RUSTFLAGS
Remove-Item Env:CMAKE_GENERATOR -ErrorAction SilentlyContinue
if ($built -ne 0) { exit $built }
Copy-Item (Join-Path $llmTarget "release\splashdown-llm.exe") "app\src-tauri\binaries\splashdown-llm-$triple.exe"

# The MCP server coding agents are set up with (Settings → Privacy →
# Coding agents) ships next to the app too.
cargo build --release -p splashdown-mcp
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Copy-Item target\release\splashdown-mcp.exe "app\src-tauri\binaries\splashdown-mcp-$triple.exe"

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
$configs = @("--config", "app/src-tauri/tauri.release.conf.json",
    "--config", '{"bundle":{"externalBin":["binaries/splashdown-llm","binaries/splashdown-llm-basic","binaries/splashdown-mcp"]}}')
if (-not $updater) { $configs += @("--config", '{"bundle":{"createUpdaterArtifacts":false}}') }

# Tauri runs signCommand on every binary it bundles and on the installer,
# before it makes the update signature, so the two always agree.
$signed = [bool]$env:AZURE_CLIENT_SECRET
if ($signed) {
    $signCommand = "artifact-signing-cli -e https://eus.codesigning.azure.net -a pointdynamics -c PointDynamicsPublic -d Splashdown %1"
    $signConfig = Join-Path (Get-Location) "target\tauri.sign.conf.json"
    New-Item -ItemType Directory -Force target | Out-Null
    $json = @{ bundle = @{ windows = @{ signCommand = $signCommand } } } | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText($signConfig, $json)
    $configs += @("--config", $signConfig)
} else {
    Write-Host "warning: no Azure signing credentials; the installer won't be code-signed" -ForegroundColor Yellow
}

cargo tauri build --bundles nsis @configs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$bundle = "target\release\bundle"
$setup = Get-ChildItem "$bundle\nsis\*-setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
$version = (Get-Content -Raw app\src-tauri\tauri.conf.json | ConvertFrom-Json).version

if ($signed) {
    # Fail rather than publish an installer that only looks signed.
    $signature = Get-AuthenticodeSignature $setup.FullName
    if ($signature.Status -ne "Valid" -or $signature.SignerCertificate.Subject -notmatch "Point Dynamics") {
        Write-Host "error: the installer's signature is $($signature.Status) ($($signature.SignerCertificate.Subject))" -ForegroundColor Red
        exit 1
    }
    Write-Host "Signed by: $($signature.SignerCertificate.Subject)"
}

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
