param(
  [string]$ExtensionId = "YOUR_EXTENSION_ID",
  [string]$HostBinary = "$PWD\target\debug\silo-native-host",
  [string]$VaultPath = "$HOME\.local\share\silo\silo.vault"
)

$hostDir = Join-Path $env:LOCALAPPDATA "Google\Chrome\User Data\NativeMessagingHosts"
New-Item -ItemType Directory -Force -Path $hostDir | Out-Null
$launcher = Join-Path $hostDir "silo-native-host-launcher.cmd"
"@echo off`r`n\"$((Resolve-Path $HostBinary).Path)\" --vault \"$VaultPath\"`r`n" | Set-Content -Encoding ASCII $launcher
$manifest = @{
  name = "com.silo.native"
  description = "Silo native messaging host"
  path = $launcher
  type = "stdio"
  allowed_origins = @("chrome-extension://$ExtensionId/")
} | ConvertTo-Json
$manifest | Set-Content -Encoding UTF8 (Join-Path $hostDir "com.silo.native.json")
Write-Host "Installed com.silo.native at $hostDir"
Write-Host "The host currently defaults to $VaultPath; update the manifest or launch configuration if needed."
