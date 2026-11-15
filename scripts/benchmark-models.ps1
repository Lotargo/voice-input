param(
    [string]$ModelRoot = (Join-Path $env:LOCALAPPDATA 'dev.lotargo.voiceinput'),
    [string]$Probe,
    [string]$FixtureDirectory,
    [string]$OutputDirectory,
    [int[]]$Threads = @(1, 2, 4),
    [int]$Repeats = 2,
    [switch]$NoSpin
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Probe) { $Probe = Join-Path $repo 'src-tauri/target/debug/model-check.exe' }
if (-not $FixtureDirectory) { $FixtureDirectory = Join-Path $repo 'dist/qa/models/recognition' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repo 'dist/qa/models/cpu-benchmark' }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$fixtures = @('long-words', 'technical', 'manufacturing', 'download')
foreach ($fixture in $fixtures) {
    if (-not (Test-Path -LiteralPath (Join-Path $FixtureDirectory "$fixture.wav"))) { throw 'Prepare local SAPI fixtures with test-model-recognition.ps1 -PrepareOnly first.' }
}
$results = @()
for ($round = 0; $round -lt $Repeats; $round++) {
    # Alternate the order to limit warm-cache and temperature bias. Never run inference in parallel.
    $order = @($Threads)
    if ($round % 2) { [array]::Reverse($order) }
    foreach ($model in @('gigaam-v3-e2e', 'nemotron-3-5-ru')) {
        foreach ($count in $order) {
            foreach ($fixture in $fixtures) {
                $start = [Diagnostics.ProcessStartInfo]::new()
                $start.FileName = $Probe
                $start.Arguments = 'transcribe "{0}" "{1}" "{2}" {3}' -f $model, $ModelRoot, (Join-Path $FixtureDirectory "$fixture.wav"), $count
                if ($NoSpin) { $start.Arguments += ' no-spin' }
                $start.UseShellExecute = $false; $start.CreateNoWindow = $true
                $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true
                $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
                $start.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
                $process = [Diagnostics.Process]::Start($start)
                $stdout = $process.StandardOutput.ReadToEndAsync(); $stderr = $process.StandardError.ReadToEndAsync()
                $peakBytes = 0L
                while (-not $process.HasExited) {
                    try { $process.Refresh(); $peakBytes = [Math]::Max($peakBytes, $process.PeakWorkingSet64) } catch {}
                    Start-Sleep -Milliseconds 20
                }
                $process.WaitForExit()
                if ($process.ExitCode -ne 0) { throw "Native benchmark failed: $model / $count / $fixture : $($stderr.Result)" }
                $json = @($stdout.Result -split "`n" | Where-Object { $_.StartsWith('{') })[-1]
                $result = $json | ConvertFrom-Json
                $result | Add-Member -NotePropertyName fixture -NotePropertyValue $fixture
                $result | Add-Member -NotePropertyName round -NotePropertyValue $round
                $result | Add-Member -NotePropertyName cpu_seconds -NotePropertyValue $process.TotalProcessorTime.TotalSeconds
                $result | Add-Member -NotePropertyName peak_working_set_mib -NotePropertyValue ([Math]::Round($peakBytes / 1MB, 1))
                $process.Dispose(); $results += $result
            }
            Write-Output "$model : $count threads, round $($round + 1) complete"
            [IO.File]::WriteAllText((Join-Path $OutputDirectory 'results.json'), (ConvertTo-Json -InputObject @($results) -Depth 8), [Text.UTF8Encoding]::new($false))
        }
    }
}
$results | Group-Object model, threads | ForEach-Object {
    $group = $_.Group
    [pscustomobject]@{ Model = $group[0].model; Threads = $group[0].threads; DecodeSeconds = [Math]::Round(($group | Measure-Object decode_seconds -Average).Average, 3); LoadSeconds = [Math]::Round(($group | Measure-Object load_seconds -Average).Average, 3); CpuSeconds = [Math]::Round(($group | Measure-Object cpu_seconds -Average).Average, 3) }
}
