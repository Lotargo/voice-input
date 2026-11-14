function Remove-BuildDirectory {
    param(
        [Parameter(Mandatory)][string]$LiteralPath,
        [Parameter(Mandatory)][string]$Within
    )

    $rootPath = [IO.Path]::GetFullPath($Within).TrimEnd('\', '/')
    $targetPath = [IO.Path]::GetFullPath($LiteralPath).TrimEnd('\', '/')
    $prefix = $rootPath + [IO.Path]::DirectorySeparatorChar
    if (-not $targetPath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove directory outside ${rootPath}: $targetPath"
    }
    if (Test-Path -LiteralPath $targetPath) {
        Remove-Item -LiteralPath $targetPath -Recurse -Force
    }
}
