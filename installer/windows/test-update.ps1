# Installs the old/ build every way it can be installed and updates it to the www/ build from a local server, with the
# real installers and the real SYSTEM task. See build-two-versions.sh. Needs VERSION, and an admin prompt.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\test-lib.ps1"

$new = '99.0.0'
$oldBase = (Resolve-Path 'old').Path + "\daleris-v$env:VERSION-windows-x64"
$www = (Resolve-Path 'www').Path
$newBase = "$www\daleris-v$new-windows-x64"
$task = 'Daleris Updater'
$systemLog = "$env:windir\System32\config\systemprofile\AppData\Local\Daleris\logs"
$ownLog = "$env:LOCALAPPDATA\Daleris\logs"
$policy = 'HKLM:\SOFTWARE\Policies\Daleris'
$switch = "$env:ProgramData\Daleris\auto-update.txt"
$defaultFolder = "$env:ProgramFiles\Daleris"

Set-Content 'C:\daleris-update-test.txt' 'http://127.0.0.1:18765'
$server = Start-Process python -ArgumentList '-m', 'http.server', '18765', '--bind', '127.0.0.1', '--directory', "`"$www`"" -PassThru -WindowStyle Hidden
Start-Sleep 3

function Say($text) { Write-Host "=== $text" }

function Run($exe, $arguments, $credential) {
  $out = New-TemporaryFile
  $options = @{ FilePath = $exe; ArgumentList = $arguments; Wait = $true; PassThru = $true; RedirectStandardOutput = $out.FullName }
  if ($credential) { $options.Credential = $credential; $options.LoadUserProfile = $true; $options.WorkingDirectory = "$env:windir\Temp" }
  $p = Start-Process @options
  [pscustomobject]@{ Code = $p.ExitCode; Text = (Get-Content -Raw $out.FullName) }
}

function Version($exe) { (Run $exe '--version').Text.Trim() }

function Assert-Version($exe, $version) {
  $got = Version $exe
  if ($got -ne "Daleris $version") { throw "$exe is '$got', not $version" }
}

# Where the installer of the update being tested writes its log.
$script:installerLog = $null

function Show-Logs {
  foreach ($log in "$systemLog\daleris.log", "$ownLog\daleris.log", $script:installerLog) {
    if ($log -and (Test-Path $log)) {
      Write-Host "--- $log"
      # an installer's log is long, so show what led to its first failure
      $failed = Select-String -Path $log -Pattern 'Return value 3|Error \d{4}\.|^.*error occurred' | Select-Object -First 1
      if ($failed) { Get-Content $log | Select-Object -Skip ([Math]::Max(0, $failed.LineNumber - 30)) -First 45 | Write-Host }
      Get-Content $log -Tail 25 | Write-Host
    }
  }
}

function Installer-Running { @(Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -match "daleris-v$new" -and $_.ProcessId -ne $PID -and $_.Name -notmatch 'python|pwsh|powershell' }).Count -gt 0 }

# Waits for the installer the update started, then checks that it says it worked.
function Wait-Updated($exe) {
  foreach ($i in 1..72) {
    Start-Sleep 5
    if ((Installer-Running)) { continue }
    try { if ((Version $exe) -eq "Daleris $new") { break } } catch {}
    if ($i -eq 72) { Show-Logs; throw "$exe didn't update" }
  }
  Assert-Version $exe $new
  if ($script:installerLog -notlike '*.log' -or -not (Test-Path $script:installerLog)) { throw "The installer left no log at $script:installerLog" }
  # an in-use file only gets replaced at the next restart
  if ((Get-Content -Raw $script:installerLog) -match 'requires a system restart|ReplacedInUseFiles|Need to restart Windows\? Yes') { Show-Logs; throw "The update needs a restart" }
}

function Install-Msi($msi, [string[]]$properties) {
  $p = Start-Process msiexec -ArgumentList (@('/i', "`"$msi`"", '/qn', '/norestart') + $properties) -Wait -PassThru
  if ($p.ExitCode) { throw "$msi didn't install: $($p.ExitCode)" }
}

function Remove-Msi($msi, [string[]]$properties) {
  $p = Start-Process msiexec -ArgumentList (@('/x', "`"$msi`"", '/qn', '/norestart') + $properties) -Wait -PassThru
  if ($p.ExitCode) { throw "$msi didn't uninstall: $($p.ExitCode)" }
}

function Install-Setup([string[]]$switches) {
  $p = Start-Process "$oldBase-setup.exe" -ArgumentList (@('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') + $switches) -Wait -PassThru
  if ($p.ExitCode) { throw "setup.exe didn't install: $($p.ExitCode)" }
}

function Remove-Setup($folder) {
  $p = Start-Process "$folder\unins000.exe" -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
  if ($p.ExitCode) { throw "setup.exe didn't uninstall: $($p.ExitCode)" }
  Start-Sleep 5
}

function Listed($root) { @(Get-ItemProperty "$root\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*" -ErrorAction SilentlyContinue | Where-Object DisplayName -eq 'Daleris') }

# The task installs the update for a PC-wide install, which stays where it is, locked, listed once and with its task.
function Test-Machine($folder) {
  $exe = "$folder\daleris.exe"
  Assert-Version $exe $env:VERSION
  $script:installerLog = "$folder\updates\installer.log"
  schtasks /Run /TN $task | Out-Null
  if ($LASTEXITCODE) { throw "The task didn't start" }
  Wait-Updated $exe
  Assert-Locked $folder
  if ((Listed 'HKLM:').Count -ne 1) { throw "Windows lists Daleris $((Listed 'HKLM:').Count) times" }
  $xml = schtasks /Query /TN $task /XML | Out-String
  if ($LASTEXITCODE -or -not $xml.Contains("$folder\daleris.exe")) { throw "After updating, the task doesn't run $exe" }
  if ($folder -ne $defaultFolder -and (Test-Path $defaultFolder)) { throw "The update also installed to the default folder" }
  # with nothing new to do, it removes its download
  schtasks /Run /TN $task | Out-Null
  Start-Sleep 15
  if (Get-ChildItem "$folder\updates" -File -ErrorAction SilentlyContinue | Where-Object Name -like '*99.0.0*') { throw "The task kept the installer it used" }
}

function Assert-Gone($folder) {
  if (Test-Path $folder) { throw "Uninstalling left $folder behind: $((Get-ChildItem -Recurse $folder).FullName -join ', ')" }
  schtasks /Query /TN $task 2>$null | Out-Null
  if ($LASTEXITCODE -eq 0) { throw "Uninstalling left the task" }
  $global:LASTEXITCODE = 0
}

# An update that must not install, as the task's program would run it: it says why in the log, or exits with a failure.
function Assert-Stays($exe, $expectedCode, $logText) {
  Remove-Item "$ownLog\daleris.log" -ErrorAction SilentlyContinue
  $r = Run $exe '--update-service'
  if ($r.Code -ne $expectedCode) { Show-Logs; throw "The update service exited with $($r.Code), not $expectedCode" }
  if (-not (Select-String -Quiet -SimpleMatch $logText "$ownLog\daleris.log")) { Show-Logs; throw "The log doesn't say '$logText'" }
  if (Installer-Running) { throw "An installer started" }
  Assert-Version $exe $env:VERSION
}

function Clear-Policy {
  Remove-Item $policy -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item $switch -Force -ErrorAction SilentlyContinue
}

try {
  Say 'The templates are valid XML'
  foreach ($file in 'PolicyDefinitions\Daleris.admx', 'PolicyDefinitions\en-US\Daleris.adml', 'PolicyDefinitions\de-DE\Daleris.adml') { [void][xml](Get-Content -Raw $file) }

  Say 'PC-wide MSI, default folder'
  Install-Msi "$oldBase.msi" 'ALLUSERS=1'
  $exe = "$defaultFolder\daleris.exe"
  Test-Machine $defaultFolder
  $target = (New-Object -ComObject WScript.Shell).CreateShortcut("$env:ProgramData\Microsoft\Windows\Start Menu\Programs\Daleris.lnk").TargetPath
  if ($target -ne $exe) { throw "After updating, the shortcut points to '$target'" }
  Remove-Msi "$newBase.msi" 'ALLUSERS=1'
  Assert-Gone $defaultFolder

  Say 'PC-wide MSI, chosen folder'
  $chosen = 'C:\Apps\Daleris MSI Test'
  Install-Msi "$oldBase.msi" 'ALLUSERS=1', "APPLICATIONFOLDER=`"$chosen`""
  Test-Machine $chosen
  Remove-Msi "$newBase.msi" 'ALLUSERS=1'
  Assert-Gone $chosen

  Say 'PC-wide setup.exe, default folder'
  Install-Setup '/ALLUSERS'
  Test-Machine $defaultFolder
  Remove-Setup $defaultFolder
  Assert-Gone $defaultFolder

  Say 'PC-wide setup.exe, chosen folder'
  $chosen = 'C:\Apps\Daleris Setup Test'
  Install-Setup '/ALLUSERS', "/DIR=`"$chosen`""
  Test-Machine $chosen
  Remove-Setup $chosen
  Assert-Gone $chosen

  Say 'What stops the task from updating'
  Install-Msi "$oldBase.msi" 'ALLUSERS=1'
  $off = 'automatic updates are off'

  New-Item $policy -Force | Out-Null
  Set-ItemProperty $policy DisableUpdates 1 -Type DWord
  schtasks /Run /TN $task | Out-Null
  Start-Sleep 25
  if (Installer-Running) { throw "DisableUpdates: the task started an installer" }
  Assert-Version $exe $env:VERSION
  if (-not (Select-String -Quiet -SimpleMatch $off "$systemLog\daleris.log")) { Show-Logs; throw "DisableUpdates: the task didn't say it's off" }
  $r = Run $exe '--update-now'
  if ($r.Text -notmatch 'managed by the administrator') { throw "DisableUpdates: --update-now said '$($r.Text)'" }
  Clear-Policy

  New-Item $policy -Force | Out-Null
  Set-ItemProperty $policy AutomaticUpdates 0 -Type DWord
  Assert-Stays $exe 0 $off
  New-Item (Split-Path $switch) -ItemType Directory -Force | Out-Null
  Set-Content $switch '1'
  Assert-Stays $exe 0 $off
  Clear-Policy

  New-Item "$policy\Recommended" -Force | Out-Null
  Set-ItemProperty "$policy\Recommended" AutomaticUpdates 0 -Type DWord
  Assert-Stays $exe 0 $off
  Clear-Policy

  New-Item (Split-Path $switch) -ItemType Directory -Force | Out-Null
  Set-Content $switch '0'
  Assert-Stays $exe 0 $off
  Clear-Policy

  Say 'What must not be installed'
  $list = "$www\latest.json"
  $signature = "$www\latest.json.sig"
  $goodList = [IO.File]::ReadAllBytes($list)
  $goodSignature = [IO.File]::ReadAllBytes($signature)
  $restore = { [IO.File]::WriteAllBytes($list, $goodList); [IO.File]::WriteAllBytes($signature, $goodSignature) }

  [IO.File]::WriteAllBytes($signature, (New-Object byte[] 64))
  Assert-Stays $exe 1 "signature doesn't match"
  & $restore

  [IO.File]::AppendAllText($list, "file 00 evil.exe`n")
  Assert-Stays $exe 1 "signature doesn't match"
  & $restore

  # signed, but the file isn't what the list says
  $text = (Get-Content -Raw $list) -replace '(?m)^file [0-9a-f]+ (daleris-v.*\.msi)\s*$', ('file ' + ('0' * 64) + ' $1')
  [IO.File]::WriteAllText($list, $text)
  & "$oldBase-portable.exe" --test-sign $list $signature | Out-Null
  Assert-Stays $exe 1 "didn't match its signed checksum"
  & $restore

  # signed, but older
  [IO.File]::WriteAllText($list, "version 0.0.0`n")
  & "$oldBase-portable.exe" --test-sign $list $signature | Out-Null
  Assert-Stays $exe 0 'install type Machine'
  & $restore

  Say 'The task waits while Daleris is open'
  $fake = "$env:TEMP\open\daleris.exe"
  New-Item (Split-Path $fake) -ItemType Directory -Force | Out-Null
  Copy-Item "$env:windir\System32\ping.exe" $fake
  $open = Start-Process $fake -ArgumentList '-n', '60', '127.0.0.1' -PassThru -WindowStyle Hidden
  Assert-Stays $exe 0 'is waiting for Daleris to close'
  $open.Kill()

  Say 'Someone without admin rights on a PC-wide install'
  $name = 'daleristest'
  $password = "Te5t-$(New-Guid)"
  $secure = ConvertTo-SecureString $password -AsPlainText -Force
  New-LocalUser $name -Password $secure | Out-Null
  $credential = [pscredential]::new($name, $secure)
  # its own folder for the log and the update list, whichever environment it starts with
  $home2 = 'C:\daleris-test-home'
  New-Item $home2 -ItemType Directory -Force | Out-Null
  icacls $home2 /grant '*S-1-1-0:(OI)(CI)F' | Out-Null
  $saved = $env:LOCALAPPDATA
  $env:LOCALAPPDATA = $home2
  $r = Run $exe '--update-now' $credential
  $env:LOCALAPPDATA = $saved
  Remove-LocalUser $name
  if ($r.Code -or $r.Text -notmatch 'InstallsOnClose') { throw "A normal user's check said '$($r.Text)', exit $($r.Code)" }
  Assert-Version $exe $env:VERSION

  Say 'A fixed on beats the switch'
  New-Item $policy -Force | Out-Null
  Set-ItemProperty $policy AutomaticUpdates 1 -Type DWord
  New-Item (Split-Path $switch) -ItemType Directory -Force | Out-Null
  Set-Content $switch '0'
  Test-Machine $defaultFolder
  Clear-Policy
  Remove-Msi "$newBase.msi" 'ALLUSERS=1'
  Assert-Gone $defaultFolder

  $own = "$env:LOCALAPPDATA\Programs\Daleris"
  foreach ($test in @(
      @{ Name = 'One account, MSI, default folder'; Folder = $own; Msi = $true; Extra = @() },
      @{ Name = 'One account, MSI, chosen folder'; Folder = "$env:LOCALAPPDATA\Daleris MSI Test"; Msi = $true; Extra = @("APPLICATIONFOLDER=`"$env:LOCALAPPDATA\Daleris MSI Test`"") },
      @{ Name = 'One account, setup.exe, default folder'; Folder = $own; Msi = $false; Extra = @() },
      @{ Name = 'One account, setup.exe, chosen folder'; Folder = "$env:LOCALAPPDATA\Daleris Setup Test"; Msi = $false; Extra = @("/DIR=`"$env:LOCALAPPDATA\Daleris Setup Test`"") })) {
    Say $test.Name
    $perUser = 'ALLUSERS=2', 'MSIINSTALLPERUSER=1'
    if ($test.Msi) { Install-Msi "$oldBase.msi" ($perUser + $test.Extra) } else { Install-Setup (@('/CURRENTUSER') + $test.Extra) }
    $exe = "$($test.Folder)\daleris.exe"
    Assert-Version $exe $env:VERSION
    $script:installerLog = "$ownLog\updates\installer.log"
    $r = Run $exe '--update-now'
    if ($r.Code -or $r.Text -notmatch 'InstallsOnClose') { Show-Logs; throw "--update-now said '$($r.Text)', exit $($r.Code)" }
    Wait-Updated $exe
    if ($test.Folder -ne $own -and (Test-Path "$own\daleris.exe")) { throw "The update also installed to the default folder" }
    if (Test-Path "$($test.Folder)\machine-install.txt") { throw "It became a PC-wide install" }
    schtasks /Query /TN $task 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { throw "An install for one account added the task" }
    $global:LASTEXITCODE = 0
    if ($test.Msi) { Remove-Msi "$newBase.msi" $perUser } else { Remove-Setup $test.Folder }
    if (Test-Path $exe) { throw "$exe is still there after uninstalling" }
  }

  Say 'Portable'
  $portable = "$env:TEMP\portable\daleris.exe"
  New-Item (Split-Path $portable) -ItemType Directory -Force | Out-Null
  Copy-Item "$oldBase-portable.exe" $portable
  $r = Run $portable '--update-now'
  if ($r.Code -or $r.Text -notmatch 'RestartToFinish') { Show-Logs; throw "--update-now said '$($r.Text)', exit $($r.Code)" }
  Assert-Version $portable $new

  Say 'Everything passed'
} catch {
  Show-Logs
  throw
} finally {
  if (-not $server.HasExited) { $server.Kill() }
  Remove-Item 'C:\daleris-update-test.txt' -ErrorAction SilentlyContinue
}
$global:LASTEXITCODE = 0
