param([string]$HelperExecutable)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'release-common.ps1')
$repo = Split-Path -Parent $PSScriptRoot
if (-not $HelperExecutable) { $HelperExecutable = Join-Path $repo 'src-tauri/target/debug/voice-input.exe' }
$HelperExecutable = (Resolve-Path -LiteralPath $HelperExecutable).Path
$version = Get-VoiceInputVersion $repo
$parts = $version.Split('.')
$nextVersion = '{0}.{1}.{2}' -f $parts[0], $parts[1], (1 + [int]$parts[2])
$work = Join-Path $repo ('dist/qa/portable-updates/run-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work -Force | Out-Null
$source = @'
#![windows_subsystem = "windows"]
use std::{env, fs, path::PathBuf, thread, time::{Duration, Instant}};
fn main() {
    let args: Vec<_> = env::args().collect();
    let root = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    if args.get(1).map(String::as_str) == Some("wait") {
        let work = PathBuf::from(&args[2]);
        fs::write(work.join("parent-started"), b"started").unwrap();
        let start = Instant::now();
        while !work.join("ready").exists() && !work.join("stop-fixture").exists() && start.elapsed() < Duration::from_secs(90) { thread::sleep(Duration::from_millis(30)); }
    } else { fs::write(root.join("restarted.txt"), b"FIXTURE_VERSION").unwrap(); }
}
'@
foreach ($kind in @('old', 'new')) {
    $path = Join-Path $work "$kind.rs"
    [IO.File]::WriteAllText($path, $source.Replace('FIXTURE_VERSION', $kind), [Text.UTF8Encoding]::new($false))
    & rustc --edition 2021 -O $path -o (Join-Path $work "$kind.exe")
    if ($LASTEXITCODE -ne 0) { throw 'Fixture compilation failed.' }
}
$packageRoot = Join-Path $work 'package/Voice-Input'
New-Item -ItemType Directory -Path (Join-Path $packageRoot 'models/fixture/am') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $work 'new.exe') -Destination (Join-Path $packageRoot 'voice-input.exe')
$dlls = @('libvosk.dll', 'libgcc_s_seh-1.dll', 'libstdc++-6.dll', 'libwinpthread-1.dll')
foreach ($dll in $dlls) { [IO.File]::WriteAllText((Join-Path $packageRoot $dll), "new $dll") }
[IO.File]::WriteAllText((Join-Path $packageRoot 'models/fixture/am/final.mdl'), 'fixture only')
Write-VoiceInputJson (Join-Path $packageRoot 'portable.json') @{ app_id = 'dev.lotargo.voiceinput'; kind = 'portable'; version = $nextVersion }
$package = Join-Path $work 'package.zip'
Compress-Archive -LiteralPath $packageRoot -DestinationPath $package
$oldKey = $env:TAURI_SIGNING_PRIVATE_KEY
$oldPassword = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
try {
    Enable-VoiceInputSigning
    $signOutput = & cargo tauri signer sign --app-version $nextVersion $package 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Fixture signing failed: $signOutput" }
} finally { $env:TAURI_SIGNING_PRIVATE_KEY = $oldKey; $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $oldPassword }
$signature = (Get-Content -LiteralPath "$package.sig" -Raw).Trim()
$results = @()
foreach ($testCase in @('success', 'tamper', 'rollback')) {
    $case = Join-Path $work $testCase
    $target = Join-Path $case 'target'
    $handoff = Join-Path $case 'handoff'
    New-Item -ItemType Directory -Path $target, $handoff -Force | Out-Null
    $executable = Join-Path $target 'voice-input.exe'
    Copy-Item -LiteralPath (Join-Path $work 'old.exe') -Destination $executable
    foreach ($dll in $dlls) { [IO.File]::WriteAllText((Join-Path $target $dll), "old $dll") }
    [IO.File]::WriteAllText((Join-Path $target 'personal.txt'), 'preserve this')
    Write-VoiceInputJson (Join-Path $target 'portable.json') @{ app_id = 'dev.lotargo.voiceinput'; kind = 'portable'; version = $version }
    Copy-Item -LiteralPath $package -Destination (Join-Path $handoff 'package.zip')
    if ($testCase -eq 'tamper') {
        $tampered = [IO.File]::ReadAllBytes((Join-Path $handoff 'package.zip'))
        $tampered[50] = $tampered[50] -bxor 1
        [IO.File]::WriteAllBytes((Join-Path $handoff 'package.zip'), $tampered)
    }
    Copy-Item -LiteralPath $HelperExecutable -Destination (Join-Path $handoff 'update-helper.exe')
    $parent = Start-Process -FilePath $executable -ArgumentList @('wait', ('"' + $handoff + '"')) -WorkingDirectory $target -WindowStyle Hidden -PassThru
    $helper = $null
    $lockedFile = $null
    try {
        Write-VoiceInputJson (Join-Path $handoff 'plan.json') @{ app_id = 'dev.lotargo.voiceinput'; parent = $parent.Id; executable = $executable; version = $nextVersion; signature = $signature }
        if ($testCase -eq 'rollback') {
            $lockedFile = [IO.File]::Open((Join-Path $target 'libwinpthread-1.dll'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        }
        $helper = Start-Process -FilePath (Join-Path $handoff 'update-helper.exe') -ArgumentList '--apply-portable-update' -WorkingDirectory $handoff -WindowStyle Hidden -PassThru
        if (-not $helper.WaitForExit(45000)) { throw "Helper timed out: $testCase" }
        if ($testCase -eq 'tamper') {
            if ($helper.ExitCode -eq 0 -or (Test-Path -LiteralPath (Join-Path $handoff 'ready'))) { throw 'A tampered package was accepted.' }
            [IO.File]::WriteAllText((Join-Path $handoff 'stop-fixture'), 'stop')
            if ((Get-FileHash -LiteralPath $executable).Hash -ne (Get-FileHash -LiteralPath (Join-Path $work 'old.exe')).Hash) { throw 'Tampered update changed the executable.' }
        } else {
            $expected = if ($testCase -eq 'success') { 'new' } else { 'old' }
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            while (-not (Test-Path -LiteralPath (Join-Path $target 'restarted.txt')) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
            if ((Get-Content -LiteralPath (Join-Path $target 'restarted.txt') -Raw) -ne $expected) { throw "Wrong restarted executable: $testCase" }
            if ($testCase -eq 'success' -and $helper.ExitCode -ne 0) { throw 'Successful update returned an error.' }
            if ($testCase -eq 'rollback' -and $helper.ExitCode -eq 0) { throw 'A locked destination did not trigger rollback.' }
            foreach ($dll in $dlls) {
                if ((Get-Content -LiteralPath (Join-Path $target $dll) -Raw) -ne "$expected $dll") { throw "Wrong DLL after ${testCase}: $dll" }
            }
        }
        if ((Get-Content -LiteralPath (Join-Path $target 'personal.txt') -Raw) -ne 'preserve this') { throw 'An unrelated file was changed.' }
        $results += @{ case = $testCase; passed = $true; helper_exit = $helper.ExitCode }
        Write-Output "Portable helper: $testCase passed."
    } finally {
        if ($lockedFile) { $lockedFile.Dispose() }
        [IO.File]::WriteAllText((Join-Path $handoff 'stop-fixture'), 'stop')
        if (-not $parent.WaitForExit(3000)) { $parent.Kill(); $parent.WaitForExit() }
        if ($helper -and -not $helper.HasExited) { $helper.Kill(); $helper.WaitForExit() }
    }
}
Write-VoiceInputJson (Join-Path $work 'results.json') $results
Write-Output "Evidence: $work"
