param(
  [string]$VaultPath = "$HOME\silo.vault",
  [string]$SiloBinary = "",
  [int]$Timeout = 900
)

if (-not $SiloBinary) {
  $command = Get-Command silo -ErrorAction SilentlyContinue
  if ($command) { $SiloBinary = $command.Source }
}

if (-not $SiloBinary -or -not (Test-Path $SiloBinary)) {
  throw "Silo CLI not found. Pass -SiloBinary with the installed silo executable."
}

$taskName = "Silo Broker"
$arguments = "--vault `"$VaultPath`" broker --background --timeout $Timeout"
$action = New-ScheduledTaskAction -Execute $SiloBinary -Argument $arguments
$trigger = New-ScheduledTaskTrigger -AtLogOn
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -RunLevel Limited -Force | Out-Null
Start-ScheduledTask -TaskName $taskName
Write-Output "Installed Silo broker scheduled task for $VaultPath"
