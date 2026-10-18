<#
.SYNOPSIS
Runs the dashboard in a browser on made-up data (demo.js), with no Rust build needed.

.DESCRIPTION
Writes a copy of ui/index.html to %TEMP%\oriel-preview that loads the real ui/ files plus demo.js
in place of the Rust backend, then opens it in your default browser. Edit anything in ui/ and
reload the page to see it.

With -Screenshots it renders the README images in docs/images instead, using headless Edge
(or Chrome).

.EXAMPLE
powershell -ExecutionPolicy RemoteSigned -File tools\preview\preview.ps1

.EXAMPLE
powershell -ExecutionPolicy RemoteSigned -File tools\preview\preview.ps1 -Screenshots
#>
param([switch]$Screenshots)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$ui = Join-Path $root "ui"
$out = Join-Path $env:TEMP "oriel-preview"
New-Item -ItemType Directory -Force $out | Out-Null
$utf8 = New-Object System.Text.UTF8Encoding($false)
function Url($path) { ([Uri]$path).AbsoluteUri }
function Write-Page($path, $html) { [IO.File]::WriteAllText($path, $html, $utf8) }

# The real page, with relative links pointing back at ui/ and the demo backend loaded first.
$html = [IO.File]::ReadAllText((Join-Path $ui "index.html"))
$base = "<base href=`"$(Url "$ui\")`">"
$demo = "<script src=`"$(Url (Join-Path $PSScriptRoot 'demo.js'))`"></script>"
if (-not $html.Contains("<head>") -or -not $html.Contains('<script src="core.js">')) {
    throw "ui/index.html no longer has <head> or <script src=`"core.js`">; update preview.ps1 to match."
}
$html = $html.Replace("<head>", "<head>`n$base").Replace('<script src="core.js">', "$demo`n<script src=`"core.js`">")
$page = Join-Path $out "index.html"
Write-Page $page $html

if (-not $Screenshots) {
    Start-Process (Url $page)
    Write-Host "Opened $page"
    Write-Host "Options go after the address: ?palette=nord&theme=light&settings=1"
    return
}

# ---- screenshots ----------------------------------------------------------------------
function Find-Browser {
    foreach ($exe in "msedge.exe", "chrome.exe") {
        foreach ($hive in "HKLM:", "HKCU:") {
            $p = (Get-ItemProperty "$hive\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\$exe" -ErrorAction SilentlyContinue).'(default)'
            if ($p -and (Test-Path $p)) { return $p }
        }
    }
    $candidates = @(
        "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
        "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
        "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
        "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe"
    )
    $candidates += Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft\EdgeCore\*\msedge.exe" -ErrorAction SilentlyContinue |
        Sort-Object { [version]$_.Directory.Name } -Descending | ForEach-Object FullName
    foreach ($p in $candidates) { if ($p -and (Test-Path $p)) { return $p } }
    throw "Couldn't find Edge or Chrome for the screenshots."
}
$browser = Find-Browser
$profileDir = Join-Path $out "profile"
Remove-Item $profileDir -Recurse -Force -ErrorAction SilentlyContinue # no sort order or view left over from last time
$raw = Join-Path $out "raw"
$images = Join-Path $root "docs\images"
New-Item -ItemType Directory -Force $raw, $images | Out-Null

function Shot($url, $file, $w, $h, $scale = 1, [switch]$Transparent) {
    $a = @("--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=$scale", "--lang=en-GB", "--log-level=3",
        "--window-size=$w,$h", "--virtual-time-budget=5000", "--user-data-dir=$profileDir", "--screenshot=$file")
    if ($Transparent) { $a += "--default-background-color=00000000" }
    $a = $a | ForEach-Object { if ($_ -match " ") { "`"$_`"" } else { $_ } }
    Remove-Item $file -ErrorAction SilentlyContinue
    & $browser @a $url | Out-Null # piping makes PowerShell wait for the browser to exit
    if (-not (Test-Path $file)) { throw "No screenshot came out for $url" }
}

$W = 1920; $H = 1080
$time = "time=2026-10-18T14:32"
$palettes = [ordered]@{ oriel = "Oriel"; catppuccin = "Catppuccin"; nord = "Nord"; gruvbox = "Gruvbox"; "rose-pine" = "Ros&eacute; Pine"; solarized = "Solarized" }
foreach ($mode in "dark", "light") {
    foreach ($id in $palettes.Keys) {
        Write-Host "Rendering $id ($mode)"
        Shot "$(Url $page)?palette=$id&theme=$mode&$time" (Join-Path $raw "$id-$mode.png") $W $H
    }
}
Write-Host "Rendering settings"
Shot "$(Url $page)?palette=oriel&theme=dark&settings=1&$time" (Join-Path $raw "settings-dark.png") $W $H
# The process tree on its own, at twice the pixels so it stays sharp at README size.
$TW = 900; $TH = 640
foreach ($mode in "dark", "light") {
    Write-Host "Rendering process tree ($mode)"
    Shot "$(Url $page)?palette=oriel&theme=$mode&layout=processes&procs=tree&click=Code,chrome&$time" (Join-Path $raw "tree-$mode.png") $TW $TH 2
}

# Frame the shots: rounded corners on a transparent background, and a grid of the themes.
$style = "html,body{margin:0;background:transparent;overflow:hidden}img{display:block;border-radius:18px}"
function Framed($name, $src, $w = $W, $h = $H, $scale = 1) {
    $p = Join-Path $out "$name.html"
    Write-Page $p "<!doctype html><meta charset=`"utf-8`"><style>$style</style><img src=`"$(Url $src)`" width=`"$w`" height=`"$h`">"
    Shot (Url $p) (Join-Path $images "$name.png") $w $h $scale -Transparent
}
Framed "dashboard-dark" (Join-Path $raw "oriel-dark.png")
Framed "dashboard-light" (Join-Path $raw "oriel-light.png")
Framed "settings" (Join-Path $raw "settings-dark.png")
Framed "process-tree-dark" (Join-Path $raw "tree-dark.png") $TW $TH 2
Framed "process-tree-light" (Join-Path $raw "tree-light.png") $TW $TH 2

$gap = 30; $tw = ($W - 2 * $gap) / 3; $th = [math]::Round($tw * $H / $W); $label = 52
$gh = 2 * ($th + $label) + $gap
foreach ($mode in "dark", "light") {
    $ink = if ($mode -eq "dark") { "#e6edf3" } else { "#1f2328" }
    $cells = foreach ($id in $palettes.Keys) {
        "<figure><img src=`"$(Url (Join-Path $raw "$id-$mode.png"))`"><figcaption>$($palettes[$id])</figcaption></figure>"
    }
    $p = Join-Path $out "themes-$mode.html"
    Write-Page $p ("<!doctype html><meta charset=`"utf-8`"><style>$style" +
        "body{display:grid;grid-template-columns:repeat(3,${tw}px);gap:${gap}px;font:600 26px 'Segoe UI Variable Text','Segoe UI',sans-serif;color:$ink}" +
        "figure{margin:0}figure img{width:${tw}px;height:${th}px;border-radius:12px}figcaption{height:${label}px;line-height:${label}px;padding-left:4px}" +
        "</style>" + ($cells -join ""))
    Shot (Url $p) (Join-Path $images "themes-$mode.png") $W $gh -Transparent
}

Get-ChildItem $images | Format-Table Name, @{ n = "KB"; e = { [math]::Round($_.Length / 1KB) } } -AutoSize
