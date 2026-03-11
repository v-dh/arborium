param(
  [Parameter(Mandatory = $true)][string]$Tag,
  [Parameter(Mandatory = $true)][string]$TargetTriple,
  [Parameter(Mandatory = $true)][string]$BinaryPath,
  [Parameter(Mandatory = $true)][string]$OutputDir
)

$ErrorActionPreference = 'Stop'
$AppName = 'Arbor'
$StagingDir = Join-Path $OutputDir "$AppName-$Tag-$TargetTriple"
$ArchivePath = Join-Path $OutputDir "$AppName-$Tag-$TargetTriple.zip"

$BinDir = Join-Path $StagingDir 'bin'
$ShareDir = Join-Path $StagingDir 'share\arbor'
New-Item -Path $BinDir -ItemType Directory -Force | Out-Null
New-Item -Path $ShareDir -ItemType Directory -Force | Out-Null
Copy-Item -Path $BinaryPath -Destination (Join-Path $BinDir "$AppName.exe") -Force
Copy-Item -Path README.md -Destination (Join-Path $StagingDir 'README.md') -Force

# Bundle arbor-httpd alongside the main binary
$HttpdPath = Join-Path (Split-Path $BinaryPath) 'arbor-httpd.exe'
if (Test-Path $HttpdPath) {
  Copy-Item -Path $HttpdPath -Destination (Join-Path $BinDir 'arbor-httpd.exe') -Force
  Write-Output "bundled arbor-httpd from $HttpdPath"
}

# Bundle arbor-mcp alongside the main binary
$McpPath = Join-Path (Split-Path $BinaryPath) 'arbor-mcp.exe'
if (Test-Path $McpPath) {
  Copy-Item -Path $McpPath -Destination (Join-Path $BinDir 'arbor-mcp.exe') -Force
  Write-Output "bundled arbor-mcp from $McpPath"
}

# Bundle arbor CLI for scripting and automation
$CliPath = Join-Path (Split-Path $BinaryPath) 'arbor.exe'
if (Test-Path $CliPath) {
  Copy-Item -Path $CliPath -Destination (Join-Path $BinDir 'arbor.exe') -Force
  Write-Output "bundled arbor CLI from $CliPath"
}

# Bundle web UI assets
$WebUiDist = Join-Path $PSScriptRoot '..\..\crates\arbor-web-ui\app\dist'
if (Test-Path $WebUiDist) {
  Copy-Item -Path $WebUiDist -Destination (Join-Path $ShareDir 'web-ui') -Recurse -Force
  Write-Output "bundled web-ui assets from $WebUiDist"
}

# Bundle icon assets used by the GUI
$IconsDir = Join-Path $PSScriptRoot '..\..\assets\icons'
if (Test-Path $IconsDir) {
  Copy-Item -Path $IconsDir -Destination (Join-Path $ShareDir 'icons') -Recurse -Force
  Write-Output "bundled icon assets from $IconsDir"
}

if (Test-Path $ArchivePath) {
  Remove-Item -Path $ArchivePath -Force
}
Compress-Archive -Path (Join-Path $StagingDir '*') -DestinationPath $ArchivePath

Write-Output $ArchivePath
