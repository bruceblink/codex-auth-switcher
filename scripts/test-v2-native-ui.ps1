param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot "..\target\release\codex-auth-switcher.exe"),
    [ValidateRange(0, 120)]
    [int]$HoldWindowSeconds = 0
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "The V2 executable was not found."
}

$script:ExecutablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
if ([System.IO.Path]::GetExtension($script:ExecutablePath) -ne ".exe") {
    throw "The native UI test requires a Windows executable, not a ZIP archive."
}
$executableBytes = [System.IO.File]::ReadAllBytes($script:ExecutablePath)
$peOffset = [BitConverter]::ToInt32($executableBytes, 0x3c)
if ([System.Text.Encoding]::ASCII.GetString($executableBytes, $peOffset, 4) -ne "PE`0`0") {
    throw "The native UI test executable has an invalid PE header."
}
$optionalHeaderOffset = $peOffset + 24
$optionalHeaderMagic = [BitConverter]::ToUInt16($executableBytes, $optionalHeaderOffset)
if ($optionalHeaderMagic -ne 0x10b -and $optionalHeaderMagic -ne 0x20b) {
    throw "The native UI test executable has an unsupported PE header."
}
$subsystem = [BitConverter]::ToUInt16($executableBytes, $optionalHeaderOffset + 68)
if ($subsystem -ne 2) {
    throw "The Release executable must use the Windows GUI subsystem."
}
$script:TestRoot = Join-Path ([System.IO.Path]::GetTempPath()) "codex-auth-switcher-native-ui-$([guid]::NewGuid().ToString('N'))"
$script:CodexHome = Join-Path $script:TestRoot "codex-home"
$script:OriginalPath = $env:PATH
$script:EnvironmentNames = @(
    "CODEX_HOME",
    "AUTOCODE_API_KEY",
    "AUTOCODE_BASE_URL",
    "CODEX_API_PROVIDER",
    "CODEX_MODEL_PROVIDER"
)
$script:OriginalEnvironment = @{}
foreach ($name in $script:EnvironmentNames) {
    $script:OriginalEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}
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

function Wait-ForButtonDisabled([string]$Name) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline) {
        $button = Find-Button $Name
        if ($null -ne $button -and -not $button.Current.IsEnabled) {
            return $button
        }
        Start-Sleep -Milliseconds 100
    }
    throw "The native GUI button '$Name' did not become disabled after saving."
}

function Find-TextInput([string]$Name) {
    $window = Get-MainWindow
    if ($null -eq $window) {
        return $null
    }

    $editCondition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit
    )
    $inputs = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $editCondition)
    foreach ($input in $inputs) {
        if ($input.Current.Name -eq $Name) {
            return $input
        }
    }
    return $null
}

function Wait-ForTextInput([string]$Name) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline) {
        $input = Find-TextInput $Name
        if ($null -ne $input -and $input.Current.IsEnabled) {
            return $input
        }
        Start-Sleep -Milliseconds 100
    }
    throw "The native GUI text input '$Name' did not become available."
}

function Set-TextInputValue([string]$Name, [string]$Value) {
    $input = Wait-ForTextInput $Name
    $pattern = $input.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
    $pattern.SetValue($Value)
}

function Click-NativeButton([string]$Name) {
    $button = Wait-ForButton $Name
    $window = Get-MainWindow
    $windowHandle = [IntPtr]$window.Current.NativeWindowHandle
    # SW_RESTORE keeps acceptance at the app's normal window size instead of hiding layout issues by maximizing it.
    $null = [NativeUiInput]::ShowWindow($windowHandle, 9)
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

function Read-ActiveFiles {
    $configPath = Join-Path $script:CodexHome "config.toml"
    $authPath = Join-Path $script:CodexHome "auth.json"
    if (-not (Test-Path -LiteralPath $configPath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $authPath -PathType Leaf)) {
        return $null
    }

    $config = Get-Content -LiteralPath $configPath -Raw
    $auth = Get-Content -LiteralPath $authPath -Raw | ConvertFrom-Json
    $mode = if ($auth.auth_mode -eq "chatgpt") {
        "subscription"
    }
    elseif (-not [string]::IsNullOrWhiteSpace([string]$auth.OPENAI_API_KEY)) {
        "api"
    }
    else {
        "unknown"
    }
    $modelMatch = [regex]::Match($config, '(?m)^model\s*=\s*"([^"]+)"')
    $model = if ($modelMatch.Success) { $modelMatch.Groups[1].Value } else { "" }
    return [pscustomobject]@{ mode = $mode; auth_mode = $mode; model = $model }
}

function Wait-ForMode([string]$Mode) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ([DateTime]::UtcNow -lt $deadline) {
        $status = Read-ActiveFiles
        if ($null -ne $status -and $status.mode -eq $Mode -and $status.auth_mode -eq $Mode) {
            return $status
        }
        Start-Sleep -Milliseconds 100
    }
    throw "The native GUI did not apply mode '$Mode'."
}

try {
    New-Item -ItemType Directory -Path $script:TestRoot | Out-Null
    New-Item -ItemType Directory -Path $script:CodexHome | Out-Null

    # Seed only synthetic active files and backups; start the release GUI without arguments.
    $subscriptionConfig = @'
model_provider = "openai"
model = "demo-subscription-model"
model_reasoning_effort = "medium"
cli_auth_credentials_store = "file"

[features]
multi_agent = true

[projects."F:/example"]
trust_level = "trusted"
'@
    $apiConfig = @'
model_provider = "SyntheticApi"
model = "demo-api-model"
model_reasoning_effort = "xhigh"

[model_providers.SyntheticApi]
name = "Synthetic gateway"
base_url = "https://gateway.example.invalid/v1"
wire_api = "responses"
requires_openai_auth = true
'@
    $subscriptionAuth = @{
        auth_mode = "chatgpt"
        tokens = @{
            access_token = "synthetic-access-token"
            refresh_token = "synthetic-refresh-token"
            id_token = "synthetic-id-token"
            account_id = "synthetic-account"
        }
    } | ConvertTo-Json -Depth 4 -Compress
    [System.IO.File]::WriteAllText((Join-Path $script:CodexHome "config.toml"), $subscriptionConfig, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText((Join-Path $script:CodexHome "auth.json"), $subscriptionAuth, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText((Join-Path $script:CodexHome "config.toml.bak-synthetic-api"), $apiConfig, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText((Join-Path $script:CodexHome "auth.json.bak-synthetic-api"), '{"OPENAI_API_KEY":"synthetic-api-key"}', [System.Text.UTF8Encoding]::new($false))
    $modelCatalog = @{
        catalog_marker = "native-ui-synthetic"
        models = @(
            @{
                slug = "synthetic-model-alpha"
                display_name = "Synthetic Alpha"
                visibility = "list"
                default_reasoning_level = "high"
                supported_reasoning_levels = @(
                    @{ effort = "high"; description = "Synthetic high" },
                    @{ effort = "low"; description = "Synthetic low" }
                )
                priority = 10
                context_window = 65536
            }
        )
    } | ConvertTo-Json -Depth 10
    [System.IO.File]::WriteAllText((Join-Path $script:CodexHome "models-codex.json"), $modelCatalog, [System.Text.UTF8Encoding]::new($false))

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

    # Keep host credentials and API settings out of the isolated native-window test.
    foreach ($name in $script:EnvironmentNames) {
        [Environment]::SetEnvironmentVariable($name, $null, "Process")
    }
    $env:CODEX_HOME = $script:CodexHome
    $env:PATH = Join-Path $env:SystemRoot "System32"
    $script:Process = Start-Process -FilePath $script:ExecutablePath -PassThru

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

    # The synthetic active profile starts in subscription mode, so switch through the real controls.
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
        throw "The native GUI did not restore the synthetic subscription model."
    }

    $config = Get-Content -LiteralPath (Join-Path $script:CodexHome "config.toml") -Raw
    $auth = Get-Content -LiteralPath (Join-Path $script:CodexHome "auth.json") -Raw | ConvertFrom-Json
    if (-not $config.Contains("[features]") -or -not $config.Contains('[projects."F:/example"]')) {
        throw "The native GUI did not preserve shared Codex settings."
    }
    if ($auth.auth_mode -ne "chatgpt") {
        throw "The native GUI did not restore subscription authentication."
    }

    Click-NativeButton "模型目录"
    Click-NativeButton "读取 models-codex.json"
    $null = Wait-ForButton "复制当前模型"
    Click-NativeButton "复制当前模型"
    Set-TextInputValue "模型 ID" "native-ui-cloned-model"
    Set-TextInputValue "显示名称" "Native UI Cloned Model"
    Click-NativeButton "保存模型选项"
    $null = Wait-ForButtonDisabled "保存模型选项"

    $savedCatalog = Get-Content -LiteralPath (Join-Path $script:CodexHome "models-codex.json") -Raw | ConvertFrom-Json
    if ($savedCatalog.catalog_marker -ne "native-ui-synthetic" -or
        $savedCatalog.models.Count -ne 2 -or
        $savedCatalog.models[1].slug -ne "native-ui-cloned-model" -or
        $savedCatalog.models[1].display_name -ne "Native UI Cloned Model" -or
        $savedCatalog.models[1].context_window -ne 65536 -or
        $savedCatalog.models[1].default_reasoning_level -ne "high") {
        throw "The native GUI did not save the cloned model while preserving unknown metadata."
    }
    if (-not (Get-ChildItem -LiteralPath $script:CodexHome -Filter "models-codex.json.bak-*" -File)) {
        throw "The native GUI did not back up the previous model catalog."
    }

    Write-Output "V2 native Windows UI acceptance passed: initialize, target selection, preview, API apply, subscription restore, shared-setting preservation, and model catalog load/clone/edit/save/backup."
    if ($HoldWindowSeconds -gt 0) {
        Write-Output "Keeping the isolated native test window open for $HoldWindowSeconds seconds for visual inspection."
        Start-Sleep -Seconds $HoldWindowSeconds
    }
}
finally {
    $env:PATH = $script:OriginalPath
    foreach ($name in $script:EnvironmentNames) {
        [Environment]::SetEnvironmentVariable($name, $script:OriginalEnvironment[$name], "Process")
    }
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
