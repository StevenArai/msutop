param(
    [string]$Version = '0.1.0',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$') {
    throw 'Version must be a semantic version'
}
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $projectRoot
try {
    if (-not $SkipBuild) {
        & (Join-Path $PSScriptRoot 'preview.ps1') -BuildOnly -Check
    }
    $stageParent = Join-Path $projectRoot 'output\release-package'
    New-Item -ItemType Directory -Path $stageParent -Force | Out-Null
    $stageRoot = Join-Path $stageParent ([Guid]::NewGuid().ToString('N'))
    $packageName = "msutop-v$Version-windows-x86_64"
    $packageRoot = Join-Path $stageRoot $packageName
    New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
    foreach ($name in @('msutop.exe','SDL2.dll')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "target\release\$name") -Destination (Join-Path $packageRoot $name)
    }
    foreach ($name in @('LICENSE','NOTICE.md')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination (Join-Path $packageRoot $name)
    }
    $docsRoot = Join-Path $packageRoot 'docs'
    New-Item -ItemType Directory -Path $docsRoot -Force | Out-Null
    foreach ($name in @('windows.md','rgb565.md','performance.md','pve-r-full-profile.json','pve-r-full.gif')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "docs\$name") -Destination (Join-Path $docsRoot $name)
    }
    $readme = "# msutop v$Version for Windows x86_64`n`nDouble-click ``00-PREVIEW.cmd`` or run ``.\msutop.exe`` after extracting the whole package.`n`nSee [Windows usage](docs/windows.md) for preview, serial output, and build instructions.`n`nKeep ``SDL2.dll`` beside ``msutop.exe``. Windows uses mock telemetry; Linux ``--live`` collects actual counters.`n`nProject: https://github.com/StevenArai/msutop`nLicense: MIT; third-party notices are in ``NOTICE.md`` and ``licenses/``.`n"
    [IO.File]::WriteAllText((Join-Path $packageRoot 'README.md'), $readme, [Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath (Join-Path $projectRoot 'licenses') -Destination (Join-Path $packageRoot 'licenses') -Recurse
    $launcher = "@echo off`r`npushd `"%~dp0`"`r`nmsutop.exe %*`r`nif errorlevel 1 pause`r`npopd`r`n"
    [IO.File]::WriteAllText((Join-Path $packageRoot '00-PREVIEW.cmd'), $launcher, [Text.Encoding]::ASCII)
    $hashLines = @('msutop.exe','SDL2.dll') | ForEach-Object {
        $hash = (Get-FileHash -LiteralPath (Join-Path $packageRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $_"
    }
    [IO.File]::WriteAllLines((Join-Path $packageRoot 'SHA256SUMS.txt'), $hashLines, [Text.Encoding]::ASCII)
    $distRoot = Join-Path $projectRoot 'dist'
    New-Item -ItemType Directory -Path $distRoot -Force | Out-Null
    $archive = Join-Path $distRoot "$packageName.zip"
    Compress-Archive -LiteralPath $packageRoot -DestinationPath $archive -Force
    $digest = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $distRoot "$packageName.zip.sha256"), "$digest  $packageName.zip`n", [Text.Encoding]::ASCII)
    Write-Output $archive
    Write-Output "SHA256: $digest"
} finally {
    Pop-Location
}
