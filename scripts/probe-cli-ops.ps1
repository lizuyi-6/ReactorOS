$env:CARGO_TARGET_DIR = "C:\tmp\xingshu-target-v3"
$bin = "C:\tmp\xingshu-target-v3\debug\xingshu.exe"
& $bin ops --help
Write-Host "---"
& $bin key --help
Write-Host "---"
$tmpDb = "C:\tmp\ops-test.db"
$tmpBackup = "C:\tmp\ops-backup.db"
if (Test-Path $tmpDb) { Remove-Item $tmpDb -Force }
"abc" | Out-File -FilePath $tmpDb -Encoding ascii -NoNewline
& $bin ops backup --db $tmpDb --out $tmpBackup --include-ciphertext
Write-Host "---"
& $bin ops wipe --db $tmpDb --yes
Write-Host "---"
Test-Path $tmpDb
Test-Path $tmpBackup
Get-Item $tmpBackup | Select-Object Length
& $bin key rotate --db $tmpDb --yes 2>&1 | Out-Host
