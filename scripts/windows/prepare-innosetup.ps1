[CmdletBinding()]
param(
    [string]$ToolsDirectory = "target\build-tools\innosetup-6.7.4"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$installerVersion = "6.7.4"
$installerName = "innosetup-$installerVersion.exe"
$installerUrl = "https://github.com/jrsoftware/issrc/releases/download/is-6_7_4/$installerName"
$installerSha256 = "a549929319f9120716677c685535ca4671e4b937b5fdc87e2585f32d8eb30e8c"
$installerSignerThumbprint = "E0AB19C8D38CBF9C44709925122A7A02F8C70CB7"
$compilerSha256 = "f02fa0e5f237b47d7ed1edae9b991866ec94fe02af61eb6a73e94f92fe8e0604"

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw "Inno Setup can only be prepared on Windows."
}

$toolsPath = if ([System.IO.Path]::IsPathRooted($ToolsDirectory)) {
    [System.IO.Path]::GetFullPath($ToolsDirectory)
}
else {
    $repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
    [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot $ToolsDirectory))
}
$installerPath = Join-Path $toolsPath $installerName
$compilerPath = Join-Path $toolsPath "ISCC.exe"
New-Item -ItemType Directory -Path $toolsPath -Force | Out-Null

function Assert-OfficialInstaller {
    param([Parameter(Mandatory = $true)][string]$Path)

    $actualHash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -cne $installerSha256) {
        throw "Pinned Inno Setup installer SHA-256 verification failed."
    }
    $signature = Get-AuthenticodeSignature -FilePath $Path
    if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
        $null -eq $signature.SignerCertificate -or
        $signature.SignerCertificate.Thumbprint -cne $installerSignerThumbprint) {
        throw "Pinned Inno Setup installer does not have the expected valid Pyrsys B.V. signature."
    }
}

if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
    $downloadPath = Join-Path $toolsPath (".download-" + [guid]::NewGuid().ToString("N"))
    try {
        $previousProgressPreference = $ProgressPreference
        $ProgressPreference = "SilentlyContinue"
        try {
            Invoke-WebRequest -Uri $installerUrl -OutFile $downloadPath -UseBasicParsing
        }
        finally {
            $ProgressPreference = $previousProgressPreference
        }
        Assert-OfficialInstaller -Path $downloadPath
        Move-Item -LiteralPath $downloadPath -Destination $installerPath
    }
    finally {
        if (Test-Path -LiteralPath $downloadPath) {
            Remove-Item -LiteralPath $downloadPath -Force
        }
    }
}
else {
    Assert-OfficialInstaller -Path $installerPath
}

if (-not (Test-Path -LiteralPath $compilerPath -PathType Leaf)) {
    $arguments = @(
        "/VERYSILENT",
        "/SUPPRESSMSGBOXES",
        "/NORESTART",
        "/NOICONS",
        "/CURRENTUSER",
        "/PORTABLE=1",
        "/DIR=`"$toolsPath`""
    )
    $process = Start-Process -FilePath $installerPath -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) {
        throw "Portable Inno Setup $installerVersion preparation failed with exit code $($process.ExitCode)."
    }
}

if (-not (Test-Path -LiteralPath $compilerPath -PathType Leaf)) {
    throw "The pinned Inno Setup compiler was not created: $compilerPath"
}
$actualCompilerHash = (Get-FileHash -LiteralPath $compilerPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualCompilerHash -cne $compilerSha256) {
    throw "Pinned Inno Setup compiler SHA-256 verification failed."
}

Write-Host "Verified Inno Setup $installerVersion from the official signed release."
Write-Output ([System.IO.Path]::GetFullPath($compilerPath))
