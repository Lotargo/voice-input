param([Parameter(Mandatory)][ValidatePattern('^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$')][string]$Version)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$path = Join-Path $repo 'src-tauri/Cargo.toml'
$manifest = Get-Content -LiteralPath $path -Raw
$manifest = [regex]::new('(?m)^version\s*=\s*"[^"]+"\s*$').Replace($manifest, "version = `"$Version`"", 1)
[IO.File]::WriteAllText($path, $manifest, [Text.UTF8Encoding]::new($false))
Push-Location (Join-Path $repo 'src-tauri')
try { & cargo update --workspace --offline; if ($LASTEXITCODE -ne 0) { throw 'Unable to synchronize Cargo.lock' } } finally { Pop-Location }
Write-Output "Voice Input version: $Version. Add releases/$Version.md before publishing."
