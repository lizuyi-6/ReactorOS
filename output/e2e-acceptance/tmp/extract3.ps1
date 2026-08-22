
$c = Get-Content output/e2e-acceptance/logs/playwright-final2.log -Raw
$blocks = [regex]::Matches($c, '(?s)(\d+\) \[(?:desktop|mobile)-chromium\].*?)(?=\r?\n  \d+\) \[|\r?\n\s*\d+ failed|$)')
foreach ($b in $blocks) {
  $t = $b.Value
  if ($t -match 'fullchain.desktop.spec.mjs:94|vue-acceptance.desktop.spec.mjs:30|fullchain.mobile.spec.mjs:63') {
    $nl = [char]10
    $head = (($t -split $nl)[0]).Trim()
    Write-Output ('===== ' + $head.Substring(0, [Math]::Min(100, $head.Length)))
    $lines = ([regex]::Matches($t, '(?m)^\s+(Error:.*|Received.*|Expected.*|>\s+\d+ \|.*|waiting for.*|locator\..*)$') | ForEach-Object { $_.Value.Trim() } | Select-Object -First 10) -join ($nl + '    ')
    Write-Output ('    ' + $lines)
  }
}
