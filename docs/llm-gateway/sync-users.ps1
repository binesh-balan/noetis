<#
.SYNOPSIS
  Gives every enabled member of the employee group a LiteLLM user record with a monthly
  budget. Run it on a schedule (hourly), e.g. Task Scheduler or an Azure Automation runbook.

  Access itself is decided by the Entra app role in each token (custom_auth.py); this script
  only sets budgets. Until a new hire's first sync they are rate-limited but have no budget.

  Needs: Azure CLI signed in (az login) as an identity that can read group members
  (GroupMember.Read.All), and the LiteLLM master key.

.EXAMPLE
  $env:LITELLM_MASTER_KEY = '<master key>'
  .\sync-users.ps1 -GatewayUrl https://llm-gateway.contoso.internal -GroupId <employee group id> -MonthlyBudgetUsd 10
#>
param(
    [Parameter(Mandatory)] [string] $GatewayUrl,
    [Parameter(Mandatory)] [string] $GroupId,
    [double] $MonthlyBudgetUsd = 10,
    [string] $MasterKey = $env:LITELLM_MASTER_KEY
)
$ErrorActionPreference = 'Stop'
if (-not $MasterKey) { throw 'Pass -MasterKey or set LITELLM_MASTER_KEY.' }

$graphToken = az account get-access-token --resource-type ms-graph --query accessToken -o tsv
if (-not $graphToken) { throw 'Run az login first.' }

$url = "https://graph.microsoft.com/v1.0/groups/$GroupId/transitiveMembers/microsoft.graph.user?`$select=id,userPrincipalName,accountEnabled&`$top=999"
$users = @()
while ($url) {
    $page = Invoke-RestMethod -Uri $url -Headers @{ Authorization = "Bearer $graphToken" }
    $users += @($page.value | Where-Object { $_.accountEnabled })
    $url = $page.'@odata.nextLink'
}

$headers = @{ Authorization = "Bearer $MasterKey" }
$base = $GatewayUrl.TrimEnd('/')
$created = 0; $updated = 0; $failed = 0
foreach ($u in $users) {
    $budget = @{ user_id = $u.id; user_email = $u.userPrincipalName; max_budget = $MonthlyBudgetUsd; budget_duration = '30d' }
    try {
        # No virtual key: employees authenticate with Entra ID only.
        $new = $budget + @{ user_role = 'internal_user'; auto_create_key = $false }
        Invoke-RestMethod -Method Post -Uri "$base/user/new" -Headers $headers -ContentType 'application/json' -Body ($new | ConvertTo-Json) | Out-Null
        $created++
    } catch {
        try {
            Invoke-RestMethod -Method Post -Uri "$base/user/update" -Headers $headers -ContentType 'application/json' -Body ($budget | ConvertTo-Json) | Out-Null
            $updated++
        } catch {
            Write-Warning "$($u.userPrincipalName): $($_.Exception.Message)"
            $failed++
        }
    }
}
"Synced $($users.Count) employees: $created new, $updated updated, $failed failed."
if ($failed) { exit 1 }
