$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'release-common.ps1')
$repo = Split-Path -Parent $PSScriptRoot
$configuration = Join-Path $repo 'src-tauri/tauri.conf.json'
$config = Get-Content -LiteralPath $configuration -Raw | ConvertFrom-Json -AsHashtable
$directory = Get-VoiceInputSigningDirectory
New-Item -ItemType Directory -Path $directory -Force | Out-Null
$key = Join-Path $directory 'update.key'
$passwordFile = Join-Path $directory 'password.dpapi'
if (-not (Test-Path -LiteralPath $key)) {
    if ($config.ContainsKey('plugins') -and $config.plugins.ContainsKey('updater') -and $config.plugins.updater.pubkey) {
        throw 'A public update key is already configured. Restore the existing private key; generating another key would break updates for released apps.'
    }
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
    $acl.AddAccessRule($rule)
    Set-Acl -LiteralPath $directory -AclObject $acl
    $random = [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)
    $password = [Convert]::ToBase64String($random)
    $plain = [Text.Encoding]::UTF8.GetBytes($password)
    Add-Type -AssemblyName System.Security
    try {
        $protected = [Security.Cryptography.ProtectedData]::Protect($plain, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
        [IO.File]::WriteAllBytes($passwordFile, $protected)
        $output = & cargo tauri signer generate --ci --write-keys $key --password $password 2>&1
        if ($LASTEXITCODE -ne 0) { throw 'Tauri signing key generation failed' }
    } finally { [Array]::Clear($plain, 0, $plain.Length); $password = $null }
}
if (-not (Test-Path -LiteralPath $passwordFile)) { throw 'The existing key has no DPAPI password; recover it instead of replacing the key.' }
$publicKey = (Get-Content -LiteralPath "$key.pub" -Raw).Trim()
if ($config.ContainsKey('plugins') -and $config.plugins.ContainsKey('updater') -and $config.plugins.updater.pubkey -and $config.plugins.updater.pubkey -ne $publicKey) {
    throw 'The local key does not match the configured public key. Restore the correct release key instead of rotating it silently.'
}
if (-not $config.ContainsKey('plugins')) { $config.plugins = @{} }
$config.plugins.updater = @{
    pubkey = $publicKey
    requireSignedVersion = $true
    windows = @{ installMode = 'passive' }
}
[IO.File]::WriteAllText($configuration, ($config | ConvertTo-Json -Depth 30) + "`n", [Text.UTF8Encoding]::new($false))
Write-Output 'Update signing configured. Only the public key was written into the repository.'
Write-Output "Private signing material: $directory"
