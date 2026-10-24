param(
    [string]$ExecutablePath = (Join-Path $PSScriptRoot "..\target\release\codex-auth-switcher.exe")
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "The V2 executable was not found."
}
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) "codex-auth-switcher-v2-headless-$([guid]::NewGuid().ToString('N'))"
$codexHome = Join-Path $testRoot "codex-home"
$sources = Join-Path $testRoot "sources"
$syntheticKey = "synthetic-headless-override-key"
$capturedOutput = [System.Collections.Generic.List[string]]::new()
$originalPath = $env:PATH
$summary = $null

function Assert-That {
    param(
        [bool]$Condition,
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Invoke-Switcher {
    param(
        [string[]]$Arguments,
        [AllowNull()]
        [string]$InputText
    )

    if ($null -eq $InputText) {
        $lines = & $script:executable @Arguments
    }
    else {
        $lines = $InputText | & $script:executable @Arguments
    }
    if ($LASTEXITCODE -ne 0) {
        throw "The packaged CLI returned a nonzero exit code."
    }

    $text = $lines -join "`n"
    foreach ($sentinel in @($script:syntheticKey, "synthetic-import-api-key", "synthetic-access-token")) {
        if ($text.Contains($sentinel)) {
            throw "The CLI output exposed a synthetic credential."
        }
    }
    $script:capturedOutput.Add($text)
    return $text
}

New-Item -ItemType Directory -Path $testRoot | Out-Null
try {
    # Run the packaged process without Rust, Python or package-manager paths.
    $env:PATH = Join-Path $env:SystemRoot "System32"
    New-Item -ItemType Directory -Path $sources | Out-Null
    $subscriptionConfig = Join-Path $sources "subscription.toml"
    $subscriptionAuth = Join-Path $sources "subscription-auth.json"
    $apiConfig = Join-Path $sources "api.toml"
    $apiAuth = Join-Path $sources "api-auth.json"

    [System.IO.File]::WriteAllText(
        $subscriptionConfig,
        "model_provider = `"openai`"`nmodel = `"import-subscription-model`"`nmodel_reasoning_effort = `"medium`"`ncli_auth_credentials_store = `"file`"`n",
        [System.Text.UTF8Encoding]::new($false)
    )
    [System.IO.File]::WriteAllText(
        $subscriptionAuth,
        '{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-access-token","refresh_token":"synthetic-refresh-token","id_token":"synthetic-id-token","account_id":"synthetic-account"}}',
        [System.Text.UTF8Encoding]::new($false)
    )
    [System.IO.File]::WriteAllText(
        $apiConfig,
        "model_provider = `"CustomApi`"`nmodel = `"import-api-model`"`nmodel_reasoning_effort = `"high`"`n[model_providers.CustomApi]`nname = `"Synthetic gateway`"`nbase_url = `"https://gateway.example.invalid/v1`"`nwire_api = `"responses`"`nrequires_openai_auth = true`n",
        [System.Text.UTF8Encoding]::new($false)
    )
    [System.IO.File]::WriteAllText(
        $apiAuth,
        '{"OPENAI_API_KEY":"synthetic-import-api-key"}',
        [System.Text.UTF8Encoding]::new($false)
    )

    $initialize = Invoke-Switcher -Arguments @(
        "--demo", "--isolated", "--codex-home", $codexHome, "--command", "initialize",
        "--api-config", $apiConfig, "--api-auth", $apiAuth
    ) -InputText $null | ConvertFrom-Json
    Assert-That ($initialize.initialized -and $initialize.modes.Count -eq 2) "Initialization did not save both modes."
    Assert-That ($initialize.mode -eq "subscription") "Initialization changed the active mode."
    $apiProfile = @($initialize.modes | Where-Object { $_.mode -eq "api" })[0]
    $subscriptionProfile = @($initialize.modes | Where-Object { $_.mode -eq "subscription" })[0]
    Assert-That ($apiProfile.ready -and $apiProfile.provider -eq "CustomApi") "Imported API profile is not ready."
    Assert-That ($subscriptionProfile.ready -and $subscriptionProfile.model -eq "demo-subscription-model") "The active subscription profile was not captured."
    Assert-That ($apiProfile.config_source -eq "api.toml" -and $apiProfile.auth_source -eq "api-auth.json") "API import sources were not recorded."

    $configPath = Join-Path $codexHome "config.toml"
    $authPath = Join-Path $codexHome "auth.json"
    $configHashBeforePreview = (Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash
    $authHashBeforePreview = (Get-FileHash -LiteralPath $authPath -Algorithm SHA256).Hash
    $previewText = Invoke-Switcher -Arguments @(
        "--isolated", "--codex-home", $codexHome, "--command", "preview", "--target", "api"
    ) -InputText $null
    $preview = $previewText | ConvertFrom-Json
    Assert-That ($preview.target -eq "api" -and $preview.auth_changes) "API preview omitted the mode or authentication change."
    Assert-That (($preview.changes | Where-Object { $_.field -eq "model_provider" }).after -eq "CustomApi") "Preview omitted the imported provider."
    Assert-That ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -eq $configHashBeforePreview) "Preview changed config.toml."
    Assert-That ((Get-FileHash -LiteralPath $authPath -Algorithm SHA256).Hash -eq $authHashBeforePreview) "Preview changed auth.json."

    $apiSwitch = Invoke-Switcher -Arguments @(
        "--isolated", "--codex-home", $codexHome, "--command", "switch", "--target", "api", "--api-key-stdin"
    ) -InputText $syntheticKey | ConvertFrom-Json
    Assert-That ($apiSwitch.target -eq "api" -and $apiSwitch.changed -and $apiSwitch.restart_required) "API switch did not complete."
    $apiStatus = Invoke-Switcher -Arguments @(
        "--isolated", "--codex-home", $codexHome, "--command", "status"
    ) -InputText $null | ConvertFrom-Json
    Assert-That ($apiStatus.mode -eq "api" -and $apiStatus.auth_mode -eq "api") "API switch status is incorrect."
    $activeApiAuth = Get-Content -LiteralPath $authPath -Raw | ConvertFrom-Json
    Assert-That ($activeApiAuth.OPENAI_API_KEY -eq $syntheticKey) "API switch did not write the provided stdin key."

    $configHashBeforeImport = (Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash
    $authHashBeforeImport = (Get-FileHash -LiteralPath $authPath -Algorithm SHA256).Hash
    $subscriptionInitialize = Invoke-Switcher -Arguments @(
        "--isolated", "--codex-home", $codexHome, "--command", "initialize",
        "--subscription-config", $subscriptionConfig, "--subscription-auth", $subscriptionAuth
    ) -InputText $null | ConvertFrom-Json
    $subscriptionProfile = @($subscriptionInitialize.modes | Where-Object { $_.mode -eq "subscription" })[0]
    Assert-That ($subscriptionProfile.ready -and $subscriptionProfile.model -eq "import-subscription-model") "Imported subscription profile is not ready."
    Assert-That ($subscriptionProfile.config_source -eq "subscription.toml" -and $subscriptionProfile.auth_source -eq "subscription-auth.json") "Subscription import sources were not recorded."
    Assert-That ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -eq $configHashBeforeImport) "Subscription import changed config.toml."
    Assert-That ((Get-FileHash -LiteralPath $authPath -Algorithm SHA256).Hash -eq $authHashBeforeImport) "Subscription import changed auth.json."

    $subscriptionSwitch = Invoke-Switcher -Arguments @(
        "--isolated", "--codex-home", $codexHome, "--command", "switch", "--target", "subscription"
    ) -InputText $null | ConvertFrom-Json
    Assert-That ($subscriptionSwitch.target -eq "subscription" -and $subscriptionSwitch.changed) "Subscription switch did not complete."
    $subscriptionStatus = Invoke-Switcher -Arguments @(
        "--isolated", "--codex-home", $codexHome, "--command", "status"
    ) -InputText $null | ConvertFrom-Json
    Assert-That ($subscriptionStatus.mode -eq "subscription" -and $subscriptionStatus.auth_mode -eq "subscription") "Subscription switch status is incorrect."
    $activeSubscriptionAuth = Get-Content -LiteralPath $authPath -Raw | ConvertFrom-Json
    Assert-That ($activeSubscriptionAuth.auth_mode -eq "chatgpt" -and $activeSubscriptionAuth.tokens.access_token -eq "synthetic-access-token") "Subscription credentials were not restored."
    $subscriptionConfigText = Get-Content -LiteralPath $configPath -Raw
    Assert-That ($subscriptionConfigText.Contains('model = "import-subscription-model"')) "Imported subscription model was not applied."

    $apiAgain = Invoke-Switcher -Arguments @(
        "--isolated", "--codex-home", $codexHome, "--command", "switch", "--target", "api"
    ) -InputText $null | ConvertFrom-Json
    Assert-That ($apiAgain.target -eq "api" -and $apiAgain.changed) "Saved API profile could not be restored."
    $activeConfig = Get-Content -LiteralPath $configPath -Raw
    Assert-That ($activeConfig.Contains('model_provider = "CustomApi"') -and $activeConfig.Contains('model = "import-api-model"')) "API model settings were not restored."
    $restoredApiAuth = Get-Content -LiteralPath $authPath -Raw | ConvertFrom-Json
    Assert-That ($restoredApiAuth.OPENAI_API_KEY -eq $syntheticKey) "Saved API credentials were not restored."
    Assert-That ($activeConfig.Contains("[features]") -and $activeConfig.Contains('[projects."F:/example"]') -and $activeConfig.Contains('trust_level = "trusted"')) "Shared Codex settings were not preserved."
    Assert-That (-not (($script:capturedOutput -join "`n").Contains($syntheticKey))) "A CLI response exposed the stdin key."

    $summary = "V2 headless CLI acceptance passed: both mode imports, redacted/no-write previews, UI-independent API/subscription round trip, credential restore, and shared-setting preservation."
}
finally {
    $env:PATH = $originalPath
    $tempPrefix = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    $testRootFullPath = [System.IO.Path]::GetFullPath($testRoot)
    if (-not $testRootFullPath.StartsWith($tempPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "The headless test directory escaped the system temporary directory."
    }
    Remove-Item -LiteralPath $testRootFullPath -Recurse -Force
}

Write-Output $summary
