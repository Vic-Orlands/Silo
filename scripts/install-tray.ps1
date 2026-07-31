param(
  [string]$VaultPath = "$HOME\silo.vault",
  [string]$TrayBinary = "",
  [string]$SiloBinary = "",
  [int]$Timeout = 900
)

if (-not $TrayBinary) {
  $command = Get-Command silo-tray -ErrorAction SilentlyContinue
  if ($command) { $TrayBinary = $command.Source }
}
if (-not $SiloBinary) {
  $command = Get-Command silo -ErrorAction SilentlyContinue
  if ($command) { $SiloBinary = $command.Source }
}
if (-not $TrayBinary -or -not (Test-Path $TrayBinary)) { throw "silo-tray binary not found." }
if (-not $SiloBinary -or -not (Test-Path $SiloBinary)) { throw "Silo CLI binary not found." }

$taskName = "Silo Tray"
$arguments = "--vault `"$VaultPath`" --cli `"$SiloBinary`" --timeout $Timeout"
$action = New-ScheduledTaskAction -Execute $TrayBinary -Argument $arguments
$trigger = New-ScheduledTaskTrigger -AtLogOn
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -RunLevel Limited -Force | Out-Null
Start-ScheduledTask -TaskName $taskName
Write-Output "Installed Silo tray companion for $VaultPath"
