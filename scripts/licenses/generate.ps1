$ErrorActionPreference = "Stop"

$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
if ($null -eq $cargoCommand) {
    $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
    if (-not (Test-Path -LiteralPath (Join-Path $cargoBin "cargo.exe") -PathType Leaf)) {
        throw "Cargo was not found. Install Rust stable and ensure cargo is available on PATH."
    }
    $env:PATH = "$cargoBin;$env:PATH"
    $cargoCommand = Get-Command cargo -ErrorAction Stop
}
$cargoExecutable = $cargoCommand.Source

$expectedVersion = "cargo-about 0.9.3"
$versionOutput = (& $cargoExecutable about --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $versionOutput -ne $expectedVersion) {
    throw "Expected $expectedVersion. Install it with: cargo install --locked --version 0.9.3 --features cli cargo-about"
}

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
Push-Location $repositoryRoot
try {
    & $cargoExecutable about generate `
        --workspace `
        --locked `
        --fail `
        --config licenses/about.toml `
        licenses/about.hbs `
        --output-file THIRD-PARTY-NOTICES.html
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-about failed with exit code $LASTEXITCODE."
    }

    $noticePath = Join-Path $repositoryRoot "THIRD-PARTY-NOTICES.html"
    $notice = [System.IO.File]::ReadAllText($noticePath)
    $notice = $notice.Replace("`r`n", "`n").Replace("`r", "`n")
    [System.IO.File]::WriteAllText($noticePath, $notice, [System.Text.UTF8Encoding]::new($false))
} finally {
    Pop-Location
}
