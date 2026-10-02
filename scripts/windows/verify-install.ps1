<#
  Verifies the built Windows installer on a real Windows host (CI runner or a throwaway VM).
  It installs, launches, and uninstalls MuDraft for the current user, so never run it on a
  machine whose MuDraft profile matters.

  Checks: per-user install location, shortcuts, Add/Remove Programs metadata, installed
  files (no fixtures, test servers, or databases), production data folder (the
  development override is ignored), persistence across restarts, upgrade from an older
  installer keeping the profile, and an uninstall that keeps the profile by default.
#>
param(
  [Parameter(Mandatory)] [string] $Installer,
  [Parameter(Mandatory)] [string] $OldInstaller,
  [Parameter(Mandatory)] [string] $Version,
  [Parameter(Mandatory)] [string] $OldVersion,
  [Parameter(Mandatory)] [string] $Shots
)
$ErrorActionPreference = 'Stop'

$Product = 'MuDraft'
$Identifier = 'app.mudraft.desktop'
$InstallDir = Join-Path $env:LOCALAPPDATA $Product
$DataDir = Join-Path $env:LOCALAPPDATA $Identifier
$Db = Join-Path $DataDir 'mudraft.sqlite3'
$Artwork = Join-Path $DataDir 'artwork'
$UninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$Product"
$StartMenuLink = Join-Path ([Environment]::GetFolderPath('Programs')) "$Product.lnk"
$DesktopLink = Join-Path ([Environment]::GetFolderPath('Desktop')) "$Product.lnk"
$Ignored = Join-Path $env:RUNNER_TEMP 'mudraft-dev-override'
New-Item -ItemType Directory -Force $Shots | Out-Null

function Check([bool] $ok, [string] $what) {
  if (-not $ok) { throw "VERIFY FAILED: $what" }
  Write-Host "  ok: $what"
}

# Queries go through a script file so no SQL is re-quoted on the command line.
$SqlPy = Join-Path $env:RUNNER_TEMP 'mudraft-sql.py'
Set-Content $SqlPy @'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
r = c.execute(sys.argv[2]).fetchone()
c.commit()
print("" if r is None else r[0])
'@

function Sql([string] $query) {
  $out = python $SqlPy $Db $query
  if ($LASTEXITCODE -ne 0) { throw "sqlite query failed: $query" }
  return "$out".Trim()
}

function Install([string] $setup, [string] $expectVersion) {
  Write-Host "== Install $expectVersion"
  $p = Start-Process $setup -ArgumentList '/S' -Wait -PassThru
  Check ($p.ExitCode -eq 0) "installer exited 0 (got $($p.ExitCode))"
  Check ((Get-ItemProperty $UninstallKey).DisplayVersion -eq $expectVersion) "registered as version $expectVersion"
}

function MainExe {
  $exe = Get-ChildItem $InstallDir -Filter *.exe | Where-Object Name -ne 'uninstall.exe'
  Check (@($exe).Count -eq 1) "one app executable in $InstallDir"
  return $exe.FullName
}

function Shot([string] $name) {
  Add-Type -AssemblyName System.Windows.Forms, System.Drawing
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
  $bmp.Save((Join-Path $Shots "$name.png"))
  $g.Dispose(); $bmp.Dispose()
}

# Launches the installed app as a user would, with the development data-folder override
# set: a release build must ignore it. Waits for the database, then closes the app.
function Launch([string] $label) {
  Write-Host "== Launch ($label)"
  $env:MUDRAFT_DATA_DIR = $Ignored
  $app = Start-Process (MainExe) -PassThru
  Remove-Item Env:\MUDRAFT_DATA_DIR
  $deadline = (Get-Date).AddSeconds(90)
  while (-not (Test-Path $Db) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
  Check (Test-Path $Db) "profile database created at $Db"
  Start-Sleep -Seconds 8
  Check (-not $app.HasExited) 'app is still running after start-up'
  Shot $label
  $null = $app.CloseMainWindow()
  if (-not $app.WaitForExit(15000)) { Stop-Process -Id $app.Id -Force; $app.WaitForExit() }
  Check (-not (Test-Path $Ignored)) 'release build ignored the development data override'
  Check ((Sql 'PRAGMA quick_check') -eq 'ok') 'database integrity check passes'
  Check ([int](Sql 'PRAGMA user_version') -ge 11) 'database is at the current schema'
}

Check (-not (Test-Path $InstallDir)) 'no previous install on this machine'
Check (-not (Test-Path $DataDir)) 'no previous profile on this machine'

# 1. Older version: fresh per-user install.
Install $OldInstaller $OldVersion
$key = Get-ItemProperty $UninstallKey
Check ($key.Publisher -eq $Product) 'publisher recorded'
Check (Test-Path $StartMenuLink) 'Start menu shortcut'
Check (Test-Path $DesktopLink) 'desktop shortcut'
Check (-not (Test-Path (Join-Path $env:ProgramFiles $Product))) 'nothing installed under Program Files'
$files = Get-ChildItem $InstallDir -Recurse -File | ForEach-Object { $_.FullName.Substring($InstallDir.Length + 1) }
Write-Host "  installed files: $($files -join ', ')"
Check (-not ($files | Where-Object { $_ -match '\.(sqlite3?|db|csv|json|mudraft)$|fixture|e2e' })) 'no fixtures, test data, or databases installed'
$bytes = [System.IO.File]::ReadAllBytes((MainExe))
$text = [System.Text.Encoding]::ASCII.GetString($bytes)
foreach ($needle in 'wdio', 'MUDRAFT_E2E', 'TAURI_WEBDRIVER_PORT') {
  Check (-not $text.Contains($needle)) "app binary has no '$needle' test hook"
}
Launch 'first-run'

# 2. Seed a profile marker and an artwork file while the app is closed.
Write-Host '== Seed user data'
$null = Sql "INSERT INTO tag (id, name, color) VALUES ('0190f5c3-0000-7000-8000-00000000beef', 'Upgrade marker', '#22d3ee')"
New-Item -ItemType Directory -Force $Artwork | Out-Null
$png = [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==')
[System.IO.File]::WriteAllBytes((Join-Path $Artwork 'marker.png'), $png)
$marker = "SELECT COUNT(*) FROM tag WHERE name = 'Upgrade marker'"
Launch 'restart'
Check ((Sql $marker) -eq '1') 'profile kept after restart'

# 3. Upgrade in place to the release build.
Install $Installer $Version
Check (Test-Path $StartMenuLink) 'Start menu shortcut after upgrade'
Launch 'after-upgrade'
Check ((Sql $marker) -eq '1') 'profile kept across upgrade'
Check (Test-Path (Join-Path $Artwork 'marker.png')) 'artwork kept across upgrade'
Check ((Sql "SELECT color FROM tag WHERE builtin_key = 'listen_asap'") -eq '#d4a017') 'Listen ASAP default is mustard'

# 4. Uninstall with defaults: app removed, profile kept.
Write-Host '== Uninstall'
$exe = MainExe
$p = Start-Process (Join-Path $InstallDir 'uninstall.exe') -ArgumentList '/S', "_?=$InstallDir" -Wait -PassThru
Check ($p.ExitCode -eq 0) "uninstaller exited 0 (got $($p.ExitCode))"
Check (-not (Test-Path $exe)) 'app executable removed'
Check (-not (Test-Path $UninstallKey)) 'Add/Remove Programs entry removed'
Check (-not (Test-Path $StartMenuLink)) 'Start menu shortcut removed'
Check (-not (Test-Path $DesktopLink)) 'desktop shortcut removed'
Check (Test-Path $Db) 'profile database kept after uninstall'
Check ((Sql $marker) -eq '1') 'profile contents kept after uninstall'
Check (Test-Path (Join-Path $Artwork 'marker.png')) 'artwork kept after uninstall'
Write-Host 'All Windows install checks passed.'
