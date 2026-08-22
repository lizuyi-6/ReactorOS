
$c = Get-Content output/e2e-acceptance/logs/playwright-final4.log -Raw
$blocks = [regex]::Matches($c, '(?s)(\d+\) \[desktop-chromium\].*?ux-audit\.desktop.*?)(?=\r?\n  \d+\) \[|\r?\n\s*\d+ failed|$)')
foreach ($b in $blocks) {
  $t = $b.Value
  $head = (($t -split [char]10)[0]).Trim()
  $route = [regex]::Match($head, '› ([a-z]+) - overflow').Groups[1].Value
  $arr = [regex]::Match($t, '(?s)\+ Array \[(.*?)\]')
  if ($arr.Success) {
    $items = [regex]::Matches($arr.Groups[1].Value, '"([^"]+)"') | ForEach-Object { $_.Groups[1].Value } | Group-Object | Sort-Object Count -Descending | ForEach-Object { $_.Name + ' x' + $_.Count }
    Write-Output ('== ' + $route + ' ==')
    $items | ForEach-Object { Write-Output ('   ' + $_) }
  }
}
