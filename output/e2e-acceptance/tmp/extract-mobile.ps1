
$c = Get-Content output/e2e-acceptance/logs/playwright-final.log -Raw
$blocks = [regex]::Matches($c, '(?s)(\d+\) \[mobile-chromium\].*?)(?=\r?\n  \d+\) \[|\r?\n\s*\d+ failed|$)')
foreach ($b in $blocks) {
  $t = $b.Value
  $nl = [char]10
  $head = (($t -split $nl)[0]).Trim()
  $errs = ([regex]::Matches($t, '(?m)^\s+(Error:.*|Received.*|Expected.*|>\s+\d+ \|.*|waiting for.*)$') | ForEach-Object { $_.Value.Trim() } | Select-Object -First 5) -join ' | '
  Write-Output ($head.Substring(0, [Math]::Min(110, $head.Length)))
  Write-Output ('    ' + $errs)
}
