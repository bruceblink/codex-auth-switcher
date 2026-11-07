[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw "This test requires Windows."
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$enableScript = Join-Path $repositoryRoot "scripts\enable-windows-msvc.ps1"
$temporaryRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$testDirectory = Join-Path $temporaryRoot ("codex-auth-switcher-msvc-test-" + [guid]::NewGuid().ToString("N"))
$shadowDirectory = Join-Path $testDirectory "shadow-linker"
$githubEnvironmentPath = Join-Path $testDirectory "github-environment.txt"
$originalPath = [Environment]::GetEnvironmentVariable("PATH", "Process")
$originalGithubEnvironment = [Environment]::GetEnvironmentVariable("GITHUB_ENV", "Process")

try {
    New-Item -ItemType Directory -Path $shadowDirectory -Force | Out-Null

    $existingLinker = Get-Command link.exe -CommandType Application -ErrorAction Stop
    Copy-Item -LiteralPath $existingLinker.Source -Destination (Join-Path $shadowDirectory "link.exe")

    [Environment]::SetEnvironmentVariable("PATH", "$shadowDirectory;$originalPath", "Process")
    [Environment]::SetEnvironmentVariable("GITHUB_ENV", $githubEnvironmentPath, "Process")

    $shadowLinker = Get-Command link.exe -CommandType Application -All -ErrorAction Stop |
        Select-Object -First 1
    $expectedShadowLinker = [System.IO.Path]::GetFullPath(
        (Join-Path $shadowDirectory "link.exe")
    )
    if ([System.IO.Path]::GetFullPath($shadowLinker.Source) -ine $expectedShadowLinker) {
        throw "Test setup did not place the shadow linker first on PATH."
    }

    & $enableScript

    $expectedLinkerDirectory = Join-Path $env:VCToolsInstallDir "bin\Hostx64\x64"
    $expectedLinker = [System.IO.Path]::GetFullPath((Join-Path $expectedLinkerDirectory "link.exe"))
    $pathEntries = [Environment]::GetEnvironmentVariable("PATH", "Process") -split ';'
    if ([System.IO.Path]::GetFullPath($pathEntries[0]) -ine $expectedLinkerDirectory) {
        throw "The selected MSVC linker directory is not first on the process PATH."
    }

    $resolvedLinker = Get-Command link.exe -CommandType Application -All -ErrorAction Stop |
        Select-Object -First 1
    if ([System.IO.Path]::GetFullPath($resolvedLinker.Source) -ine $expectedLinker) {
        throw "Get-Command did not resolve the selected MSVC linker first."
    }

    $environmentPathLine = Get-Content -LiteralPath $githubEnvironmentPath |
        Where-Object { $_.StartsWith("PATH=", [System.StringComparison]::OrdinalIgnoreCase) } |
        Select-Object -Last 1
    if ($null -eq $environmentPathLine) {
        throw "The initialized PATH was not written to GITHUB_ENV."
    }
    $githubEnvironmentPathValue = $environmentPathLine.Substring($environmentPathLine.IndexOf('=') + 1)
    if ($githubEnvironmentPathValue -cne [Environment]::GetEnvironmentVariable("PATH", "Process")) {
        throw "The process PATH and GITHUB_ENV PATH do not match."
    }

    $rustcInfo = (& rustup run stable rustc -Vv) -join "`n"
    if ($LASTEXITCODE -ne 0 -or
        $rustcInfo -notmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$') {
        throw "The stable Rust compiler is not using the x86_64-pc-windows-msvc host."
    }

    foreach ($toolName in @("cl.exe", "link.exe", "rc.exe", "cmake.exe")) {
        if (-not (Get-Command $toolName -ErrorAction SilentlyContinue)) {
            throw "Required Windows build tool is missing after initialization: $toolName"
        }
    }

    Write-Output "MSVC initialization test passed: the selected linker takes precedence over an existing PATH linker."
}
finally {
    [Environment]::SetEnvironmentVariable("PATH", $originalPath, "Process")
    [Environment]::SetEnvironmentVariable("GITHUB_ENV", $originalGithubEnvironment, "Process")

    $resolvedTestDirectory = [System.IO.Path]::GetFullPath($testDirectory)
    if ($resolvedTestDirectory.StartsWith(
            $temporaryRoot,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -and (Test-Path -LiteralPath $resolvedTestDirectory)) {
        Remove-Item -LiteralPath $resolvedTestDirectory -Recurse -Force
    }
}
