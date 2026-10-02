param(
  [Parameter(Mandatory=$true)][string]$AgentPath,
  [Parameter(Mandatory=$true)][string]$StateDirectory
)
$ErrorActionPreference = 'Stop'
$agent = (Resolve-Path -LiteralPath $AgentPath).Path
$state = (Resolve-Path -LiteralPath $StateDirectory).Path
if ($agent.Contains('"') -or $state.Contains('"')) { throw 'Invalid path' }
# Current user interactive logon only. Never SYSTEM or highest privileges.
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$action = New-ScheduledTaskAction -Execute $agent -Argument ('--state-dir "' + $state + '" serve')
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName 'WorkspaceKnowledgeAgent' -Action $action -Trigger $trigger -Principal $principal
