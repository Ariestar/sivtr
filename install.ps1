#Requires -Version 5.1
<#
.SYNOPSIS
    sivtr installer for Windows.
.DESCRIPTION
    Downloads a prebuilt sivtr binary from GitHub releases and installs it
    to %LOCALAPPDATA%\Programs\sivtr (override with $env:SIVTR_INSTALL_DIR),
    adding that folder to the user PATH. No administrator rights required.
.NOTES
    https://github.com/Ariestar/sivtr
.EXAMPLE
    irm https://raw.githubusercontent.com/Ariestar/sivtr/main/install.ps1 | iex
#>

$ErrorActionPreference = 'Stop'

# Keep TLS healthy on older PowerShell hosts.
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocol]::Tls12 } catch {}

$Repo       = 'Ariestar/sivtr'
$Binary     = 'sivtr.exe'
$InstallDir = if ($env:SIVTR_INSTALL_DIR) { $env:SIVTR_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\sivtr' }
$Arch       = 'x86_64'   # only x64 prebuilt today

function Info($m) { Write-Host "[INFO] $m" -ForegroundColor Green }
function Warn($m) { Write-Host "[WARN] $m"  -ForegroundColor Yellow }
function Err($m)  { Write-Host "[ERROR] $m" -ForegroundColor Red; exit 1 }

# --- resolve version -------------------------------------------------------
$Version = $env:SIVTR_VERSION
if (-not $Version) {
    try {
        $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'sivtr-installer' }
        $Version = $latest.tag_name
    } catch {
        Err "Failed to query latest release. Set `$env:SIVTR_VERSION = 'vX.Y.Z' and retry."
    }
}
if (-not $Version) { Err "Could not determine version. Set `$env:SIVTR_VERSION = 'vX.Y.Z'." }

$Asset = "sivtr-$Version-windows-x64.zip"
$Url   = "https://github.com/$Repo/releases/download/$Version/$Asset"

Info "Detected: Windows $Arch"
Info "Asset:   $Asset"
Info "Version: $Version"

# --- download --------------------------------------------------------------
$tmp  = Join-Path $env:TEMP ("sivtr-install-{0}" -f (Get-Random))
$zip  = Join-Path $tmp $Asset
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

Info "Downloading $Url"
try {
    Invoke-WebRequest -Uri $Url -OutFile $zip -UseBasicParsing
} catch {
    Err "Download failed: $($_.Exception.Message)"
}

# --- extract ---------------------------------------------------------------
Info "Extracting..."
$extract = Join-Path $tmp 'out'
try {
    Expand-Archive -LiteralPath $zip -DestinationPath $extract -Force
} catch {
    Err "Extraction failed: $($_.Exception.Message)"
}

$exe = Get-ChildItem -Path $extract -Recurse -Filter $Binary -File | Select-Object -First 1
if (-not $exe) { Err "Could not find $Binary in the archive." }

# --- install ---------------------------------------------------------------
Info "Installing to $InstallDir"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item -LiteralPath $exe.FullName -Destination (Join-Path $InstallDir $Binary) -Force

# --- PATH (user scope; no admin) -------------------------------------------
# Writes the user environment, which Windows stores in HKCU\Environment under
# the hood. This is the supported .NET API, not a manual registry edit.
$userPathEntries = [Environment]::GetEnvironmentVariable('Path', 'User') -split ';' |
    Where-Object { $_ }
if ($userPathEntries -contains $InstallDir) {
    Info "Already on user PATH."
} else {
    $newPath = if ($userPathEntries) { ($InstallDir, ($userPathEntries -join ';')) -join ';' } else { $InstallDir }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    $env:Path = "$InstallDir;$env:Path"
    Info "Added $InstallDir to user PATH."
    Warn "PATH is updated for new terminals. Restart your terminal to use 'sivtr'."
}

# --- verify ----------------------------------------------------------------
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue

$exePath = Join-Path $InstallDir $Binary
$ver = & $exePath --version
Info "Verification: $ver"

Write-Host ''
Info "Done. Next steps:"
Info "  sivtr init powershell   # enable shell capture"
Info "  sivtr doctor            # verify environment"
