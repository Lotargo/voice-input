param([switch]$SkipPrepare, [string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'safe-path.ps1')
. (Join-Path $PSScriptRoot 'windows-icon.ps1')

if ($env:OS -ne 'Windows_NT') {
    throw 'Сборка поддерживается только на Windows.'
}

$repo = Split-Path -Parent $PSScriptRoot
$tauri = Join-Path $repo 'src-tauri'
$resources = Join-Path $tauri 'resources'
$release = Join-Path $tauri 'target/release'
$out = if ($OutputDirectory) { [IO.Path]::GetFullPath($OutputDirectory) } else { Join-Path $repo 'dist' }
$portable = Join-Path $out 'voice-input-portable'
$archive = Join-Path $out 'voice-input-windows-portable.zip'

if (-not $SkipPrepare) {
    & (Join-Path $PSScriptRoot 'prepare-assets.ps1')
}

if (-not (Test-Path (Join-Path $resources 'libvosk.dll'))) {
    throw 'libvosk.dll не найден. Сначала запустите scripts/prepare-assets.ps1.'
}
if (-not (Test-Path (Join-Path $resources 'models/vosk-model-small-ru-0.22/am'))) {
    throw 'Модель Vosk Small RU не найдена. Сначала запустите scripts/prepare-assets.ps1.'
}

Push-Location $tauri
try {
    & cargo tauri build --bundles nsis
    if ($LASTEXITCODE -ne 0) {
        throw "cargo tauri build завершился с кодом $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$exe = Join-Path $release 'voice-input.exe'
if (-not (Test-Path $exe)) { throw "Не найден бинарник $exe" }

Assert-WindowsExecutableIcon -ExecutablePath $exe -IconPath (Join-Path $tauri 'icons/icon.ico')
$setupDir = Join-Path $release 'bundle/nsis'
$setups = @(Get-ChildItem -LiteralPath $setupDir -Filter '*.exe' -File -ErrorAction Stop)
if ($setups.Count -eq 0) { throw 'NSIS-установщик не найден после сборки' }

New-Item -ItemType Directory -Path $out -Force | Out-Null
Remove-BuildDirectory -LiteralPath $portable -Within $out
New-Item -ItemType Directory -Path $portable -Force | Out-Null

Copy-Item -LiteralPath $exe -Destination $portable
$dlls = @(Get-ChildItem -LiteralPath $resources -Filter '*.dll' -File)
if ($dlls.Count -eq 0) { throw 'Нативные DLL для portable-версии отсутствуют' }
foreach ($dll in $dlls) {
    Copy-Item -LiteralPath $dll.FullName -Destination $portable
}
Copy-Item -LiteralPath (Join-Path $resources 'models') -Destination (Join-Path $portable 'models') -Recurse

# Check the copied runtime with both ordinary and Tauri-style canonical paths.
$previousSmokeRoot = $env:VOICE_INPUT_SMOKE_ROOT
Push-Location $tauri
try {
    $env:VOICE_INPUT_SMOKE_ROOT = $portable
    & cargo test --locked --test vosk_runtime
    if ($LASTEXITCODE -ne 0) {
        throw "Проверка portable Vosk завершилась с кодом $LASTEXITCODE"
    }
} finally {
    $env:VOICE_INPUT_SMOKE_ROOT = $previousSmokeRoot
    Pop-Location
}

if (Test-Path $archive) { Remove-Item $archive -Force }
Compress-Archive -Path (Join-Path $portable '*') -DestinationPath $archive -CompressionLevel Optimal

foreach ($setup in $setups) {
    Assert-WindowsExecutableIcon -ExecutablePath $setup.FullName -IconPath (Join-Path $tauri 'icons/icon.ico')
    Copy-Item -LiteralPath $setup.FullName -Destination $out -Force
}

Write-Host "Portable: $archive"
Write-Host "NSIS installers: $out"
Write-Host 'Для portable на целевой машине необходим Microsoft WebView2 Runtime.'
