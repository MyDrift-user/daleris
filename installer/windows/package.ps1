# Builds the Windows release files into dist/, from what `cargo build --release --workspace` and a 32-bit build of
# daleris-dialogs left in target\. Needs VERSION, Inno Setup and WiX.
$base = "daleris-v$env:VERSION-windows-x64"
New-Item -ItemType Directory -Force dist | Out-Null
# The file dialog DLLs, under the names the installers and the program use.
Copy-Item target\release\daleris_dialogs.dll target\release\daleris-dialogs-x64.dll -Force
$x86 = 'target\i686-pc-windows-msvc\release\daleris_dialogs.dll', 'target\i686-pc-windows-gnu\release\daleris_dialogs.dll' | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $x86) { throw "No 32-bit daleris_dialogs.dll: build daleris-dialogs for i686-pc-windows-msvc first" }
Copy-Item $x86 target\release\daleris-dialogs-x86.dll -Force
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" /Q "/DAppVersion=$env:VERSION" "/DOutputName=$base-setup" installer\setup.iss
if ($LASTEXITCODE) { throw "ISCC failed" }
# The wizard shows the licence as RTF, which is plain text with its special characters escaped.
$lines = (Get-Content LICENSE) -replace '\\', '\\' -replace '\{', '\{' -replace '\}', '\}'
Set-Content target\license.rtf ('{\rtf1\ansi\deff0{\fonttbl{\f0 Consolas;}}\f0\fs15 ' + ($lines -join "\par`n") + '}') -Encoding ascii
wix build -arch x64 -ext WixToolset.UI.wixext -d "Version=$env:VERSION" -o "dist\$base.msi" installer\app.wxs
if ($LASTEXITCODE) { throw "wix failed" }
Remove-Item dist/*.wixpdb
Copy-Item target\release\daleris.exe "dist\$base-portable.exe"
Compress-Archive -Path target\release\daleris.exe, target\release\daleris-open.exe, target\release\daleris-dialogs-x64.dll, target\release\daleris-dialogs-x86.dll, LICENSE, THIRD-PARTY-NOTICES.txt -DestinationPath "dist\$base.zip"
Compress-Archive -Path policy\* -DestinationPath "dist\daleris-v$env:VERSION-policy-definitions.zip"
Copy-Item LICENSE "dist\daleris-v$env:VERSION-LICENSE.txt"
Copy-Item THIRD-PARTY-NOTICES.txt "dist\daleris-v$env:VERSION-THIRD-PARTY-NOTICES.txt"
