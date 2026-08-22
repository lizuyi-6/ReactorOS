
$c = Get-Content output/e2e-acceptance/logs/playwright-final3.log -Raw
$blocks = [regex]::Matches($c, '(?s)(\d+\) \[(?:desktop|mobile)-chromium\].*?)(?=\r?\n  \d+\) \[|\r?\n\s*\d+ failed|$)')
foreach ($b in $blocks) {
  $t = $b.Value
  if ($t -match 'fullchain.mobile.spec.mjs:63|vue-acceptance.desktop.spec.mjs:8') {
    Write-Output ('===== BLOCK =====')
    Write-Output $t.Substring(0, [Math]::Min(2600, $t.Length))
  }
}
