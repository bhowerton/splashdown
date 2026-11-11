# Builds Splashdown on Windows and launches it.
#
#   powershell -ExecutionPolicy Bypass -File scripts\run-win-app.ps1
#   $env:PROFILE = "dev"; scripts\run-win-app.ps1    # faster build, slower app
#
# First time on a new PC, install the build tools (then open a new terminal):
#   winget install Rustlang.Rustup Kitware.CMake LLVM.LLVM Git.Git
#   winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
#
# Logs: %LOCALAPPDATA%\Splashdown\logs\splashdown.log
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

# Say what's missing up front rather than failing deep in a C++ build.
$missing = @()
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $missing += "Rust (winget install Rustlang.Rustup)" }
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) { $missing += "CMake (winget install Kitware.CMake)" }
# llama.cpp's and whisper.cpp's Rust bindings are generated with libclang.
if (-not $env:LIBCLANG_PATH) {
    $llvm = Join-Path $env:ProgramFiles "LLVM\bin"
    if (Test-Path (Join-Path $llvm "libclang.dll")) {
        $env:LIBCLANG_PATH = $llvm
    } else {
        $missing += "LLVM (winget install LLVM.LLVM)"
    }
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not ((Test-Path $vswhere) -and (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath))) {
    $missing += "the MSVC build tools (see the top of this script)"
}
if ($missing) {
    Write-Host "Install first, then open a new terminal:" -ForegroundColor Yellow
    $missing | ForEach-Object { Write-Host "  - $_" }
    exit 1
}

$profileName = if ($env:PROFILE) { $env:PROFILE } else { "release" }
$targetDir = if ($profileName -eq "dev") { "debug" } else { $profileName }

# The speech library is built as the installer's is, not tuned to this
# machine, so what's tried here is what ships.
. (Join-Path $PSScriptRoot "win-cpu-floor.ps1")

# The app, plus the helper that runs the built-in local text model and the
# MCP server for coding agents, which the app finds next to itself.
cargo build -p splashdown -p splashdown-llm -p splashdown-mcp --profile $profileName
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# A helper left from the previous build would keep the old model code, and
# a running copy holds the hotkey.
Stop-Process -Name splashdown-llm, splashdown -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

# Development keys, the same file run-mac-app.sh passes on.
if (Test-Path .env.local) { $env:SPLASHDOWN_ENV_FILE = (Resolve-Path .env.local).Path }

$exe = "target\$targetDir\splashdown.exe"
Start-Process $exe
$log = Join-Path $env:LOCALAPPDATA "Splashdown\logs\splashdown.log"
Write-Host "Launched $exe"
Write-Host "Logs: Get-Content -Wait `"$log`""
