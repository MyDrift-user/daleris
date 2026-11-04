# Packs Material Icon Theme (MIT, https://github.com/material-extensions/vscode-material-icon-theme) into
# assets/icons/material.bin: what the file list needs from it, compressed. To update: pwsh assets/icons/build.ps1 5.40.0
#
# Inside, one entry after another: "@name length\n" and then that many bytes. The entry called "manifest" has a
# line per association: E/N/D for extension/file name/folder name, e/n/d for the same on a light theme, then the
# name and icon separated by a tab.
param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'

$here = $PSScriptRoot
$work = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory $work | Out-Null
Invoke-WebRequest "https://registry.npmjs.org/material-icon-theme/-/material-icon-theme-$Version.tgz" -OutFile "$work\p.tgz"
& "$env:SystemRoot\System32\tar.exe" xzf "$work\p.tgz" -C $work
$pkg = Join-Path $work 'package'
$m = Get-Content -Raw "$pkg\dist\material-icons.json" | ConvertFrom-Json

$lines = [Collections.Generic.List[string]]::new()
$used = [Collections.Generic.HashSet[string]]::new()
function Add-Map($map, $code) {
    if (-not $map) { return }
    foreach ($p in $map.PSObject.Properties) {
        $lines.Add("$code`t$($p.Name.ToLowerInvariant())`t$($p.Value)")
        [void]$used.Add($p.Value)
    }
}
Add-Map $m.fileExtensions 'E'
Add-Map $m.fileNames 'N'
Add-Map $m.folderNames 'D'
Add-Map $m.light.fileExtensions 'e'
Add-Map $m.light.fileNames 'n'
Add-Map $m.light.folderNames 'd'
[void]$used.Add($m.file)
[void]$used.Add($m.folder)
foreach ($extra in 'folder-home') { [void]$used.Add($extra) }
$lines.Add("F`tfile`t$($m.file)")
$lines.Add("F`tfolder`t$($m.folder)")

$out = [IO.MemoryStream]::new()
function Add-Entry($name, [byte[]]$bytes) {
    $head = [Text.Encoding]::ASCII.GetBytes("@$name $($bytes.Length)`n")
    $out.Write($head, 0, $head.Length)
    $out.Write($bytes, 0, $bytes.Length)
}
Add-Entry 'manifest' ([Text.Encoding]::UTF8.GetBytes(($lines -join "`n")))
$count = 0
foreach ($id in $used) {
    $path = ($m.iconDefinitions.$id.iconPath -replace '^\./\.\./', '') -replace '/', '\'
    if (-not $path) { continue }
    Add-Entry $id ([IO.File]::ReadAllBytes((Join-Path $pkg $path)))
    $count++
}

$packed = [IO.MemoryStream]::new()
$deflate = [IO.Compression.DeflateStream]::new($packed, [IO.Compression.CompressionLevel]::SmallestSize, $true)
$raw = $out.ToArray()
$deflate.Write($raw, 0, $raw.Length)
$deflate.Dispose()
[IO.File]::WriteAllBytes((Join-Path $here 'material.bin'), $packed.ToArray())
Copy-Item "$pkg\LICENSE" (Join-Path $here 'MATERIAL-ICONS-LICENSE.txt')
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
"$count icons, $($lines.Count) associations, $([math]::Round($raw.Length / 1MB, 1)) MB packed to $([math]::Round($packed.Length / 1KB)) KB"
