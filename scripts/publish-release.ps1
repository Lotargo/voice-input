param([string]$OutputDirectory, [switch]$Draft)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'release-common.ps1')
$repo = Split-Path -Parent $PSScriptRoot
$version = Get-VoiceInputVersion $repo
$tag = "v$version"
$out = if ($OutputDirectory) { [IO.Path]::GetFullPath($OutputDirectory) } else { Join-Path $repo 'dist' }
$metadata = Join-Path $out "releases/$version"
$notes = Join-Path $repo "releases/$version.md"
$head = (& git -C $repo rev-parse HEAD).Trim()
if (@(& git -C $repo status --porcelain).Count -ne 0) { throw 'Commit source changes before publishing a release.' }
$build = Get-Content -LiteralPath (Join-Path $metadata 'build.json') -Raw | ConvertFrom-Json
if ($build.version -ne $version -or $build.source_commit -ne $head) { throw 'Build packages again from the current source commit.' }
$assets = @(
    (Join-Path $out "portable/$version/Voice-Input-Portable-windows-x64.zip"),
    (Join-Path $out "portable/$version/Voice-Input-Portable-windows-x64.zip.sig"),
    (Join-Path $out "installers/$version/Voice-Input-Setup-windows-x64.exe"),
    (Join-Path $out "installers/$version/Voice-Input-Setup-windows-x64.exe.sig"),
    (Join-Path $metadata 'latest.json'), (Join-Path $metadata 'latest-portable.json'),
    (Join-Path $metadata 'SHA256SUMS.txt')
)
$byName = @{}
foreach ($asset in $assets) {
    if (-not (Test-Path -LiteralPath $asset -PathType Leaf)) { throw "Missing release file: $asset" }
    $byName[[IO.Path]::GetFileName($asset)] = $asset
}
foreach ($line in Get-Content -LiteralPath (Join-Path $metadata 'SHA256SUMS.txt')) {
    if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw 'Invalid SHA256SUMS.txt' }
    if (-not $byName.ContainsKey($Matches[2]) -or (Get-FileHash -LiteralPath $byName[$Matches[2]] -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Matches[1]) { throw 'A release file changed after packaging.' }
}
$visibility = & gh repo view Lotargo/voice-input --json visibility --jq '.visibility'
if ($LASTEXITCODE -ne 0 -or $visibility -ne 'PUBLIC') { throw 'The release repository must be public for token-free updates.' }
$existingTag = & git -C $repo rev-parse -q --verify "refs/tags/$tag" 2>$null
if ($LASTEXITCODE -eq 0 -and $existingTag.Trim() -ne $head) { throw 'The version tag already points to a different commit.' }
if (-not $existingTag) { & git -C $repo tag $tag $head; if ($LASTEXITCODE -ne 0) { throw 'Tag creation failed.' } }
& git -C $repo push origin main "refs/tags/$tag"
if ($LASTEXITCODE -ne 0) { throw 'Source/tag push failed.' }
$existing = & gh release view $tag --repo Lotargo/voice-input --json isDraft --jq '.isDraft' 2>$null
if ($LASTEXITCODE -eq 0) {
    if ($existing -ne 'true') { throw 'This version is already published; bump the version instead of replacing it.' }
} else {
    & gh release create $tag --repo Lotargo/voice-input --draft --verify-tag --title "Voice Input $version" --notes-file $notes
    if ($LASTEXITCODE -ne 0) { throw 'Draft release creation failed.' }
}
& gh release upload $tag @assets --repo Lotargo/voice-input --clobber
if ($LASTEXITCODE -ne 0) { throw 'Release upload failed. The release remains a draft.' }
if (-not $Draft) {
    & gh release edit $tag --repo Lotargo/voice-input --draft=false --latest
    if ($LASTEXITCODE -ne 0) { throw 'Release publication failed.' }
}
Write-Output "https://github.com/Lotargo/voice-input/releases/tag/$tag"
