param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot "..\dist")
)

$ErrorActionPreference = "Stop"
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$manifestPath = Join-Path $repositoryRoot "Cargo.toml"
$guidePath = Join-Path $repositoryRoot "docs\06-v2-user-guide.md"
$binaryPath = Join-Path $repositoryRoot "target\release\codex-auth-switcher.exe"

if (-not (Test-Path -LiteralPath $guidePath -PathType Leaf)) {
    throw "The V2 user guide was not found."
}

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
$archiveName = "codex-auth-switcher-v$($package.version)-windows-$architecture.zip"
$archivePath = Join-Path $outputPath $archiveName
$checksumPath = "$archivePath.sha256"
$binaryName = "codex-auth-switcher-v$($package.version)-windows-$architecture.exe"
$releaseBinaryPath = Join-Path $outputPath $binaryName
$binaryChecksumPath = "$releaseBinaryPath.sha256"
if ((Test-Path -LiteralPath $archivePath) -or
    (Test-Path -LiteralPath $checksumPath) -or
    (Test-Path -LiteralPath $releaseBinaryPath) -or
    (Test-Path -LiteralPath $binaryChecksumPath)) {
    throw "The package output already exists. Choose another output directory or move the previous package."
}

$stagePath = Join-Path $outputPath ".staging-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $stagePath | Out-Null
try {
    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $stagePath "codex-auth-switcher.exe")
    Copy-Item -LiteralPath $guidePath -Destination (Join-Path $stagePath "README.md")
    Copy-Item -LiteralPath $binaryPath -Destination $releaseBinaryPath
    Compress-Archive -Path (Join-Path $stagePath "*") -DestinationPath $archivePath -CompressionLevel Optimal

    $hash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksum = "$hash  $archiveName"
    [System.IO.File]::WriteAllText($checksumPath, $checksum, [System.Text.UTF8Encoding]::new($false))
    $binaryHash = (Get-FileHash -LiteralPath $releaseBinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $binaryChecksum = "$binaryHash  $binaryName"
    [System.IO.File]::WriteAllText($binaryChecksumPath, $binaryChecksum, [System.Text.UTF8Encoding]::new($false))

    [pscustomobject]@{
        Archive = $archivePath
        Sha256 = $hash
        Binary = $releaseBinaryPath
        BinarySha256 = $binaryHash
        Target = $target
        Version = $package.version
    }
}
catch {
    if (Test-Path -LiteralPath $archivePath) {
        Remove-Item -LiteralPath $archivePath
    }
    if (Test-Path -LiteralPath $checksumPath) {
        Remove-Item -LiteralPath $checksumPath
    }
    if (Test-Path -LiteralPath $releaseBinaryPath) {
        Remove-Item -LiteralPath $releaseBinaryPath
    }
    if (Test-Path -LiteralPath $binaryChecksumPath) {
        Remove-Item -LiteralPath $binaryChecksumPath
    }
    throw
}
finally {
    $stageFullPath = [System.IO.Path]::GetFullPath($stagePath)
    $outputPrefix = $outputPath.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    if (-not $stageFullPath.StartsWith($outputPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "The staging directory escaped the package output directory."
    }
    Remove-Item -LiteralPath $stageFullPath -Recurse -Force
}
