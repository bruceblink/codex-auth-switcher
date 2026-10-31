param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot "..\dist")
)

$ErrorActionPreference = "Stop"
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$manifestPath = Join-Path $repositoryRoot "Cargo.toml"
$binaryPath = Join-Path $repositoryRoot "target\release\codex-auth-switcher.exe"

$compiler = & rustc -vV
if ($LASTEXITCODE -ne 0) {
    throw "The Rust target could not be read."
}
$hostLine = $compiler | Where-Object { $_ -match "^host: " } | Select-Object -First 1
$target = $hostLine -replace "^host: ", ""
if ($target -notmatch "^(x86_64|aarch64)-pc-windows-msvc$") {
    throw "Packaging requires a Windows MSVC target; found '$target'."
}
$rustflagsConfig = "target.$target.rustflags=['-C','target-feature=+crt-static']"

Push-Location $repositoryRoot
try {
    & cargo build --release --locked --manifest-path $manifestPath --config $rustflagsConfig
    if ($LASTEXITCODE -ne 0) {
        throw "The Windows release build failed."
    }
}
finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "The release executable was not produced."
}

$metadataJson = & cargo metadata --no-deps --format-version 1 --manifest-path $manifestPath
if ($LASTEXITCODE -ne 0) {
    throw "Cargo metadata could not be read."
}
$metadata = $metadataJson | ConvertFrom-Json
$package = $metadata.packages | Where-Object { $_.name -eq "switcher-desktop" } | Select-Object -First 1
if ($null -eq $package) {
    throw "The desktop package version could not be read."
}

$architecture = if ($target.StartsWith("x86_64-")) { "x64" } else { "arm64" }

$outputPath = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
$binaryName = "codex-auth-switcher-v$($package.version)-windows-$architecture.exe"
$releaseBinaryPath = Join-Path $outputPath $binaryName
$binaryChecksumPath = "$releaseBinaryPath.sha256"
if ((Test-Path -LiteralPath $releaseBinaryPath) -or
    (Test-Path -LiteralPath $binaryChecksumPath)) {
    throw "The release output already exists. Choose another output directory or move the previous output."
}

try {
    Copy-Item -LiteralPath $binaryPath -Destination $releaseBinaryPath

    $binaryHash = (Get-FileHash -LiteralPath $releaseBinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $binaryChecksum = "$binaryHash  $binaryName"
    [System.IO.File]::WriteAllText($binaryChecksumPath, $binaryChecksum, [System.Text.UTF8Encoding]::new($false))

    [pscustomobject]@{
        Binary = $releaseBinaryPath
        BinarySha256 = $binaryHash
        Target = $target
        Version = $package.version
    }
}
catch {
    if (Test-Path -LiteralPath $releaseBinaryPath) {
        Remove-Item -LiteralPath $releaseBinaryPath
    }
    if (Test-Path -LiteralPath $binaryChecksumPath) {
        Remove-Item -LiteralPath $binaryChecksumPath
    }
    throw
}
