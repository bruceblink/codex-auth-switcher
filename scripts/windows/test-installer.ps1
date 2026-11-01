[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SetupPath,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedVersion,
    [Parameter(Mandatory = $true)]
    [string]$ExpectedCommit,
    [Parameter(Mandatory = $true)]
    [string]$WorkDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$appId = "com.bruceblink.codex-auth-switcher"
$appName = "Codex Auth Switcher"
$exeName = "codex-auth-switcher.exe"
$uninstallKeyName = "${appId}_is1"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$uninstallKeyName"
$forbiddenUninstallKeys = @(
    "HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\$uninstallKeyName",
    "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$uninstallKeyName",
    "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\$uninstallKeyName"
)

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

function Assert-NoExistingInstallation {
    $runningProcesses = @(Get-Process -Name "codex-auth-switcher" -ErrorAction SilentlyContinue)
    if ($runningProcesses.Count -gt 0) {
        throw "Codex Auth Switcher is running. Close it before running the isolated installer test."
    }

    $existingRegistryKeys = @(
        @($uninstallKey) + $forbiddenUninstallKeys |
            Where-Object { Test-Path -LiteralPath $_ }
    )
    if ($existingRegistryKeys.Count -gt 0) {
        throw "An installation using AppId '$appId' already exists; refusing to replace it."
    }

    $defaultInstallDirectory = Join-Path $env:LOCALAPPDATA "Programs\Codex Auth Switcher"
    if (Test-Path -LiteralPath $defaultInstallDirectory) {
        throw "The default install directory already exists; refusing to risk replacing existing files."
    }

    $programsDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
    $startMenuFolder = Join-Path $programsDirectory $appName
    $desktopShortcut = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)) "$appName.lnk"
    if ((Test-Path -LiteralPath $startMenuFolder) -or (Test-Path -LiteralPath $desktopShortcut)) {
        throw "A Codex Auth Switcher shortcut already exists; refusing to replace existing user files."
    }
}

function Invoke-CheckedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$ArgumentList,
        [Parameter(Mandatory = $true)][string]$Description
    )

    $process = Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) {
        throw "$Description failed with exit code $($process.ExitCode)."
    }
}

function Assert-IconResource {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Description
    )

    $icon = [System.Drawing.Icon]::ExtractAssociatedIcon($Path)
    if ($null -eq $icon) {
        throw "$Description does not expose an executable icon resource."
    }
    try {
        if ($icon.Width -le 0 -or $icon.Height -le 0) {
            throw "$Description icon dimensions are invalid."
        }
    }
    finally {
        $icon.Dispose()
    }
}

function Assert-RegistryValue {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ExpectedValue
    )
    $value = (Get-ItemProperty -LiteralPath $uninstallKey -Name $Name).$Name
    if ([string]$value -cne $ExpectedValue) {
        throw "Installer registry value '$Name' was '$value', expected '$ExpectedValue'."
    }
}

function Assert-IsolatedCodexDataUnchanged {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][System.Collections.IDictionary]$ExpectedHashes
    )

    foreach ($name in $ExpectedHashes.Keys) {
        $path = Join-Path $Directory $name
        Assert-File -Path $path -Description "Synthetic Codex data"
        $actualHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
        if ($actualHash -cne $ExpectedHashes[$name]) {
            throw "The isolated synthetic Codex file changed during the installer test: $name"
        }
    }
}

function Wait-ForUninstall {
    param(
        [Parameter(Mandatory = $true)][string]$InstallDirectory,
        [Parameter(Mandatory = $true)][string]$InstalledExecutable
    )

    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        $remainingFiles = @(
            Get-ChildItem -LiteralPath $InstallDirectory -Force -Recurse -File -ErrorAction SilentlyContinue
        )
        if (-not (Test-Path -LiteralPath $InstalledExecutable) -and $remainingFiles.Count -eq 0) {
            return
        }
        Start-Sleep -Milliseconds 250
    }
    throw "Uninstallation left files in the isolated install directory: $InstallDirectory"
}

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw "The Windows installer test must run on Windows."
}
if ($ExpectedVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "Expected package version must use three numeric components."
}
if ($ExpectedCommit -ne "unavailable" -and $ExpectedCommit -notmatch '^[0-9a-f]{40}$') {
    throw "Expected source commit must be a 40-character Git commit."
}
Assert-File -Path $SetupPath -Description "Windows setup executable"
Assert-NoExistingInstallation

$workPath = [System.IO.Path]::GetFullPath($WorkDirectory)
$smokeDirectory = Join-Path $workPath "smoke-test"
$installDirectory = Join-Path $smokeDirectory "install"
$codexHome = Join-Path $smokeDirectory "isolated-codex-home"
$installLog = Join-Path $smokeDirectory "install.log"
$uninstallLog = Join-Path $smokeDirectory "uninstall.log"
if (Test-Path -LiteralPath $smokeDirectory) {
    throw "The isolated installer test directory already exists: $smokeDirectory"
}
New-Item -ItemType Directory -Path $installDirectory, $codexHome -Force | Out-Null
$syntheticCodexData = [ordered]@{
    "config.toml" = "model = `"gpt-5.6`"`n"
    "auth.json" = "{}`n"
}
$expectedCodexDataHashes = @{}
foreach ($name in $syntheticCodexData.Keys) {
    $path = Join-Path $codexHome $name
    [System.IO.File]::WriteAllText($path, $syntheticCodexData[$name], [System.Text.UTF8Encoding]::new($false))
    $expectedCodexDataHashes[$name] = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
}

$installed = $false
$applicationProcess = $null
$previousCodexHome = [Environment]::GetEnvironmentVariable("CODEX_HOME", "Process")
try {
    $installed = $true
    Invoke-CheckedProcess `
        -FilePath ([System.IO.Path]::GetFullPath($SetupPath)) `
        -ArgumentList @(
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/SP-",
            "/DIR=`"$installDirectory`"",
            "/TASKS=desktopicon",
            "/LOG=`"$installLog`""
        ) `
        -Description "Current-user silent setup"
    Assert-IsolatedCodexDataUnchanged -Directory $codexHome -ExpectedHashes $expectedCodexDataHashes

    $installedExecutable = Join-Path $installDirectory $exeName
    Assert-File -Path $installedExecutable -Description "Installed application"
    Assert-File -Path (Join-Path $installDirectory "LICENSE") -Description "GPLv3 license"
    Assert-File -Path (Join-Path $installDirectory "THIRD-PARTY-NOTICES.html") -Description "Third-party notices"
    Assert-File -Path (Join-Path $installDirectory "SOURCES.txt") -Description "Versioned source record"
    Assert-File -Path (Join-Path $installDirectory "LICENSE-Inno-Chinese-Translation.txt") -Description "Chinese translation license"

    Assert-IconResource -Path ([System.IO.Path]::GetFullPath($SetupPath)) -Description "Setup executable"
    Assert-IconResource -Path $installedExecutable -Description "Installed application"
    $sourceRecord = [System.IO.File]::ReadAllText((Join-Path $installDirectory "SOURCES.txt"))
    if ($sourceRecord -notmatch "(?m)^Codex Auth Switcher $([regex]::Escape($ExpectedVersion))$" -or
        $sourceRecord -notmatch "(?m)^Source commit: $([regex]::Escape($ExpectedCommit))$") {
        throw "Installed SOURCES.txt does not match the expected version and commit."
    }

    if (-not (Test-Path -LiteralPath $uninstallKey)) {
        throw "The current-user uninstall registration was not created."
    }
    Assert-RegistryValue -Name "DisplayName" -ExpectedValue $appName
    Assert-RegistryValue -Name "DisplayVersion" -ExpectedValue $ExpectedVersion
    Assert-RegistryValue -Name "DisplayIcon" -ExpectedValue $installedExecutable
    if (@($forbiddenUninstallKeys | Where-Object { Test-Path -LiteralPath $_ }).Count -gt 0) {
        throw "The per-user installer unexpectedly wrote a machine-wide or 32-bit uninstall record."
    }

    $programsDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
    $startMenuShortcut = Join-Path $programsDirectory "$appName\$appName.lnk"
    $desktopShortcut = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)) "$appName.lnk"
    Assert-File -Path $startMenuShortcut -Description "Start-menu shortcut"
    Assert-File -Path $desktopShortcut -Description "Optional desktop shortcut"

    [Environment]::SetEnvironmentVariable("CODEX_HOME", $codexHome, "Process")
    $applicationProcess = Start-Process -FilePath $installedExecutable -WorkingDirectory $installDirectory -PassThru
    $windowReady = $false
    for ($attempt = 0; $attempt -lt 80; $attempt++) {
        $applicationProcess.Refresh()
        if ($applicationProcess.HasExited) {
            throw "The installed GUI exited during startup with code $($applicationProcess.ExitCode)."
        }
        if ($applicationProcess.MainWindowHandle -ne [IntPtr]::Zero) {
            $windowReady = $true
            break
        }
        Start-Sleep -Milliseconds 250
    }
    if (-not $windowReady) {
        throw "The installed GUI did not create a top-level window within 20 seconds."
    }
    Assert-IsolatedCodexDataUnchanged -Directory $codexHome -ExpectedHashes $expectedCodexDataHashes
    Write-Host "Installed GUI process created a native window under isolated CODEX_HOME."

    Stop-Process -Id $applicationProcess.Id -Force
    $applicationProcess.WaitForExit()
    $applicationProcess.Dispose()
    $applicationProcess = $null

    $uninstallers = @(Get-ChildItem -LiteralPath $installDirectory -Filter "unins*.exe" -File)
    if ($uninstallers.Count -ne 1) {
        throw "Expected one uninstall program in the isolated installation, found $($uninstallers.Count)."
    }
    Invoke-CheckedProcess `
        -FilePath $uninstallers[0].FullName `
        -ArgumentList @(
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/LOG=`"$uninstallLog`""
        ) `
        -Description "Current-user silent uninstall"
    $installed = $false

    Wait-ForUninstall -InstallDirectory $installDirectory -InstalledExecutable $installedExecutable
    Assert-IsolatedCodexDataUnchanged -Directory $codexHome -ExpectedHashes $expectedCodexDataHashes
    Assert-NoExistingInstallation
    if (Test-Path -LiteralPath $startMenuShortcut) {
        throw "Uninstallation left the Start-menu shortcut behind."
    }
    if (Test-Path -LiteralPath $desktopShortcut) {
        throw "Uninstallation left the optional desktop shortcut behind."
    }
    Write-Host "Isolated current-user install, GUI startup, and uninstall checks passed."
}
finally {
    [Environment]::SetEnvironmentVariable("CODEX_HOME", $previousCodexHome, "Process")
    if ($null -ne $applicationProcess) {
        $applicationProcess.Refresh()
        if (-not $applicationProcess.HasExited) {
            Stop-Process -Id $applicationProcess.Id -Force
            $applicationProcess.WaitForExit()
        }
        $applicationProcess.Dispose()
    }
    if ($installed) {
        $fallbackUninstaller = Get-ChildItem `
            -LiteralPath $installDirectory `
            -Filter "unins*.exe" `
            -File `
            -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($null -ne $fallbackUninstaller) {
            try {
                Invoke-CheckedProcess `
                    -FilePath $fallbackUninstaller.FullName `
                    -ArgumentList @("/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART") `
                    -Description "Isolated test cleanup uninstall"
            }
            catch {
                Write-Warning "Could not finish isolated installer cleanup: $($_.Exception.Message)"
            }
        }
    }
}
