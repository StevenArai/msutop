param(
    [ValidateSet('balanced','idle','cpu','io','memory','offline')][string]$Scene = 'balanced',
    [ValidateSet('auto','down','nocarrier','active','transfer')][string]$Network = 'auto',
    [ValidateSet('default','vivid')][string]$Theme = 'default',
    [ValidateRange(1,16)][int]$Scale = 6,
    [ValidateRange(1,60)][int]$Fps = 30,
    [ValidateRange(1,512)][int]$MockThreads = 8,
    [switch]$NoAA,
    [switch]$BuildOnly,
    [switch]$Check
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$toolchain = '1.88.0-x86_64-pc-windows-gnu'
$originalPath = $env:PATH
$originalLinker = $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER
$originalTarget = $env:CARGO_TARGET_DIR
Push-Location $projectRoot
try {
    $installed = & rustup toolchain list
    if (-not ($installed -match [regex]::Escape($toolchain))) {
        & rustup toolchain install $toolchain --profile minimal --component rustfmt --component clippy --no-self-update
        if ($LASTEXITCODE -ne 0) { throw 'GNU Rust toolchain installation failed' }
    }
    $gnuRoot = (& rustup run $toolchain rustc --print sysroot).Trim()
    $gnuBin = Join-Path $gnuRoot 'lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained'
    $env:PATH = "$gnuBin;$originalPath"
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = Join-Path $gnuBin 'x86_64-w64-mingw32-gcc.exe'
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target\gnu'
    $depsRoot = Join-Path $projectRoot '.deps'
    $sdlRoot = Join-Path $depsRoot 'SDL2-2.32.10\x86_64-w64-mingw32'
    if (-not (Test-Path -LiteralPath (Join-Path $sdlRoot 'lib\libSDL2.dll.a'))) {
        New-Item -ItemType Directory -Path $depsRoot -Force | Out-Null
        $archive = Join-Path $depsRoot 'SDL2-devel-2.32.10-mingw.zip'
        Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/libsdl-org/SDL/releases/download/release-2.32.10/SDL2-devel-2.32.10-mingw.zip' -OutFile $archive
        $digest = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($digest -ne 'f15cff5fca62ec9381a016ef1d42a95c638cd72d2f226ba5781c76fe43dbd1ac') {
            throw 'SDL2 archive checksum mismatch'
        }
        Expand-Archive -LiteralPath $archive -DestinationPath $depsRoot -Force
    }
    if ($Check) {
        & cargo "+$toolchain" test --all-targets --features sdl,serial --locked
        if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
        & cargo "+$toolchain" clippy --all-targets --features sdl,serial --locked -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
        & cargo "+$toolchain" fmt --all -- --check
        if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed' }
    }
    & cargo "+$toolchain" build --release --features sdl,serial --locked
    if ($LASTEXITCODE -ne 0) { throw 'GNU Rust build failed' }
    $releaseRoot = Join-Path $env:CARGO_TARGET_DIR 'release'
    Copy-Item -LiteralPath (Join-Path $sdlRoot 'bin\SDL2.dll') -Destination (Join-Path $releaseRoot 'SDL2.dll')
    $runRoot = Join-Path $projectRoot 'target\release'
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $releaseRoot 'msutop.exe') -Destination (Join-Path $runRoot 'msutop.exe')
    Copy-Item -LiteralPath (Join-Path $releaseRoot 'SDL2.dll') -Destination (Join-Path $runRoot 'SDL2.dll')
    Write-Output "SDL preview: $(Join-Path $runRoot 'msutop.exe')"
    if (-not $BuildOnly) {
        $previewArguments = @('--scene', $Scene, '--theme', $Theme, '--scale', $Scale, '--fps', $Fps, '--mock-threads', $MockThreads)
        if ($Network -ne 'auto') { $previewArguments += @('--network', $Network) }
        if ($NoAA) { $previewArguments += '--no-aa' }
        & (Join-Path $runRoot 'msutop.exe') @previewArguments
        if ($LASTEXITCODE -ne 0) { throw 'SDL preview failed' }
    }
} finally {
    $env:PATH = $originalPath
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $originalLinker
    $env:CARGO_TARGET_DIR = $originalTarget
    Pop-Location
}
