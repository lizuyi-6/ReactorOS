
$c = Get-Content output/e2e-acceptance/logs/ux-round1.log -Raw
$blocks = [regex]::Matches($c, '(?s)(\d+\) \[(?:desktop|mobile)-chromium\].*?)(?=\r?\n  \d+\) \[|\r?\n\s*\d+ failed|$)')
foreach ($b in $blocks) {
  $t = $b.Value
  $head = (($t -split [char]10)[0]).Trim()
  $route = [regex]::Match($head, '› ([a-z]+) - overflow|› (mobile - no text clipping)|› (all pages: English)|› (mobile: English)').Groups
  $label = ($route[1].Value + $route[2].Value + $route[3].Value + $route[4].Value)
  $proj = [regex]::Match($head, '\[(\w+-chromium)\]').Groups[1].Value
  $arr = [regex]::Match($t, '(?s)\+ Array \[(.*?)\]')
  if ($arr.Success) {
    $items = [regex]::Matches($arr.Groups[1].Value, '"([^"]+)"') | ForEach-Object { $_.Groups[1].Value } | Group-Object | Sort-Object Count -Descending | Select-Object -First 12 | ForEach-Object { $_.Name + ' x' + $_.Count }
    Write-Output ('== ' + $proj + ' / ' + $label + ' ==')
    $items | ForEach-Object { Write-Output ('   ' + $_) }
  } else {
    $em = [regex]::Match($t, 'Error:.*').Value
    Write-Output ('== ' + $proj + ' / ' + $label + ' == (no array) ' + $em)
  }
}
