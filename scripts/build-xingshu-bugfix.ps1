$env:CARGO_TARGET_DIR = "C:\tmp\xingshu-target-v3"
$root = (Resolve-Path -LiteralPath $PSScriptRoot\..).Path
Set-Location $root
$proc = Start-Process -FilePath "cargo" -ArgumentList @(
    "build", "--bin", "xingshu"
) -WorkingDirectory $root `
  -RedirectStandardOutput "C:\tmp\xingshu-target-v3\xingshu-build.out.log" `
  -RedirectStandardError "C:\tmp\xingshu-target-v3\xingshu-build.err.log" `
  -PassThru -WindowStyle Hidden
$proc.WaitForExit()
Write-Output "build exit=$($proc.ExitCode)"
Get-Item C:\tmp\xingshu-target-v3\debug\xingshu.exe | Select-Object LastWriteTime, Length
