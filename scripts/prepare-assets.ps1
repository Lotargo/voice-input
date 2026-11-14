param([switch]$Force)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'safe-path.ps1')
$repo = Split-Path -Parent $PSScriptRoot
$resources = Join-Path $repo 'src-tauri/resources'
$models = Join-Path $resources 'models'
New-Item -ItemType Directory -Force -Path $resources, $models | Out-Null

function Download-Archive([string]$Url, [string]$ZipPath) {
    Write-Host "Downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing
    $tmp = Join-Path $env:TEMP ('voice-input-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        Expand-Archive -LiteralPath $ZipPath -DestinationPath $tmp -Force
        return $tmp
    } catch {
        Remove-BuildDirectory -LiteralPath $tmp -Within $env:TEMP
        throw
    } finally { Remove-Item -LiteralPath $ZipPath -Force -ErrorAction SilentlyContinue }
}

$dll = Join-Path $resources 'libvosk.dll'
if ($Force -or !(Test-Path $dll)) {
    $zip = Join-Path $env:TEMP ('voice-input-vosk-' + [guid]::NewGuid().ToString('N') + '.zip')
    $tmp = Download-Archive 'https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-win64-0.3.45.zip' $zip
    try {
        $allDlls = @(Get-ChildItem $tmp -Recurse -File -Filter '*.dll')
        if (!($allDlls | Where-Object Name -EQ 'libvosk.dll')) { throw 'libvosk.dll absent from archive' }
        foreach ($item in $allDlls) { Copy-Item $item.FullName $resources -Force }
    } finally { Remove-BuildDirectory -LiteralPath $tmp -Within $env:TEMP }
}

$folder = Join-Path $models 'vosk-model-small-ru-0.22'
if ($Force -or !(Test-Path (Join-Path $folder 'am'))) {
    $zip = Join-Path $env:TEMP ('voice-input-model-' + [guid]::NewGuid().ToString('N') + '.zip')
    $tmp = Download-Archive 'https://alphacephei.com/vosk/models/vosk-model-small-ru-0.22.zip' $zip
    try {
        $source = Get-ChildItem $tmp -Recurse -Directory | Where-Object { $_.Name -eq 'vosk-model-small-ru-0.22' } | Select-Object -First 1
        if (!$source) { throw 'Vosk Russian model directory absent from archive' }
        Remove-BuildDirectory -LiteralPath $folder -Within $models
        Copy-Item $source.FullName -Destination $folder -Recurse -Force
    } finally { Remove-BuildDirectory -LiteralPath $tmp -Within $env:TEMP }
}
if (!(Test-Path $dll) -or !(Test-Path (Join-Path $folder 'conf'))) { throw 'Asset verification failed' }
Write-Host 'Ready: Vosk DLL + Russian model installed for local offline dictation.'
