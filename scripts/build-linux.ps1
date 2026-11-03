param([ValidateSet('x86_64','aarch64')][string]$Architecture = 'x86_64')
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$toolchain = '1.88.0-x86_64-pc-windows-gnu'
$target = "$Architecture-unknown-linux-musl"
$originalPath = $env:PATH
$originalLinker = $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER
$originalTarget = $env:CARGO_TARGET_DIR
$targetKey = "CARGO_TARGET_$($target.ToUpperInvariant().Replace('-','_'))"
$linkerKey = "${targetKey}_LINKER"
$flagsKey = "${targetKey}_RUSTFLAGS"
$originalLinuxLinker = [Environment]::GetEnvironmentVariable($linkerKey, 'Process')
$originalLinuxFlags = [Environment]::GetEnvironmentVariable($flagsKey, 'Process')
Push-Location $projectRoot
try {
    & rustup toolchain install $toolchain --profile minimal --no-self-update
    if ($LASTEXITCODE -ne 0) { throw 'GNU Rust installation failed' }
    & rustup target add $target --toolchain $toolchain
    if ($LASTEXITCODE -ne 0) { throw 'Linux target installation failed' }
    $gnuRoot = (& rustup run $toolchain rustc --print sysroot).Trim()
    $gnuBin = Join-Path $gnuRoot 'lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained'
    $env:PATH = "$gnuBin;$originalPath"
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = Join-Path $gnuBin 'x86_64-w64-mingw32-gcc.exe'
    [Environment]::SetEnvironmentVariable($linkerKey, (Join-Path $gnuRoot 'lib\rustlib\x86_64-pc-windows-gnu\bin\rust-lld.exe'), 'Process')
    [Environment]::SetEnvironmentVariable($flagsKey, '-C linker-flavor=ld.lld -C link-self-contained=yes', 'Process')
    $env:CARGO_TARGET_DIR = Join-Path $projectRoot 'target\gnu'
    & cargo "+$toolchain" build --release --target $target --no-default-features --features serial --locked
    if ($LASTEXITCODE -ne 0) { throw 'Linux cross build failed' }
    Write-Output (Join-Path $env:CARGO_TARGET_DIR "$target\release\msutop")
} finally {
    $env:PATH = $originalPath
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $originalLinker
    $env:CARGO_TARGET_DIR = $originalTarget
    [Environment]::SetEnvironmentVariable($linkerKey, $originalLinuxLinker, 'Process')
    [Environment]::SetEnvironmentVariable($flagsKey, $originalLinuxFlags, 'Process')
    Pop-Location
}
