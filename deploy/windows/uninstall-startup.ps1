$ErrorActionPreference = 'Stop'
Stop-ScheduledTask -TaskName 'WorkspaceKnowledgeAgent' -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName 'WorkspaceKnowledgeAgent' -Confirm:$false
# User data, credential entries and agent client configuration are intentionally preserved.
