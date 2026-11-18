# Install hurad on Windows, without a checkout and without a Rust toolchain.
#
#   irm https://raw.githubusercontent.com/tobiaswadsethdev/hura/main/install.ps1 | iex
#
# The Windows half of install.sh, doing the same job the same way: it fetches
# the newest release for this machine, checks it against the published
# SHA256SUMS, and puts `hurad.exe` in a directory, ~/.local/bin unless told
# otherwise, where Claude Code's own installer puts `claude.exe`. Nothing else:
# what hurad needs at runtime (Docker Sandboxes, Docker to build images, and
# for worktree sessions Git for Windows, psmux and jq) is what `hurad doctor`
# is for, and it is run at the end to say which are missing.
#
# Options, as parameters or as environment variables:
#
#   -Version 0.57.0   HURA_VERSION       a specific release; default the newest
#   -BinDir DIR       HURA_BIN_DIR       where to install; default ~/.local/bin
#   -FromSource       HURA_FROM_SOURCE=1 build with cargo instead of downloading
#
# Piped into `iex` there is no way to pass a parameter, so either set the
# variable first or run it as a script block:
#
#   & ([scriptblock]::Create((irm .../install.ps1))) -BinDir C:\tools
#
# Windows PowerShell 5.1, which every Windows 10 and 11 has, is enough.

param(
    [string]$Version = $env:HURA_VERSION,
    [string]$BinDir = $env:HURA_BIN_DIR,
    [switch]$FromSource = [bool]$env:HURA_FROM_SOURCE
)

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 still offers TLS 1.0 first, which GitHub refuses.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
# The progress bar slows Invoke-WebRequest down by an order of magnitude.
$ProgressPreference = 'SilentlyContinue'

$Repo = 'tobiaswadsethdev/hura'
$Target = 'x86_64-pc-windows-msvc'

function Say([string]$Text) { Write-Host $Text }
# `throw` rather than `exit`: piped into `iex`, this runs in the caller's own
# session, and `exit` would close their window along with the script.
function Die([string]$Text) {
    throw "install.ps1: $Text"
}

if (-not $BinDir) { $BinDir = Join-Path $HOME '.local\bin' }

# ------------------------------------------------------------- from source

function Install-FromSource {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Die "no release to install and no cargo to build with.
     fix: install Rust from https://rustup.rs (with the MSVC build tools), then re-run this"
    }
    Say '==> building from source with cargo (this takes a few minutes)'
    & cargo install --git "https://github.com/$Repo" hurad --locked
    if ($LASTEXITCODE -ne 0) { Die 'cargo install failed' }
    $cargoBin = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'bin' } else { Join-Path $HOME '.cargo\bin' }
    Say "==> installed to $cargoBin\hurad.exe"
    Finish $cargoBin
}

# ---------------------------------------------------------------- releases

# hurad's releases are tagged `hurad-v<version>`, and were `v<version>` up to
# v0.54.0. The desktop and iPhone apps release from the same repository, so
# GitHub's single "latest release" may well be one of theirs.
$HuradTag = '^(hurad-)?v\d+\.\d+\.\d+$'

function Get-LatestTag {
    try {
        $releases = Invoke-RestMethod -TimeoutSec 20 `
            -Headers @{ Accept = 'application/vnd.github+json' } `
            -Uri "https://api.github.com/repos/$Repo/releases?per_page=50"
    } catch {
        return $null
    }
    # Newest first, as GitHub sorts them; the first hurad one that has a
    # Windows build, since releases before Windows support have none.
    foreach ($r in $releases) {
        if ($r.draft -or $r.prerelease -or $r.tag_name -notmatch $HuradTag) { continue }
        $names = @($r.assets | ForEach-Object { $_.name })
        if ($names | Where-Object { $_ -like "*-$Target.tar.gz" }) { return $r.tag_name }
    }
    return $null
}

# The tag for a version asked for by hand, which may be written `0.57.0`,
# `v0.57.0` or `hurad-v0.57.0`.
function Get-TagFor([string]$Asked) {
    $v = $Asked -replace '^hurad-', '' -replace '^v', ''
    return "hurad-v$v"
}

# --------------------------------------------------------------------- steps

function Install-Release {
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        Die "no release is built for Windows on $($env:PROCESSOR_ARCHITECTURE)
     fix: -FromSource, which needs a Rust toolchain"
    }

    if ($Version) {
        $tag = Get-TagFor $Version
    } else {
        Say '==> looking for the newest release'
        $tag = Get-LatestTag
    }
    if (-not $tag) {
        Write-Host "install.ps1: no published Windows release found for $Repo" -ForegroundColor Yellow
        Say 'falling back to building from source'
        Install-FromSource
        return
    }

    $version = $tag -replace '^hurad-', '' -replace '^v', ''
    $asset = "hurad-v$version-$Target.tar.gz"
    $base = "https://github.com/$Repo/releases/download/$tag"

    $tmp = Join-Path ([IO.Path]::GetTempPath()) "hura-install-$PID"
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    try {
        Say "==> downloading $asset"
        try {
            Invoke-WebRequest -UseBasicParsing -TimeoutSec 300 -Uri "$base/$asset" -OutFile (Join-Path $tmp $asset)
        } catch {
            Die "could not download $base/$asset
     fix: check the tag exists at https://github.com/$Repo/releases"
        }
        try {
            Invoke-WebRequest -UseBasicParsing -TimeoutSec 60 -Uri "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')
        } catch {
            Die "release $tag publishes no SHA256SUMS, so nothing can be verified"
        }

        # An unverified binary is not installed. The file covers every
        # platform's asset; only this one's line matters.
        Say '==> verifying'
        $expected = $null
        foreach ($line in Get-Content (Join-Path $tmp 'SHA256SUMS')) {
            $parts = $line -split '\s+', 2
            if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $asset) { $expected = $parts[0].ToLower() }
        }
        if (-not $expected) { Die "SHA256SUMS does not cover $asset" }
        $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $asset)).Hash.ToLower()
        if ($actual -ne $expected) {
            Die "checksum mismatch for $asset -- do not run it
     fix: report this at https://github.com/$Repo/security/advisories/new"
        }

        # Windows' own tar, which reads a .tar.gz: a Git Bash tar earlier on
        # PATH would take the C: of the path for a host to reach over ssh.
        $tar = Join-Path $env:SystemRoot 'System32\tar.exe'
        & $tar -xzf (Join-Path $tmp $asset) -C $tmp
        if ($LASTEXITCODE -ne 0) { Die "could not unpack $asset" }
        $fresh = Join-Path $tmp 'hurad.exe'
        if (-not (Test-Path $fresh)) { Die "$asset does not contain hurad.exe" }

        New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
        Install-Binary $fresh (Join-Path $BinDir 'hurad.exe')
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }

    Say "==> installed $tag to $BinDir\hurad.exe"
    Finish $BinDir
}

# Windows will not replace an .exe that is running, a server under Task
# Scheduler for one, but it will rename it, and the process carries on from
# the new name. So a running one moves aside first, to the name `hurad update`
# gives it, where that server still finds its own binary to start relays
# with, and where the next hurad to start deletes it once nothing runs it.
function Install-Binary([string]$From, [string]$To) {
    $dir = Split-Path $To
    $staged = Join-Path $dir ".hurad-install-$PID.exe"
    try {
        Copy-Item -Force $From $staged
    } catch {
        Die "cannot write to $dir
     fix: -BinDir DIR for somewhere you own"
    }
    try {
        Move-Item -Force $staged $To
        return
    } catch {
        if (-not (Test-Path $To)) {
            Remove-Item -Force $staged -ErrorAction SilentlyContinue
            Die "cannot replace $To"
        }
    }
    $running = 'unknown'
    try {
        $said = & $To --version 2>$null
        if ($said -match '^hurad (\S+)') { $running = $Matches[1] }
    } catch {}
    $aside = Join-Path $dir ".hurad-v$running-old.exe"
    if (Test-Path $aside) {
        try { Remove-Item -Force $aside } catch { $aside = Join-Path $dir ".hurad-v$running-old-$PID.exe" }
    }
    try {
        Move-Item -Force $To $aside
        Move-Item -Force $staged $To
    } catch {
        Remove-Item -Force $staged -ErrorAction SilentlyContinue
        Die "cannot replace $To"
    }
}

# What to do next, and the one thing that silently goes wrong: a binary in a
# directory that is not on PATH looks like an install that did nothing.
function Finish([string]$Dir) {
    $onPath = ($env:PATH -split ';') | Where-Object { $_.TrimEnd('\') -ieq $Dir.TrimEnd('\') }
    if (-not $onPath) {
        Say ''
        Say "    $Dir is not on your PATH. Add it for your account:"
        Say ''
        Say "        [Environment]::SetEnvironmentVariable('Path', `"$Dir;`" + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')"
        Say ''
        Say '    then open a new terminal.'
        Say ''
    }
    $hurad = Join-Path $Dir 'hurad.exe'
    if (Test-Path $hurad) {
        Say ''
        Say '==> hurad doctor'
        # Never fatal: doctor exits non-zero when a prerequisite is missing,
        # which is the normal state of a machine that has just installed this.
        & $hurad doctor
        Say ''
        Say 'Prerequisites and what to do about them:'
        Say "    https://github.com/$Repo/blob/main/docs/install.md#windows"
    }
}

# ---------------------------------------------------------------------- main

if ($FromSource) {
    Install-FromSource
} else {
    Install-Release
}
