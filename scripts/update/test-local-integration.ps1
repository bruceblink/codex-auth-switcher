[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$targetRoot = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot "target\update-integration"))
$runId = [guid]::NewGuid().ToString("N")
$runRoot = Join-Path $targetRoot $runId
$containerName = "codex-auth-switcher-update-$runId"
$backendContainerName = "$containerName-backend"
$networkName = "$containerName-network"
$image = "nginx:1.27-alpine@sha256:65645c7bb6a0661892a8b03b89d0743208a18dd2f3f17a54ef4b76fb8e2f2a10"
$backendImage = "python:3.13.18-alpine3.23@sha256:e4aeaca9081549f19bd5bc8605ed5ab257ba34f97e0471f8e09161f45b4b08b8"
$containerCreated = $false
$backendContainerCreated = $false
$networkCreated = $false
$integrationPassed = $false
$dockerMode = $null
$dockerExecutable = $null
$wslExecutable = $null
$savedEnvironment = @{}
$testEnvironmentNames = @(
    "UPDATE_TEST_ROOT",
    "UPDATE_TEST_CACHE",
    "UPDATE_TEST_TLS_CERT",
    "UPDATE_TEST_HTTPS_PORT",
    "UPDATE_TEST_BACKEND_MODE_FILE",
    "UPDATE_TEST_RELEASE_VERSION"
)

function Get-AvailablePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    try {
        $listener.Start()
        return [int]$listener.LocalEndpoint.Port
    }
    finally {
        $listener.Stop()
    }
}

function Convert-ToWslPath([string]$Path) {
    $converted = & $script:wslExecutable --exec wslpath -a $Path
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace(($converted -join ""))) {
        throw "Could not convert a temporary test path for WSL."
    }
    return ([string]($converted -join "")).Trim()
}

function Convert-ToDockerPath([string]$Path) {
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    if ($script:dockerMode -eq "wsl") {
        return Convert-ToWslPath $fullPath
    }
    if ($IsWindows) {
        return $fullPath.Replace("\", "/")
    }
    return $fullPath
}

function Invoke-Docker([string[]]$Arguments) {
    if ($script:dockerMode -eq "native") {
        & $script:dockerExecutable @Arguments
    }
    else {
        & $script:wslExecutable --exec docker @Arguments
    }
    if ($LASTEXITCODE -ne 0) {
        throw "Docker command failed with exit code $LASTEXITCODE."
    }
}

function Invoke-OpenSsl([string[]]$Arguments) {
    $openssl = Get-Command openssl -ErrorAction SilentlyContinue
    if ($null -ne $openssl) {
        & $openssl.Source @Arguments
    }
    elseif ($IsWindows -and $null -ne $script:wslExecutable) {
        $translated = @()
        foreach ($argument in $Arguments) {
            if ($argument -eq $script:certificatePath -or $argument -eq $script:privateKeyPath) {
                $translated += Convert-ToWslPath $argument
            }
            else {
                $translated += $argument
            }
        }
        & $script:wslExecutable --exec openssl @translated
    }
    else {
        throw "OpenSSL is required to create the temporary HTTPS certificate."
    }
    if ($LASTEXITCODE -ne 0) {
        throw "OpenSSL could not create the temporary HTTPS certificate."
    }
}

function Get-WorkspaceVersion {
    $inWorkspacePackage = $false
    foreach ($line in Get-Content -LiteralPath (Join-Path $repositoryRoot "Cargo.toml")) {
        if ($line -match '^\[workspace\.package\]$') {
            $inWorkspacePackage = $true
            continue
        }
        if ($line -match '^\[') {
            $inWorkspacePackage = $false
        }
        if ($inWorkspacePackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
            return [Version]::Parse($Matches[1])
        }
    }
    throw "Cargo.toml does not define a workspace package version."
}

function Test-TcpPort([int]$Port) {
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $client.Connect([System.Net.IPAddress]::Loopback, $Port)
        return $true
    }
    catch {
        return $false
    }
    finally {
        $client.Dispose()
    }
}

try {
    foreach ($name in $testEnvironmentNames) {
        $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
    }

    New-Item -ItemType Directory -Path $runRoot | Out-Null
    $webRoot = Join-Path $runRoot "www"
    New-Item -ItemType Directory -Path (Join-Path $webRoot "assets") -Force | Out-Null
    $script:certificatePath = Join-Path $runRoot "server.crt"
    $script:privateKeyPath = Join-Path $runRoot "server.key"
    $nginxConfigPath = Join-Path $runRoot "nginx.conf"
    $tlsPort = Get-AvailablePort

    $nativeDocker = Get-Command docker -ErrorAction SilentlyContinue
    if ($null -ne $nativeDocker) {
        & $nativeDocker.Source version --format '{{.Server.Version}}' 2>$null
        if ($LASTEXITCODE -eq 0) {
            $script:dockerMode = "native"
            $script:dockerExecutable = $nativeDocker.Source
        }
    }
    if ($null -eq $script:dockerMode -and $IsWindows) {
        $script:wslExecutable = (Get-Command wsl.exe -ErrorAction SilentlyContinue).Source
        if (-not [string]::IsNullOrWhiteSpace($script:wslExecutable)) {
            & $script:wslExecutable --exec docker version --format '{{.Server.Version}}' 2>$null
            if ($LASTEXITCODE -eq 0) {
                $script:dockerMode = "wsl"
            }
        }
    }
    if ($null -eq $script:dockerMode) {
        throw "A running Docker engine is required for the update integration test."
    }
    $serverVersion = if ($script:dockerMode -eq "native") {
        & $script:dockerExecutable version --format '{{.Server.Version}}'
    }
    else {
        & $script:wslExecutable --exec docker version --format '{{.Server.Version}}'
    }
    if ($LASTEXITCODE -ne 0) {
        throw "The Docker server version could not be read."
    }

    $workspaceVersion = Get-WorkspaceVersion
    $releaseVersion = [Version]::new($workspaceVersion.Major, $workspaceVersion.Minor, $workspaceVersion.Build + 1)
    Invoke-OpenSsl @(
        "req",
        "-x509",
        "-newkey", "rsa:2048",
        "-nodes",
        "-sha256",
        "-days", "2",
        "-subj", "/CN=github.com",
        "-addext", "basicConstraints=critical,CA:FALSE",
        "-addext", "keyUsage=critical,digitalSignature,keyEncipherment",
        "-addext", "extendedKeyUsage=serverAuth",
        "-addext", "subjectAltName=DNS:github.com,DNS:release-assets.githubusercontent.com",
        "-keyout", $script:privateKeyPath,
        "-out", $script:certificatePath
    )

    $backendEndpoint = "http://update-backend:8080"
    $backendScriptPath = Join-Path $runRoot "backend.py"
    $backendScript = @'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import time

MODE_FILE = Path("/test/www/backend-mode.txt")
PACKAGE_FILE = Path("/test/www/assets/update-package.bin")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        if self.path == "/health":
            self.send_response(200)
            self.send_header("Content-Length", "2")
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(b"ok")
            self.close_connection = True
            return

        if not self.path.startswith("/assets/"):
            self.send_error(404)
            return

        try:
            mode = int(MODE_FILE.read_text(encoding="ascii"))
            package = PACKAGE_FILE.read_bytes()
        except (OSError, ValueError):
            self.send_error(503, "test backend is not ready")
            return

        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        if mode == 2 or mode == 3:
            self.send_header("Transfer-Encoding", "chunked")
        else:
            self.send_header("Content-Length", str(len(package)))
        self.send_header("Connection", "close")
        self.end_headers()

        if mode == 1:
            self.wfile.write(package[: len(package) // 2])
            self.wfile.flush()
        elif mode == 2:
            self.write_chunk(package + b"overflow")
            self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()
        elif mode == 3:
            first = len(package) // 4
            second = len(package) // 2
            self.write_chunk(package[:first])
            self.wfile.flush()
            time.sleep(0.25)
            self.write_chunk(package[first:second])
            self.wfile.flush()
            time.sleep(2)
            self.write_chunk(package[second:])
            self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()
        else:
            self.wfile.write(package)
            self.wfile.flush()
        self.close_connection = True

    def write_chunk(self, data):
        self.wfile.write(f"{len(data):X}\r\n".encode("ascii"))
        self.wfile.write(data)
        self.wfile.write(b"\r\n")


ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
'@
    $backendScript = $backendScript.Replace("`r`n", "`n")
    [System.IO.File]::WriteAllText(
        $backendScriptPath,
        $backendScript,
        [System.Text.UTF8Encoding]::new($false)
    )

    $nginxTemplate = @'
events {
    worker_connections 64;
}

http {
    server {
        listen 443 ssl;
        server_name github.com release-assets.githubusercontent.com;

        ssl_certificate /test/server.crt;
        ssl_certificate_key /test/server.key;

        location = /bruceblink/codex-auth-switcher/releases/latest {
            return 302 /bruceblink/codex-auth-switcher/releases/tag/v__VERSION__;
        }

        location = /bruceblink/codex-auth-switcher/releases/tag/v__VERSION__ {
            return 200 "test release";
        }

        location = /bruceblink/codex-auth-switcher/releases/download/v__VERSION__/update-manifest.json {
            alias /test/www/update-manifest.json;
        }

        location ~ ^/bruceblink/codex-auth-switcher/releases/download/v__VERSION__/(Codex-Auth-Switcher-[A-Za-z0-9._-]+)$ {
            return 302 https://release-assets.githubusercontent.com/assets/$1;
        }

        location /assets/ {
            proxy_buffering off;
            proxy_request_buffering off;
            proxy_read_timeout 10s;
            proxy_http_version 1.1;
            proxy_set_header Connection "";
            proxy_pass __BACKEND_ENDPOINT__;
        }
    }
}
'@
    $nginxConfig = $nginxTemplate.Replace("__VERSION__", $releaseVersion.ToString())
    $nginxConfig = $nginxConfig.Replace("__BACKEND_ENDPOINT__", $backendEndpoint)
    [System.IO.File]::WriteAllText(
        $nginxConfigPath,
        $nginxConfig,
        [System.Text.UTF8Encoding]::new($false)
    )

    $dockerRoot = Convert-ToDockerPath $runRoot
    $mount = "type=bind,source=$dockerRoot,target=/test,readonly"
    Invoke-Docker @("network", "create", $networkName)
    $networkCreated = $true

    $backendArguments = @(
        "run",
        "--detach",
        "--name", $backendContainerName,
        "--network", $networkName,
        "--network-alias", "update-backend",
        "--mount", $mount,
        $backendImage,
        "python3", "-u", "/test/backend.py"
    )
    Invoke-Docker $backendArguments
    $backendContainerCreated = $true

    $backendReady = $false
    $backendReadyDeadline = [DateTime]::UtcNow.AddSeconds(30)
    $backendReadyProbe = "import urllib.request; urllib.request.build_opener(urllib.request.ProxyHandler({})).open('http://127.0.0.1:8080/health', timeout=2)"
    while ([DateTime]::UtcNow -lt $backendReadyDeadline -and -not $backendReady) {
        try {
            Invoke-Docker @(
                "exec",
                $backendContainerName,
                "python3",
                "-c",
                $backendReadyProbe
            )
            $backendReady = $true
        }
        catch {
            Start-Sleep -Milliseconds 500
        }
    }
    if (-not $backendReady) {
        throw "The local Docker download backend did not become ready."
    }
    Write-Output "Docker download backend health probe: passed."

    $dockerArguments = @(
        "run",
        "--detach",
        "--name", $containerName,
        "--network", $networkName,
        "--publish", "127.0.0.1:${tlsPort}:443",
        "--mount", $mount,
        $image,
        "nginx", "-c", "/test/nginx.conf", "-g", "daemon off;"
    )
    Invoke-Docker $dockerArguments
    $containerCreated = $true

    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline -and -not (Test-TcpPort $tlsPort)) {
        Start-Sleep -Milliseconds 250
    }
    if (-not (Test-TcpPort $tlsPort)) {
        throw "The local Docker HTTPS service did not start listening."
    }
    $tlsProbe = Invoke-WebRequest `
        -Uri "https://127.0.0.1:${tlsPort}/bruceblink/codex-auth-switcher/releases/tag/v${releaseVersion}" `
        -Method Get `
        -TimeoutSec 10 `
        -SkipCertificateCheck
    if ($tlsProbe.StatusCode -ne 200 -or $tlsProbe.Content -ne "test release") {
        throw "The Windows HTTPS probe did not receive the expected local release response."
    }
    Write-Output "Windows HTTPS probe: HTTP $($tlsProbe.StatusCode) with certificate checking skipped for the temporary test certificate."

    $rustup = Get-Command rustup -ErrorAction SilentlyContinue
    if ($IsWindows) {
        $rustupPath = Join-Path $env:USERPROFILE ".cargo\bin\rustup.exe"
        if (Test-Path -LiteralPath $rustupPath -PathType Leaf) {
            $rustup = Get-Item -LiteralPath $rustupPath
        }
    }
    if ($null -eq $rustup) {
        throw "Rustup is required to run the update integration test on stable."
    }
    $rustupExecutable = if ($rustup.PSObject.Properties.Name -contains "Source") {
        $rustup.Source
    }
    else {
        $rustup.FullName
    }

    if ($IsWindows) {
        $savedGitHubEnv = [Environment]::GetEnvironmentVariable("GITHUB_ENV", "Process")
        $env:GITHUB_ENV = Join-Path $runRoot "msvc-environment.txt"
        try {
            & (Join-Path $repositoryRoot "scripts\enable-windows-msvc.ps1")
            if ($LASTEXITCODE -ne 0) {
                throw "The Windows MSVC environment could not be initialized."
            }
        }
        finally {
            [Environment]::SetEnvironmentVariable("GITHUB_ENV", $savedGitHubEnv, "Process")
        }
    }

    $releaseString = $releaseVersion.ToString()
    $testEnvironment = @{
        UPDATE_TEST_ROOT = $webRoot
        UPDATE_TEST_CACHE = Join-Path $runRoot "cache"
        UPDATE_TEST_TLS_CERT = $script:certificatePath
        UPDATE_TEST_HTTPS_PORT = [string]$tlsPort
        UPDATE_TEST_BACKEND_MODE_FILE = Join-Path $webRoot "backend-mode.txt"
        UPDATE_TEST_RELEASE_VERSION = $releaseString
    }
    foreach ($entry in $testEnvironment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, "Process")
    }

    Write-Output "Docker server: $($serverVersion -join '')."
    Write-Output "Docker images: $image; $backendImage."
    Write-Output "HTTPS port: $tlsPort; backend port: 8080 on the private Docker network."
    Write-Output "Disposable container: $containerName."
    Push-Location $repositoryRoot
    try {
        & $rustupExecutable run stable rustc -Vv
        if ($LASTEXITCODE -ne 0) {
            throw "Rust stable could not be inspected."
        }
        & $rustupExecutable run stable cargo test -p switcher-core --locked update::local_release_tests::docker_update_integration -- --ignored --exact --nocapture
        if ($LASTEXITCODE -ne 0) {
            throw "The Docker HTTPS update integration test failed."
        }
        $integrationPassed = $true
    }
    finally {
        Pop-Location
    }
}
finally {
    if (($containerCreated -or $backendContainerCreated) -and -not $integrationPassed) {
        Write-Output "Failed Docker integration container logs:"
        if ($backendContainerCreated) {
            if ($script:dockerMode -eq "native") {
                & $script:dockerExecutable logs $backendContainerName 2>&1
            }
            elseif ($script:dockerMode -eq "wsl") {
                & $script:wslExecutable --exec docker logs $backendContainerName 2>&1
            }
        }
        if ($containerCreated) {
            if ($script:dockerMode -eq "native") {
                & $script:dockerExecutable logs $containerName 2>&1
            }
            elseif ($script:dockerMode -eq "wsl") {
                & $script:wslExecutable --exec docker logs $containerName 2>&1
            }
        }
    }

    foreach ($name in $testEnvironmentNames) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
    }

    if ($containerCreated -and $containerName -match '^codex-auth-switcher-update-[0-9a-f]{32}$') {
        if ($script:dockerMode -eq "native") {
            & $script:dockerExecutable rm --force $containerName 2>$null
        }
        elseif ($script:dockerMode -eq "wsl") {
            & $script:wslExecutable --exec docker rm --force $containerName 2>$null
        }
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "The disposable Docker container did not clean up successfully: $containerName"
        }
        else {
            Write-Output "Removed disposable container: $containerName."
        }
    }

    if ($backendContainerCreated -and $backendContainerName -match '^codex-auth-switcher-update-[0-9a-f]{32}-backend$') {
        if ($script:dockerMode -eq "native") {
            & $script:dockerExecutable rm --force $backendContainerName 2>$null
        }
        elseif ($script:dockerMode -eq "wsl") {
            & $script:wslExecutable --exec docker rm --force $backendContainerName 2>$null
        }
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "The disposable Docker backend container did not clean up successfully: $backendContainerName"
        }
        else {
            Write-Output "Removed disposable backend container: $backendContainerName."
        }
    }

    if ($networkCreated -and $networkName -match '^codex-auth-switcher-update-[0-9a-f]{32}-network$') {
        if ($script:dockerMode -eq "native") {
            & $script:dockerExecutable network rm $networkName 2>$null
        }
        elseif ($script:dockerMode -eq "wsl") {
            & $script:wslExecutable --exec docker network rm $networkName 2>$null
        }
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "The disposable Docker network did not clean up successfully: $networkName"
        }
        else {
            Write-Output "Removed disposable Docker network: $networkName."
        }
    }

    $resolvedTargetRoot = [System.IO.Path]::GetFullPath($targetRoot).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
    $resolvedRunRoot = [System.IO.Path]::GetFullPath($runRoot)
    if ($resolvedRunRoot.StartsWith($resolvedTargetRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $resolvedRunRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
