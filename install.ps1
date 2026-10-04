# Installs the BrainWashed command-line host on Windows and starts it:
#
#   irm https://raw.githubusercontent.com/ahmadalshouly/brainwashed/main/install.ps1 | iex
#
# It downloads `brainwashed.exe` from the newest GitHub release (pre-releases
# included) into %LOCALAPPDATA%\Programs\BrainWashed, adds that folder to your
# PATH, and runs it. Set $env:BRAINWASHED_VERSION = "v0.1.0" first to pick a
# release, or $env:BRAINWASHED_NO_START = "1" to only install.

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"   # Invoke-WebRequest is very slow with the progress bar.

$repo = "ahmadalshouly/brainwashed"
$asset = "brainwashed-x86_64-pc-windows-msvc.zip"   # Also runs on Windows on Arm through emulation.
$dir = Join-Path $env:LOCALAPPDATA "Programs\BrainWashed"

if ($env:BRAINWASHED_VERSION) {
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/tags/$($env:BRAINWASHED_VERSION)"
} else {
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases?per_page=20" |
        Where-Object { -not $_.draft -and ($_.assets.name -contains $asset) } |
        Select-Object -First 1
}
$download = $release.assets | Where-Object { $_.name -eq $asset } | Select-Object -First 1
if (-not $download) {
    Write-Host "No BrainWashed release with a Windows command-line build was found yet." -ForegroundColor Red
    Write-Host "See https://github.com/$repo/releases"
    return
}

Write-Host "Downloading BrainWashed $($release.tag_name)..."
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$zip = Join-Path $env:TEMP $asset
Invoke-WebRequest $download.browser_download_url -OutFile $zip
Expand-Archive -Force $zip $dir
Remove-Item $zip
# Downloaded files are marked as coming from the internet; this one is wanted.
Unblock-File (Join-Path $dir "brainwashed.exe")

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($userPath -split ";") -notcontains $dir) {
    $newPath = if ($userPath) { "$userPath;$dir" } else { $dir }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Added $dir to your PATH. New terminals can run 'brainwashed' directly."
}
$env:Path = "$env:Path;$dir"

Write-Host "Installed. Run 'brainwashed --help' to see what it can do."
if (-not $env:BRAINWASHED_NO_START) {
    & (Join-Path $dir "brainwashed.exe")
}
