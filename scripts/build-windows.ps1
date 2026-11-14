param([switch]$SkipPrepare, [string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'safe-path.ps1')
. (Join-Path $PSScriptRoot 'windows-icon.ps1')
. (Join-Path $PSScriptRoot 'release-common.ps1')
if ($env:OS -ne 'Windows_NT') { throw 'Windows is required.' }
$repo = Split-Path -Parent $PSScriptRoot
$tauri = Join-Path $repo 'src-tauri'
$resources = Join-Path $tauri 'resources'
$release = Join-Path $tauri 'target/release'
$version = Get-VoiceInputVersion $repo
$out = if ($OutputDirectory) { [IO.Path]::GetFullPath($OutputDirectory) } else { Join-Path $repo 'dist' }
$portableDirectory = Join-Path $out "portable/$version"
$portable = Join-Path $portableDirectory 'Voice-Input'
$installerDirectory = Join-Path $out "installers/$version"
$metadata = Join-Path $out "releases/$version"
$portableName = 'Voice-Input-Portable-windows-x64.zip'
$installerName = 'Voice-Input-Setup-windows-x64.exe'
$archive = Join-Path $portableDirectory $portableName
$installer = Join-Path $installerDirectory $installerName
$notesPath = Join-Path $repo "releases/$version.md"
if (-not (Test-Path -LiteralPath $notesPath)) { throw "Add release notes: $notesPath" }
if (-not $SkipPrepare) { & (Join-Path $PSScriptRoot 'prepare-assets.ps1') }
if (-not (Test-Path -LiteralPath (Join-Path $resources 'libvosk.dll'))) { throw 'Prepare Vosk assets first.' }
if (-not (Test-Path -LiteralPath (Join-Path $resources 'models/vosk-model-small-ru-0.22/am'))) { throw 'Prepare the Russian Vosk model first.' }

$oldKey = $env:TAURI_SIGNING_PRIVATE_KEY
$oldPassword = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
$oldBuildJobs = $env:CARGO_BUILD_JOBS
Enable-VoiceInputSigning
try {
    # Bound concurrent compiler memory while keeping the optimized release profile.
    if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = '1' }
    Push-Location $tauri
    try {
        & cargo tauri build --bundles nsis
        if ($LASTEXITCODE -ne 0) { throw 'Tauri build failed.' }
    } finally { Pop-Location }
    $exe = Join-Path $release 'voice-input.exe'
    Assert-WindowsExecutableIcon -ExecutablePath $exe -IconPath (Join-Path $tauri 'icons/icon.ico')
    $sourceInstaller = Join-Path $release "bundle/nsis/Voice Input_${version}_x64-setup.exe"
    if (-not (Test-Path -LiteralPath $sourceInstaller)) { throw "Missing installer: $sourceInstaller" }
    Assert-WindowsExecutableIcon -ExecutablePath $sourceInstaller -IconPath (Join-Path $tauri 'icons/icon.ico')
    foreach ($directory in @($portableDirectory, $installerDirectory, $metadata)) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
    $portableFull = [IO.Path]::GetFullPath($portable).TrimEnd('\')
    foreach ($process in @(Get-CimInstance Win32_Process -Filter "Name='voice-input.exe'")) {
        if ($process.ExecutablePath -and [IO.Path]::GetFullPath($process.ExecutablePath).StartsWith($portableFull + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw 'The portable output is running. Close that copy before rebuilding.'
        }
    }
    Remove-BuildDirectory -LiteralPath $portable -Within $out
    New-Item -ItemType Directory -Path $portable -Force | Out-Null
    Copy-Item -LiteralPath $exe -Destination $portable
    foreach ($dll in @(Get-ChildItem -LiteralPath $resources -Filter '*.dll' -File)) { Copy-Item -LiteralPath $dll.FullName -Destination $portable }
    Copy-Item -LiteralPath (Join-Path $resources 'models') -Destination (Join-Path $portable 'models') -Recurse
    Write-VoiceInputJson (Join-Path $portable 'portable.json') @{ app_id = 'dev.lotargo.voiceinput'; kind = 'portable'; version = $version }
    $oldSmokeRoot = $env:VOICE_INPUT_SMOKE_ROOT
    Push-Location $tauri
    try {
        $env:VOICE_INPUT_SMOKE_ROOT = $portable
        & cargo test --locked --test vosk_runtime
        if ($LASTEXITCODE -ne 0) { throw 'Portable Vosk smoke test failed.' }
    } finally { $env:VOICE_INPUT_SMOKE_ROOT = $oldSmokeRoot; Pop-Location }
    if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
    # Keep a named root inside the ZIP: extraction is also used by the portable updater.
    Compress-Archive -LiteralPath $portable -DestinationPath $archive -CompressionLevel Optimal
    Copy-Item -LiteralPath $sourceInstaller -Destination $installer -Force
    Copy-Item -LiteralPath "$sourceInstaller.sig" -Destination "$installer.sig" -Force
    $signOutput = & cargo tauri signer sign --app-version $version $archive 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Portable signing failed: $signOutput" }
    $notes = (Get-Content -LiteralPath $notesPath -Raw).Trim()
    $changes = [regex]::Match($notes, '(?s)## Что нового\r?\n\s*(.+?)(?:\r?\n\r?\n\*\*Переход|\z)')
    if ($changes.Success) { $notes = $changes.Groups[1].Value.Trim() }
    $date = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
    $baseUrl = "https://github.com/Lotargo/voice-input/releases/download/v$version"
    foreach ($item in @(
        @{ Manifest = 'latest.json'; Artifact = $installer; Name = $installerName },
        @{ Manifest = 'latest-portable.json'; Artifact = $archive; Name = $portableName }
    )) {
        $signature = (Get-Content -LiteralPath ($item.Artifact + '.sig') -Raw).Trim()
        Write-VoiceInputJson (Join-Path $metadata $item.Manifest) @{
            version = $version; notes = $notes; pub_date = $date
            platforms = @{ 'windows-x86_64' = @{ url = "$baseUrl/$($item.Name)"; signature = $signature } }
        }
    }
    $assets = @($archive, "$archive.sig", $installer, "$installer.sig", (Join-Path $metadata 'latest.json'), (Join-Path $metadata 'latest-portable.json'))
    $hashes = foreach ($asset in $assets) { '{0}  {1}' -f (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant(), [IO.Path]::GetFileName($asset) }
    [IO.File]::WriteAllText((Join-Path $metadata 'SHA256SUMS.txt'), ($hashes -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
    $commit = (& git -C $repo rev-parse HEAD).Trim()
    Write-VoiceInputJson (Join-Path $metadata 'build.json') @{ version = $version; source_commit = $commit; built_at = $date }
    Write-Output "Portable: $archive"
    Write-Output "Installer: $installer"
    Write-Output "Release metadata: $metadata"
} finally {
    $env:TAURI_SIGNING_PRIVATE_KEY = $oldKey
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $oldPassword
    $env:CARGO_BUILD_JOBS = $oldBuildJobs
}
