param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot "..\dist")
)

$ErrorActionPreference = "Stop"
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$manifestPath = Join-Path $repositoryRoot "Cargo.toml"
$binaryPath = Join-Path $repositoryRoot "target\release\codex-auth-switcher.exe"
$licenseSource = Join-Path $repositoryRoot "LICENSE"
$noticesSource = Join-Path $repositoryRoot "THIRD-PARTY-NOTICES.html"

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
$licensePath = Join-Path $outputPath "LICENSE"
$noticesPath = Join-Path $outputPath "THIRD-PARTY-NOTICES.html"
$sourcesPath = Join-Path $outputPath "SOURCES.txt"

foreach ($source in @($licenseSource, $noticesSource)) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "A required source notice is missing: $source"
    }
}
$outputFiles = @($releaseBinaryPath, $binaryChecksumPath, $licensePath, $noticesPath, $sourcesPath)
$existingOutput = @($outputFiles | Where-Object { Test-Path -LiteralPath $_ })
if ($existingOutput.Count -gt 0) {
    throw "The release output already exists. Choose another output directory or move the previous output."
}

$sourceCommit = $null
if (Get-Command git -ErrorAction SilentlyContinue) {
    $commitOutput = & git -C $repositoryRoot rev-parse --verify HEAD 2>$null
    if ($LASTEXITCODE -eq 0 -and $commitOutput.Count -gt 0) {
        $candidateCommit = ($commitOutput -join "").Trim()
        if ($candidateCommit -match "^[0-9a-f]{40}$") {
            $sourceCommit = $candidateCommit
        }
    }
}
$expectedTag = "v$($package.version)"
if ($null -ne $sourceCommit) {
    $sourceChanges = & git -C $repositoryRoot status --porcelain=v1 --untracked-files=all
    if ($LASTEXITCODE -ne 0) {
        throw "Source changes could not be checked."
    }
    if ($sourceChanges.Count -gt 0) {
        $sourceLabel = "Source baseline only: $($package.repository)/tree/$sourceCommit"
        $sourceState = "modified worktree or index; the HEAD URL is a baseline, not the exact build source"
        $lockfileSource = "Cargo.lock from the build working tree (may differ from HEAD)"
    } else {
        $sourceLabel = "Source code: $($package.repository)/tree/$sourceCommit"
        $sourceState = "clean worktree and index; the build corresponds to this commit"
        $lockfileSource = "Cargo.lock at the source commit"
    }
} else {
    $sourceLabel = "Source code: $($package.repository)"
    $sourceState = "unavailable (no Git HEAD)"
    $lockfileSource = "Cargo.lock from the selected repository revision"
}
$sourceLines = @($sourceLabel)
if ($null -ne $sourceCommit) {
    $sourceLines += "Source commit: $sourceCommit"
}
$sourceLines += @(
    "Build source state: $sourceState"
    "Expected version tag: $expectedTag (tag creation is not assumed)"
    "Build command: cargo build --release --locked"
    "Dependency lockfile: $lockfileSource"
)
$sourcesText = $sourceLines -join "`n"
$sourcesText += "`n"

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

try {
    Copy-Item -LiteralPath $binaryPath -Destination $releaseBinaryPath

    $binaryHash = (Get-FileHash -LiteralPath $releaseBinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $binaryChecksum = "$binaryHash  $binaryName"
    [System.IO.File]::WriteAllText($binaryChecksumPath, $binaryChecksum, [System.Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath $licenseSource -Destination $licensePath
    Copy-Item -LiteralPath $noticesSource -Destination $noticesPath
    [System.IO.File]::WriteAllText($sourcesPath, $sourcesText, [System.Text.UTF8Encoding]::new($false))

    [pscustomobject]@{
        Binary = $releaseBinaryPath
        BinarySha256 = $binaryHash
        License = $licensePath
        ThirdPartyNotices = $noticesPath
        Sources = $sourcesPath
        Target = $target
        Version = $package.version
    }
}
catch {
    foreach ($path in $outputFiles) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path
        }
    }
    throw
}
