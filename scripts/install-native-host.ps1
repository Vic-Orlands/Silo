param(
  [string]$ExtensionId = "YOUR_EXTENSION_ID",
  [string]$HostBinary = "$PWD\target\debug\silo-native-host",
  [string]$SiloBinary = "",
  [ValidateSet("chrome", "firefox")][string]$Browser = "chrome"
)

$hostDir = if ($Browser -eq "firefox") {
  Join-Path $env:APPDATA "Mozilla\NativeMessagingHosts"
} else {
  Join-Path $env:LOCALAPPDATA "Google\Chrome\User Data\NativeMessagingHosts"
}
New-Item -ItemType Directory -Force -Path $hostDir | Out-Null
$launcher = Join-Path $hostDir "silo-native-host-launcher.cmd"
$binaryPath = (Resolve-Path $HostBinary).Path
$cliPath = if ($SiloBinary) { (Resolve-Path $SiloBinary).Path } else { (Get-Command silo -ErrorAction SilentlyContinue).Source }
$launcherContents = "@echo off`r`n"
if ($cliPath) { $launcherContents += "set SILO_BIN=$cliPath`r`n" }
$launcherContents += "`"$binaryPath`"`r`n"
$launcherContents | Set-Content -Encoding ASCII $launcher
$manifest = @{
  name = "com.silo.native"
  description = "Silo native messaging host"
  path = $launcher
  type = "stdio"
}
if ($Browser -eq "firefox") {
  $manifest.allowed_extensions = @($ExtensionId)
} else {
  $manifest.allowed_origins = @("chrome-extension://$ExtensionId/")
}
$manifest = $manifest | ConvertTo-Json
$manifest | Set-Content -Encoding UTF8 (Join-Path $hostDir "com.silo.native.json")
Write-Host "Installed com.silo.native at $hostDir"
Write-Host "The broker must be running before browser requests are made."
