$log = Get-Content 'C:\Users\Abraham\AppData\Local\Temp\dsh-subprocess-YczHex\dsh-subprocess-9108-1-1dbec46b5a0d-stdout.log'
$errors = @()
$currentTest = ""
$inError = $false
$errorLines = @()

foreach ($line in $log) {
  if ($line -match '\u2718\s*\d+\s+\[\w-+\]\s*\u231b\s*(.+)') {
    if ($inError -and $errorLines.Count -gt 0) {
      $errors += "$currentTest|$($errorLines -join ' ')"
    }
    $currentTest = $Matches[1].Trim().Substring(0, [Math]::Min(80, $Matches[1].Trim().Length))
    $inError = $false
    $errorLines = @()
  }
  if ($line -match 'Error:') {
    $inError = $true
    $errorLines = @($line)
  } elseif ($inError) {
    $errorLines += $line
    if ($errorLines.Count -gt 20) { $inError = $false }
  }
}
$errors | ForEach-Object { Write-Output $_ }
