# Replaces the media_kit libmpv-2.dll in a Windows release build with the
# VVC-capable build from VVCEasy (mpv v0.41.0, FFmpeg with the native vvc
# decoder), so the 「原始 VVC」 short-drama mode can decode bytevc2.
#
# The archive is pinned by URL and SHA-256, and the extracted DLL by SHA-256.
# VVCEasy keeps old release assets, unlike the rolling shinchiro builds.
# The DLL imports vulkan-1.dll, which media_kit's ANGLE bundle already ships.
param(
    [string]$ReleaseDirectory = '',
    [string]$CacheDirectory = '',
    [ValidateRange(1, 5)][int]$MaxAttempts = 3
)
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $ReleaseDirectory) { $ReleaseDirectory = Join-Path $repoRoot 'build/windows/x64/runner/Release' }
if (-not $CacheDirectory) { $CacheDirectory = Join-Path $repoRoot 'build/windows/x64/mpv-vvc' }

$archiveName = 'mpv-dev-x86_64-20260927-git-c646f167d0.7z'
$archiveUrl = "https://github.com/MartinEesmaa/VVCEasy/releases/download/v4.0.0/$archiveName"
$archiveSha256 = '38d148e48876dff906bc2ff4c6210fbe2a0ba13c32b013092243d18799db0bdc'
$dllSha256 = '507841c47baf777c3160a596069ed1df253fc5408d6c5169e63dd3c6092ff44b'

$target = Join-Path $ReleaseDirectory 'libmpv-2.dll'
if (-not (Test-Path -LiteralPath $target)) { throw "Missing $target; build the Windows release first." }
New-Item -ItemType Directory -Path $CacheDirectory -Force | Out-Null
$archive = Join-Path $CacheDirectory $archiveName

function Get-Sha256([string]$path) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }

if ((Test-Path -LiteralPath $archive) -and (Get-Sha256 $archive) -ne $archiveSha256) {
    Write-Warning "Removing corrupt $archiveName"
    Remove-Item -LiteralPath $archive -Force
}
if (-not (Test-Path -LiteralPath $archive)) {
    $partial = "$archive.download"
    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        try {
            if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Force }
            Write-Host "Downloading $archiveName (attempt $attempt/$MaxAttempts)"
            & curl.exe --fail --location --connect-timeout 20 --max-time 300 --output $partial --url $archiveUrl
            if ($LASTEXITCODE -ne 0) { throw "curl failed with exit code $LASTEXITCODE" }
            $actual = Get-Sha256 $partial
            if ($actual -ne $archiveSha256) { throw "Integrity check failed for $archiveName (SHA-256 $actual)" }
            Move-Item -LiteralPath $partial -Destination $archive -Force
            break
        } catch {
            Write-Warning $_.Exception.Message
            if ($attempt -eq $MaxAttempts) { throw }
            Start-Sleep -Seconds 2
        } finally {
            if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Force }
        }
    }
}

$extract = Join-Path $CacheDirectory 'extract'
if (Test-Path -LiteralPath $extract) { Remove-Item -LiteralPath $extract -Recurse -Force }
New-Item -ItemType Directory -Path $extract -Force | Out-Null
$sevenZip = (Get-Command 7z -ErrorAction SilentlyContinue).Source
if (-not $sevenZip) { $sevenZip = Join-Path $env:ProgramFiles '7-Zip/7z.exe' }
if (-not (Test-Path -LiteralPath $sevenZip)) { throw '7-Zip is required to extract the VVC libmpv archive.' }
& $sevenZip e $archive "-o$extract" libmpv-2.dll -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw "7-Zip failed with exit code $LASTEXITCODE" }
$dll = Join-Path $extract 'libmpv-2.dll'
$actualDll = Get-Sha256 $dll
if ($actualDll -ne $dllSha256) { throw "Unexpected libmpv-2.dll (SHA-256 $actualDll)" }

$before = (Get-Item -LiteralPath $target).Length
Copy-Item -LiteralPath $dll -Destination $target -Force
if ((Get-Sha256 $target) -ne $dllSha256) { throw 'libmpv-2.dll was not replaced' }
$after = (Get-Item -LiteralPath $target).Length
if (-not (Test-Path -LiteralPath (Join-Path $ReleaseDirectory 'vulkan-1.dll'))) {
    throw 'vulkan-1.dll (from the media_kit ANGLE bundle) is missing next to libmpv-2.dll'
}
Write-Host ("Installed VVC libmpv-2.dll: {0:N1} MiB -> {1:N1} MiB" -f ($before / 1MB), ($after / 1MB))
