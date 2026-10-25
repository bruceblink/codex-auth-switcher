param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot "..\target\release\codex-auth-switcher.exe")
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "The V2 executable was not found."
}

$script:ExecutablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
if ([System.IO.Path]::GetExtension($script:ExecutablePath) -ne ".exe") {
    throw "The native UI test requires a Windows executable, not a ZIP archive."
}
$script:TestRoot = Join-Path ([System.IO.Path]::GetTempPath()) "codex-auth-switcher-native-ui-$([guid]::NewGuid().ToString('N'))"
$script:CodexHome = Join-Path $script:TestRoot "codex-home"
$script:OriginalPath = $env:PATH
$script:Process = $null

function Get-MainWindow {
    if ($null -eq $script:Process) {
        return $null
    }

    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $processCondition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ProcessIdProperty,
        $script:Process.Id
    )
    return $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $processCondition)
}

function Find-Button([string]$Name) {
    $window = Get-MainWindow
    if ($null -eq $window) {
        return $null
    }

    $buttonCondition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button
    )
    $buttons = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    foreach ($button in $buttons) {
        if ($button.Current.Name -eq $Name) {
            return $button
        }
    }
    return $null
}

function Wait-ForButton([string]$Name, [bool]$RequireEnabled = $true) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline) {
        $button = Find-Button $Name
        if ($null -ne $button -and (-not $RequireEnabled -or $button.Current.IsEnabled)) {
            return $button
        }
        Start-Sleep -Milliseconds 100
    }
    throw "The native GUI button '$Name' did not become available."
}

function Click-NativeButton([string]$Name) {
    $button = Wait-ForButton $Name
    $window = Get-MainWindow
    $windowHandle = [IntPtr]$window.Current.NativeWindowHandle
    $null = [NativeUiInput]::ShowWindow($windowHandle, 3)
    $null = [NativeUiInput]::SetForegroundWindow($windowHandle)
    Start-Sleep -Milliseconds 100
    $button = Wait-ForButton $Name
    try {
        $point = $button.GetClickablePoint()
        $x = [int][Math]::Round($point.X)
        $y = [int][Math]::Round($point.Y)
    }
    catch {
        $bounds = $button.Current.BoundingRectangle
        if ($button.Current.IsOffscreen -or $bounds.Width -le 0 -or $bounds.Height -le 0) {
            throw "The native GUI button '$Name' has no visible click target."
        }
        $x = [int][Math]::Round($bounds.X + $bounds.Width / 2)
        $y = [int][Math]::Round($bounds.Y + $bounds.Height / 2)
    }
    if (-not [NativeUiInput]::SetCursorPos($x, $y)) {
        throw "The native GUI button '$Name' could not be reached."
    }
    [NativeUiInput]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 40
    [NativeUiInput]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 100
}

function Select-Target([string]$Name) {
    $button = Wait-ForButton $Name
    $pattern = $button.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
    if ($pattern.Current.ToggleState -ne [System.Windows.Automation.ToggleState]::On) {
        Click-NativeButton $Name
    }

    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $deadline) {
        $button = Find-Button $Name
        if ($null -ne $button) {
            $pattern = $button.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
            if ($pattern.Current.ToggleState -eq [System.Windows.Automation.ToggleState]::On) {
                return
            }
        }
        Start-Sleep -Milliseconds 100
    }
    throw "The native GUI did not select target '$Name'."
}

function Read-Status {
    $output = & $script:ExecutablePath --command status --codex-home $script:CodexHome --isolated 2>$null
    if ($LASTEXITCODE -ne 0) {
        return $null
    }
    return ($output -join "`n") | ConvertFrom-Json
}

function Wait-ForMode([string]$Mode) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline) {
        $status = Read-Status
        if ($null -ne $status -and $status.mode -eq $Mode -and $status.auth_mode -eq $Mode) {
            return $status
        }
        Start-Sleep -Milliseconds 100
    }
    throw "The native GUI did not apply mode '$Mode'."
}

try {
    New-Item -ItemType Directory -Path $script:TestRoot | Out-Null
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

public static class NativeUiInput
{
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr windowHandle);

    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr windowHandle, int command);
}
"@

    # Restrict the launched process to Windows system tools to verify the portable runtime boundary.
    $env:PATH = Join-Path $env:SystemRoot "System32"
    $script:Process = Start-Process -FilePath $script:ExecutablePath `
        -ArgumentList @("--demo", "--isolated", "--codex-home", $script:CodexHome) -PassThru

    $windowDeadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $windowDeadline -and $null -eq (Get-MainWindow)) {
        if ($script:Process.HasExited) {
            throw "The packaged GUI exited before creating a native window."
        }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq (Get-MainWindow)) {
        throw "The packaged GUI did not create a native window."
    }

    Click-NativeButton "初始化模式"
    $null = Wait-ForButton "补齐模式"

    # The demo starts in subscription mode, so apply the stored API profile through the real controls.
    Select-Target "API key"
    Click-NativeButton "预览变更"
    $null = Wait-ForButton "应用切换"
    Click-NativeButton "应用切换"
    $apiStatus = Wait-ForMode "api"
    if ($apiStatus.model -ne "demo-api-model") {
        throw "The native GUI applied the wrong API model."
    }

    Select-Target "订阅"
    Click-NativeButton "预览变更"
    $null = Wait-ForButton "应用切换"
    Click-NativeButton "应用切换"
    $subscriptionStatus = Wait-ForMode "subscription"
    if ($subscriptionStatus.model -ne "demo-subscription-model") {
        throw "The native GUI did not restore the demo subscription model."
    }

    $config = Get-Content -LiteralPath (Join-Path $script:CodexHome "config.toml") -Raw
    $auth = Get-Content -LiteralPath (Join-Path $script:CodexHome "auth.json") -Raw | ConvertFrom-Json
    if (-not $config.Contains("[features]") -or -not $config.Contains('[projects."F:/example"]')) {
        throw "The native GUI did not preserve shared Codex settings."
    }
    if ($auth.auth_mode -ne "chatgpt") {
        throw "The native GUI did not restore subscription authentication."
    }

    Write-Output "V2 native Windows UI acceptance passed: initialize, target selection, preview, API apply, subscription restore, and shared-setting preservation."
}
finally {
    $env:PATH = $script:OriginalPath
    if ($null -ne $script:Process -and -not $script:Process.HasExited) {
        Stop-Process -Id $script:Process.Id -Force
    }

    $tempPrefix = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    $testRootFullPath = [System.IO.Path]::GetFullPath($script:TestRoot)
    if (-not $testRootFullPath.StartsWith($tempPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "The native UI test directory escaped the system temporary directory."
    }
    if (Test-Path -LiteralPath $testRootFullPath) {
        Remove-Item -LiteralPath $testRootFullPath -Recurse -Force
    }
}
