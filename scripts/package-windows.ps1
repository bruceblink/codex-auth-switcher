[CmdletBinding()]
param(
    [switch]$SmokeTest,
    [string]$OutputDirectory = "target\windows-dist",
    [string]$InnoSetupDirectory = "target\build-tools\innosetup-6.7.4"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$manifestPath = Join-Path $repositoryRoot "Cargo.toml"
$binaryPath = Join-Path $repositoryRoot "target\release\codex-auth-switcher.exe"
$innoScript = Join-Path $PSScriptRoot "windows\CodexAuthSwitcher.iss"
$innoPreparationScript = Join-Path $PSScriptRoot "windows\prepare-innosetup.ps1"
$smokeTestScript = Join-Path $PSScriptRoot "windows\test-installer.ps1"
$licenseSource = Join-Path $repositoryRoot "LICENSE"
$noticesSource = Join-Path $repositoryRoot "THIRD-PARTY-NOTICES.html"
$translationLicenseSource = Join-Path $PSScriptRoot "windows\languages\LICENSE.txt"
$targetDirectory = Join-Path $repositoryRoot "target"
$outputPath = if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
}
else {
    [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $OutputDirectory))
}
$innoToolsDirectory = if ([System.IO.Path]::IsPathRooted($InnoSetupDirectory)) {
    [System.IO.Path]::GetFullPath($InnoSetupDirectory)
}
else {
    [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $InnoSetupDirectory))
}

function Get-DesktopMetadata {
    $metadataJson = (& cargo metadata --locked --no-deps --format-version 1 --manifest-path $manifestPath) -join "`n"
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo metadata could not be read."
    }

    try {
        $metadata = $metadataJson | ConvertFrom-Json
    }
    catch {
        throw "Cargo metadata is invalid: $($_.Exception.Message)"
    }

    $packages = @($metadata.packages | Where-Object { $_.name -eq "switcher-desktop" })
    if ($packages.Count -ne 1) {
        throw "Cargo metadata must contain exactly one switcher-desktop package."
    }

    $version = [string]$packages[0].version
    if ($version -notmatch '^(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)$') {
        throw "Windows packaging requires a stable three-part Cargo version: $version"
    }
    foreach ($part in @($Matches.major, $Matches.minor, $Matches.patch)) {
        if ([uint32]$part -gt 65535) {
            throw "Windows file version component exceeds 65535: $version"
        }
    }
    if ([string]$packages[0].repository -cne "https://github.com/bruceblink/codex-auth-switcher") {
        throw "Cargo repository metadata does not match the Codex Auth Switcher release repository."
    }

    return [PSCustomObject]@{
        Version = $version
        FileVersion = "$version.0"
        Repository = [string]$packages[0].repository
    }
}

function Get-SourceCommit {
    $commit = & git -C $repositoryRoot rev-parse --verify HEAD 2>$null
    if ($LASTEXITCODE -ne 0) {
        return $null
    }
    $commit = ($commit -join "").Trim()
    if ($commit -notmatch "^[0-9a-f]{40}$") {
        return $null
    }
    return $commit
}

function Test-RustBuildInputsClean {
    $status = & git -C $repositoryRoot status --porcelain=v1 --untracked-files=all -- `
        Cargo.toml Cargo.lock rust-toolchain.toml crates
    if ($LASTEXITCODE -ne 0) {
        throw "Rust build input changes could not be checked."
    }
    return @($status).Count -eq 0
}

function Get-VersionInfoNumber {
    param([Parameter(Mandatory = $true)][string]$Version)
    return "$Version.0"
}

function Assert-TagMatchesVersion {
    param([Parameter(Mandatory = $true)][string]$Version)

    $tag = $null
    if ($env:GITHUB_REF_TYPE -eq "tag") {
        $tag = $env:GITHUB_REF_NAME
    }
    elseif ($env:GITHUB_REF -like "refs/tags/*") {
        $tag = $env:GITHUB_REF.Substring("refs/tags/".Length)
    }
    if ($null -ne $tag -and $tag -cne "v$Version") {
        throw "Release tag $tag does not match Cargo version v$Version."
    }
}

function Assert-File {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Description
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Description is missing: $Path"
    }
    if ((Get-Item -LiteralPath $Path).Length -eq 0) {
        throw "$Description is empty: $Path"
    }
}

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw "Windows setup packages must be built on Windows."
}
if (-not (Test-Path -LiteralPath $innoScript -PathType Leaf)) {
    throw "Inno Setup source is missing: $innoScript"
}
if ($SmokeTest -and -not (Test-Path -LiteralPath $smokeTestScript -PathType Leaf)) {
    throw "The installer smoke-test script is missing: $smokeTestScript"
}
foreach ($requiredFile in @($licenseSource, $noticesSource, $translationLicenseSource)) {
    Assert-File -Path $requiredFile -Description "Required package notice"
}

$stableRust = (& rustup run stable rustc -vV) -join "`n"
if ($LASTEXITCODE -ne 0 -or $stableRust -notmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$') {
    throw "Rust stable must use the x86_64-pc-windows-msvc host."
}
$activeRust = (& rustc -vV) -join "`n"
if ($LASTEXITCODE -ne 0 -or $activeRust -notmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$') {
    throw "The active Rust toolchain must use the x86_64-pc-windows-msvc host."
}
if ((& rustup run stable rustc -V) -join "" -cne ((& rustc -V) -join "")) {
    throw "The active Rust compiler does not match the installed stable toolchain."
}

if ([string]::IsNullOrWhiteSpace($env:GITHUB_ACTIONS)) {
    $msvcScript = Join-Path $repositoryRoot "scripts\enable-windows-msvc.ps1"
    $previousGithubEnv = [Environment]::GetEnvironmentVariable("GITHUB_ENV", "Process")
    $temporaryGithubEnv = Join-Path ([System.IO.Path]::GetTempPath()) (
        "codex-auth-switcher-msvc-" + [guid]::NewGuid().ToString("N") + ".env"
    )
    try {
        [Environment]::SetEnvironmentVariable("GITHUB_ENV", $temporaryGithubEnv, "Process")
        . $msvcScript
    }
    finally {
        [Environment]::SetEnvironmentVariable("GITHUB_ENV", $previousGithubEnv, "Process")
        if (Test-Path -LiteralPath $temporaryGithubEnv) {
            Remove-Item -LiteralPath $temporaryGithubEnv -Force
        }
    }
}

if ($env:CMAKE_GENERATOR -notmatch '^Visual Studio (17 2022|18 2026)$' -or
    $env:CMAKE_GENERATOR_PLATFORM -cne "x64" -or
    $env:CMAKE_GENERATOR_TOOLSET -cne "host=x64") {
    throw "Initialize the supported x64 MSVC/CMake environment before building."
}
foreach ($toolName in @("cl.exe", "link.exe", "rc.exe", "cmake.exe")) {
    if (-not (Get-Command $toolName -ErrorAction SilentlyContinue)) {
        throw "Required Windows build tool is missing: $toolName"
    }
}
$expectedTargetDirectory = [System.IO.Path]::GetFullPath($targetDirectory)
if ($env:CARGO_TARGET_DIR -and
    [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) -ine $expectedTargetDirectory) {
    throw "Cargo target directory must remain inside the repository target folder."
}

$metadata = Get-DesktopMetadata
Assert-TagMatchesVersion -Version $metadata.Version
$versionInfo = Get-VersionInfoNumber -Version $metadata.Version
$sourceCommit = Get-SourceCommit
$rustVersion = ((& rustup run stable rustc -V) -join "").Trim()
$buildInputsClean = Test-RustBuildInputsClean
$buildConfiguration = "target.x86_64-pc-windows-msvc.rustflags=['-C','target-feature=+crt-static']"

Push-Location $repositoryRoot
try {
    & cargo build --release --locked --manifest-path $manifestPath --package switcher-desktop --config $buildConfiguration
    if ($LASTEXITCODE -ne 0) {
        throw "The Windows x64 static-CRT release build failed."
    }
}
finally {
    Pop-Location
}
Assert-File -Path $binaryPath -Description "Windows release executable"

$prepareOutput = @(& $innoPreparationScript -ToolsDirectory $innoToolsDirectory)
if ($prepareOutput.Count -eq 0) {
    throw "Pinned Inno Setup 6.7.4 could not be prepared."
}
$compiler = [string]$prepareOutput[-1]
Assert-File -Path $compiler -Description "Pinned Inno Setup compiler"

$setupBaseName = "Codex-Auth-Switcher-$($metadata.Version)-windows-x64-setup"
$setupPath = Join-Path $outputPath "$setupBaseName.exe"
$checksumPath = Join-Path $outputPath "SHA256SUMS.txt"
foreach ($path in @($setupPath, $checksumPath)) {
    if (Test-Path -LiteralPath $path) {
        throw "Refusing to overwrite an existing Windows release artifact: $path"
    }
}
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null

$packageStage = Join-Path $targetDirectory ("windows-package\stage-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $packageStage -Force | Out-Null
$sourcesPath = Join-Path $packageStage "SOURCES.txt"
$sourceCommitText = if ($null -eq $sourceCommit) { "unavailable" } else { $sourceCommit }
$sourceStateText = if ($buildInputsClean) {
    "Cargo manifests, lockfile, toolchain and Rust sources are clean at the source commit."
}
else {
    "The Rust build inputs contain working-tree or index changes; the source commit is the baseline only."
}
$sourcesLines = @(
    "Codex Auth Switcher $($metadata.Version)"
    "Source repository: $($metadata.Repository)"
    "Source commit: $sourceCommitText"
    "Rust build input state: $sourceStateText"
    "Windows target: x86_64-pc-windows-msvc"
    "Rust compiler: $rustVersion"
    "Build command: cargo build --release --locked --manifest-path Cargo.toml --package switcher-desktop --config $buildConfiguration"
    "Dependency lockfile: Cargo.lock at the source commit or current build tree as described above"
    "Windows C runtime: statically linked MSVC runtime"
    "Installer compiler: Inno Setup 6.7.4 (official signed setup SHA-256 a549929319f9120716677c685535ca4671e4b937b5fdc87e2585f32d8eb30e8c)"
    "Packaging behavior reference: ramag-platform commit 59399356bc88f215c8eacca632dcb3e13953632a, scripts/package-windows.ps1 and scripts/windows/ramag.iss; independently adapted for this GPL-3.0-only project."
)
[System.IO.File]::WriteAllText(
    $sourcesPath,
    (($sourcesLines -join "`n") + "`n"),
    [System.Text.UTF8Encoding]::new($false)
)

$environmentValues = @{
    "CAS_PACKAGE_EXE" = [Environment]::GetEnvironmentVariable("CAS_PACKAGE_EXE", "Process")
    "CAS_PACKAGE_VERSION" = [Environment]::GetEnvironmentVariable("CAS_PACKAGE_VERSION", "Process")
    "CAS_PACKAGE_VERSION_INFO" = [Environment]::GetEnvironmentVariable("CAS_PACKAGE_VERSION_INFO", "Process")
    "CAS_PACKAGE_STAGE_DIR" = [Environment]::GetEnvironmentVariable("CAS_PACKAGE_STAGE_DIR", "Process")
}
try {
    [Environment]::SetEnvironmentVariable("CAS_PACKAGE_EXE", $binaryPath, "Process")
    [Environment]::SetEnvironmentVariable("CAS_PACKAGE_VERSION", $metadata.Version, "Process")
    [Environment]::SetEnvironmentVariable("CAS_PACKAGE_VERSION_INFO", $versionInfo, "Process")
    [Environment]::SetEnvironmentVariable("CAS_PACKAGE_STAGE_DIR", $packageStage, "Process")
    & $compiler "/Qp" "/O$outputPath" "/F$setupBaseName" $innoScript
    if ($LASTEXITCODE -ne 0) {
        throw "Inno Setup compilation failed with exit code $LASTEXITCODE."
    }
}
finally {
    foreach ($name in $environmentValues.Keys) {
        [Environment]::SetEnvironmentVariable($name, $environmentValues[$name], "Process")
    }
}

Assert-File -Path $setupPath -Description "Windows x64 setup executable"
if ($SmokeTest) {
    & $smokeTestScript `
        -SetupPath $setupPath `
        -ExpectedVersion $metadata.Version `
        -ExpectedCommit $sourceCommitText `
        -WorkDirectory $packageStage
}

$setupHash = (Get-FileHash -LiteralPath $setupPath -Algorithm SHA256).Hash.ToLowerInvariant()
[System.IO.File]::WriteAllText(
    $checksumPath,
    "$setupHash  $([System.IO.Path]::GetFileName($setupPath))`n",
    [System.Text.UTF8Encoding]::new($false)
)
$verifiedHash = (Get-FileHash -LiteralPath $setupPath -Algorithm SHA256).Hash.ToLowerInvariant()
$checksumLine = [System.IO.File]::ReadAllText($checksumPath)
if ($checksumLine -cne "$verifiedHash  $([System.IO.Path]::GetFileName($setupPath))`n") {
    throw "Windows setup SHA-256 checksum verification failed."
}

[PSCustomObject]@{
    Installer = $setupPath
    InstallerSha256 = $verifiedHash
    Checksums = $checksumPath
    Sources = $sourcesPath
    Target = "x86_64-pc-windows-msvc"
    Version = $metadata.Version
}
