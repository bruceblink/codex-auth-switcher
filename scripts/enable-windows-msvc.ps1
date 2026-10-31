$ErrorActionPreference = "Stop"

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw "The Windows MSVC environment can only be initialized on Windows."
}
if ([string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) {
    throw "GITHUB_ENV is required to pass the MSVC environment to later workflow steps."
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$programFilesX86 = [Environment]::GetFolderPath("ProgramFilesX86")
$vswhere = Join-Path $programFilesX86 "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
    throw "vswhere.exe is required to locate the Visual Studio C++ toolchain."
}

$requirements = @(
    "-latest",
    "-products", "*",
    "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"
)
$installationPath = & $vswhere @requirements -property installationPath
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($installationPath)) {
    throw "A Visual Studio installation with the x64 MSVC tools is required."
}
$installationVersion = & $vswhere @requirements -property installationVersion
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($installationVersion)) {
    throw "The Visual Studio installation version could not be read."
}

$majorVersion = [int]([version]$installationVersion).Major
$generatorYear = switch ($majorVersion) {
    17 { "2022" }
    18 { "2026" }
    default { throw "Unsupported Visual Studio version: $installationVersion" }
}
$vcvarsAll = Join-Path $installationPath "VC\Auxiliary\Build\vcvarsall.bat"
if (-not (Test-Path -LiteralPath $vcvarsAll -PathType Leaf)) {
    throw "Visual Studio x64 environment setup was not found: $vcvarsAll"
}

$command = 'call "{0}" x64 >nul && set' -f $vcvarsAll
$output = @(& cmd.exe /d /s /c $command 2>&1)
if ($LASTEXITCODE -ne 0) {
    throw "vcvarsall.bat failed with exit code $LASTEXITCODE."
}
$visualStudioEnvironment = @{}
foreach ($line in $output) {
    if ([string]$line -match '^([^=]+)=(.*)$') {
        $visualStudioEnvironment[$Matches[1]] = $Matches[2]
    }
}
foreach ($requiredName in @("PATH", "INCLUDE", "LIB", "VCToolsInstallDir", "WindowsSdkDir", "WindowsSDKVersion")) {
    if (-not $visualStudioEnvironment.ContainsKey($requiredName)) {
        throw "vcvarsall.bat did not set $requiredName."
    }
}

$compiler = Join-Path $visualStudioEnvironment["VCToolsInstallDir"] "bin\Hostx64\x64\cl.exe"
$linker = Join-Path $visualStudioEnvironment["VCToolsInstallDir"] "bin\Hostx64\x64\link.exe"
$sdkVersion = ([string]$visualStudioEnvironment["WindowsSDKVersion"]).TrimEnd([char]'\')
$resourceCompiler = Join-Path $visualStudioEnvironment["WindowsSdkDir"] "bin\$sdkVersion\x64\rc.exe"
foreach ($toolPath in @($compiler, $linker, $resourceCompiler)) {
    if (-not (Test-Path -LiteralPath $toolPath -PathType Leaf)) {
        throw "A required MSVC tool was not found: $toolPath"
    }
}

$environmentNames = @(
    "PATH", "INCLUDE", "LIB", "LIBPATH",
    "PROCESSOR_ARCHITECTURE", "PROCESSOR_ARCHITEW6432",
    "VCToolsInstallDir", "VCToolsVersion", "VCINSTALLDIR", "VSINSTALLDIR",
    "VisualStudioVersion", "VSCMD_ARG_HOST_ARCH", "VSCMD_ARG_TGT_ARCH", "VSCMD_VER",
    "WindowsSdkDir", "WindowsSDKVersion", "UniversalCRTSdkDir", "UCRTVersion",
    "DevEnvDir", "CommandPromptType"
)
foreach ($name in $environmentNames) {
    if ($visualStudioEnvironment.ContainsKey($name)) {
        $value = [string]$visualStudioEnvironment[$name]
        [Environment]::SetEnvironmentVariable($name, $value, "Process")
        "$name=$value" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
    }
}

$settings = @{
    CMAKE_GENERATOR = "Visual Studio $majorVersion $generatorYear"
    CMAKE_GENERATOR_PLATFORM = "x64"
    CMAKE_GENERATOR_INSTANCE = $installationPath
    CMAKE_GENERATOR_TOOLSET = "host=x64"
    CARGO_TARGET_DIR = Join-Path $repositoryRoot "target"
    CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $linker
}
foreach ($entry in $settings.GetEnumerator()) {
    $value = [string]$entry.Value
    [Environment]::SetEnvironmentVariable($entry.Key, $value, "Process")
    "$($entry.Key)=$value" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
}

$expectedTargetDirectory = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot "target"))
$actualTargetDirectory = [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
if ($actualTargetDirectory -ine $expectedTargetDirectory) {
    throw "Cargo target directory is not repository-local: $actualTargetDirectory"
}
$rustcInfo = (& rustc -vV) -join "`n"
if ($LASTEXITCODE -ne 0 -or $rustcInfo -notmatch '(?m)^host:\s+x86_64-pc-windows-msvc\s*$') {
    throw "Rust stable must use the x86_64-pc-windows-msvc host."
}

$expectedGenerator = $env:CMAKE_GENERATOR
if ($expectedGenerator -notin @("Visual Studio 17 2022", "Visual Studio 18 2026")) {
    throw "Unsupported Visual Studio CMake generator: $expectedGenerator"
}
if ($env:CMAKE_GENERATOR_PLATFORM -ne "x64" -or $env:CMAKE_GENERATOR_TOOLSET -ne "host=x64") {
    throw "The Visual Studio CMake generator configuration is inconsistent."
}
if (-not (Test-Path -LiteralPath $env:CMAKE_GENERATOR_INSTANCE -PathType Container)) {
    throw "The configured Visual Studio instance does not exist: $env:CMAKE_GENERATOR_INSTANCE"
}
$cmakeHelp = (& cmake --help) -join "`n"
if ($LASTEXITCODE -ne 0 -or $cmakeHelp -notmatch [regex]::Escape($expectedGenerator)) {
    throw "CMake does not support the configured generator: $expectedGenerator"
}
foreach ($toolName in @("cl.exe", "link.exe", "rc.exe", "cmake.exe")) {
    $tool = Get-Command $toolName -ErrorAction Stop
    Write-Output "$toolName=$($tool.Source)"
}
rustc -vV
cmake --version
