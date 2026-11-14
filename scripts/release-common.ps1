function Get-VoiceInputVersion {
    param([Parameter(Mandatory)][string]$RepositoryRoot)
    $manifest = Get-Content -LiteralPath (Join-Path $RepositoryRoot 'src-tauri/Cargo.toml') -Raw
    $match = [regex]::Match($manifest, '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$')
    if (-not $match.Success) { throw 'Cargo.toml must contain a stable MAJOR.MINOR.PATCH package version' }
    return $match.Groups[1].Value
}

function Get-VoiceInputSigningDirectory {
    return Join-Path $env:LOCALAPPDATA 'VoiceInput/release-signing'
}

function Enable-VoiceInputSigning {
    $directory = Get-VoiceInputSigningDirectory
    $key = Join-Path $directory 'update.key'
    $passwordFile = Join-Path $directory 'password.dpapi'
    if (-not (Test-Path -LiteralPath $key) -or -not (Test-Path -LiteralPath $passwordFile)) {
        throw 'Signing key is missing. Run scripts/initialize-signing.ps1 once on this Windows account.'
    }
    Add-Type -AssemblyName System.Security
    $protected = [IO.File]::ReadAllBytes($passwordFile)
    $plain = [Security.Cryptography.ProtectedData]::Unprotect($protected, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    try { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [Text.Encoding]::UTF8.GetString($plain) } finally { [Array]::Clear($plain, 0, $plain.Length) }
    $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -LiteralPath $key -Raw).Trim()
}

function Write-VoiceInputJson {
    param([string]$Path, [object]$Value)
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 20) + "`n", [Text.UTF8Encoding]::new($false))
}
