param(
    [string]$ModelRoot = (Join-Path $env:LOCALAPPDATA 'dev.lotargo.voiceinput'),
    [string]$Probe,
    [string]$OutputDirectory,
    [switch]$PrepareOnly
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Probe) { $Probe = Join-Path $repo 'src-tauri/target/debug/model-check.exe' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repo 'dist/qa/models/recognition' }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
Add-Type -AssemblyName System.Speech
$speaker = New-Object System.Speech.Synthesis.SpeechSynthesizer
$voice = $speaker.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -eq 'ru-RU' } | Select-Object -First 1
if (-not $voice) { throw 'A local Russian Windows SAPI voice is required for this synthetic-only check.' }
$speaker.SelectVoice($voice.VoiceInfo.Name)
$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$phrases = @(
    @{ id = 'long-words'; text = 'Идентификация пользователя и реализация алгоритма завершены.' },
    @{ id = 'technical'; text = 'Система поддерживает аутентификацию, сериализацию и многопоточную обработку.' },
    @{ id = 'manufacturing'; text = 'Диаметр заготовки двадцать миллиметров, допуск пять сотых миллиметра.' },
    @{ id = 'download'; text = 'После проверки соединения загрузка продолжается с сохранённого места.' }
)
try {
    foreach ($phrase in $phrases) {
        $path = Join-Path $OutputDirectory ($phrase.id + '.wav')
        $speaker.SetOutputToWaveFile($path, $format)
        $speaker.Speak($phrase.text)
        $speaker.SetOutputToNull()
    }
} finally { $speaker.Dispose() }
if ($PrepareOnly) { Write-Output 'Synthetic Russian WAV fixtures prepared.'; return }
$results = @()
foreach ($model in @('gigaam-v3-e2e', 'nemotron-3-5-ru')) {
    foreach ($phrase in $phrases) {
        $path = Join-Path $OutputDirectory ($phrase.id + '.wav')
        $start = New-Object System.Diagnostics.ProcessStartInfo
        $start.FileName = $Probe
        $start.Arguments = 'transcribe "{0}" "{1}" "{2}"' -f $model, $ModelRoot, $path
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
        $start.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
        $process = [Diagnostics.Process]::Start($start)
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $peakBytes = 0L
        while (-not $process.HasExited) {
            try { $process.Refresh(); $peakBytes = [Math]::Max($peakBytes, $process.PeakWorkingSet64) } catch {}
            Start-Sleep -Milliseconds 20
        }
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw "Native check failed: $model / $($phrase.id): $($stderr.Result)" }
        $json = @($stdout.Result -split "`n" | Where-Object { $_.StartsWith('{') })[-1]
        $result = $json | ConvertFrom-Json
        $result | Add-Member -NotePropertyName peak_working_set_mb -NotePropertyValue ([Math]::Round($peakBytes / 1MB, 1))
        $result | Add-Member -NotePropertyName cpu_seconds -NotePropertyValue $process.TotalProcessorTime.TotalSeconds
        $process.Dispose()
        $results += @{ phrase = $phrase.id; reference = $phrase.text; result = $result }
        Write-Output "$model / $($phrase.id): $($result.text)"
    }
}
$report = @{ source = 'Local Windows SAPI, synthetic Russian speech; no microphone or human dictation'; cases = $results }
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'results.json'), ($report | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
