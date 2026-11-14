function Assert-WindowsExecutableIcon {
    param(
        [Parameter(Mandatory)][string]$ExecutablePath,
        [Parameter(Mandatory)][string]$IconPath
    )

    # Inspect the actual PE icon, not the independent webview/tray asset.
    Add-Type -AssemblyName System.Drawing
    $embeddedIcon = $null
    $expectedIcon = $null
    $embeddedBitmap = $null
    $expectedBitmap = $null
    try {
        $embeddedIcon = [Drawing.Icon]::ExtractAssociatedIcon($ExecutablePath)
        if (-not $embeddedIcon) { throw "В EXE отсутствует иконка Windows: $ExecutablePath" }
        $expectedIcon = [Drawing.Icon]::new($IconPath, $embeddedIcon.Size)
        $embeddedBitmap = $embeddedIcon.ToBitmap()
        $expectedBitmap = $expectedIcon.ToBitmap()
        for ($iconY = 0; $iconY -lt $embeddedBitmap.Height; $iconY++) {
            for ($iconX = 0; $iconX -lt $embeddedBitmap.Width; $iconX++) {
                if ($embeddedBitmap.GetPixel($iconX, $iconY).ToArgb() -ne $expectedBitmap.GetPixel($iconX, $iconY).ToArgb()) {
                    throw "Иконка в EXE не совпадает с исходным ICO: $ExecutablePath. Пересоберите ресурсы Windows."
                }
            }
        }
        Write-Host "Иконка Windows проверена: $(Split-Path -Leaf $ExecutablePath)"
    } finally {
        if ($embeddedBitmap) { $embeddedBitmap.Dispose() }
        if ($expectedBitmap) { $expectedBitmap.Dispose() }
        if ($expectedIcon) { $expectedIcon.Dispose() }
        if ($embeddedIcon) { $embeddedIcon.Dispose() }
    }
}
