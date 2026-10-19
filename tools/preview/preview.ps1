<#
.SYNOPSIS
Runs the dashboard in a browser on made-up data (demo.js), with no Rust build needed.

.DESCRIPTION
Writes a copy of ui/index.html to %TEMP%\oriel-preview that loads the real ui/ files plus demo.js
in place of the Rust backend, then opens it in your default browser. Edit anything in ui/ and
reload the page to see it.

With -Screenshots it renders the README images and the repo's social preview in docs/images
instead, using headless Edge (or Chrome).

With -Installer it renders the installer's sidebar and header images (24-bit BMPs, which is what
NSIS needs) into src-tauri/windows.

.EXAMPLE
powershell -ExecutionPolicy RemoteSigned -File tools\preview\preview.ps1

.EXAMPLE
powershell -ExecutionPolicy RemoteSigned -File tools\preview\preview.ps1 -Screenshots

.EXAMPLE
powershell -ExecutionPolicy RemoteSigned -File tools\preview\preview.ps1 -Installer
#>
param([switch]$Screenshots, [switch]$Installer)

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

if (-not $Screenshots -and -not $Installer) {
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

# ---- installer images -----------------------------------------------------------------
# NSIS wants 24-bit BMPs: 164x314 for the welcome and finish pages, 150x57 for the others
# (on a white header). Each is drawn at 4x in the browser and scaled down with System.Drawing,
# which gives smoother text than rendering at 1x.
if ($Installer) {
    Add-Type -AssemblyName System.Drawing
    $windows = Join-Path $root "src-tauri\windows"
    $logo = Url (Join-Path $root "assets\oriel.svg")
    $fonts = "'Segoe UI Variable Display','Segoe UI',sans-serif"
    $S = 4
    $pages = [ordered]@{
        "installer-sidebar" = @{ w = 164; h = 314; html =
            "html,body{margin:0;width:164px;height:314px;overflow:hidden}" +
            "body{background:radial-gradient(circle at 100% 0,#9085e938,transparent 70%),radial-gradient(circle at 0 100%,#3987e52e,transparent 65%),linear-gradient(#1a1a21,#0c0c10);" +
            "color:#f0efec;font-family:$fonts;text-align:center}" +
            "img{display:block;width:68px;height:68px;margin:70px auto 18px;border-radius:16px;box-shadow:0 0 0 1px #ffffff1a,0 8px 22px #0008}" +
            "h1{margin:0 0 10px;font-weight:600;font-size:34px;line-height:1;letter-spacing:-.5px}" +
            "p{margin:0 auto;width:118px;font-size:11.5px;line-height:1.4;color:#b4b8c2}" +
            "<img src=`"$logo`"><h1>Oriel</h1><p>A command center for your second screen</p>" }
        "installer-header" = @{ w = 150; h = 57; html =
            "html,body{margin:0;width:150px;height:57px;overflow:hidden;background:#fff}" +
            "body{display:flex;align-items:center;justify-content:flex-end;gap:9px;padding-right:14px;box-sizing:border-box;font-family:$fonts}" +
            "img{width:34px;height:34px;border-radius:8px}" +
            "b{font-weight:600;font-size:21px;letter-spacing:-.3px;color:#1f2328}" +
            "<img src=`"$logo`"><b>Oriel</b>" }
    }
    foreach ($name in $pages.Keys) {
        $pg = $pages[$name]
        Write-Host "Rendering $name"
        # The html key holds the CSS up to the first "<" and the markup after it.
        $i = $pg.html.IndexOf("<")
        $p = Join-Path $out "$name.html"
        Write-Page $p ("<!doctype html><meta charset=`"utf-8`"><style>" + $pg.html.Substring(0, $i) + "</style>" + $pg.html.Substring($i))
        $png = Join-Path $raw "$name.png"
        # A roomy window, cropped below: headless Edge won't always go as small as 150px wide.
        Shot (Url $p) $png 800 600 $S
        $src = [Drawing.Image]::FromFile($png)
        $bmp = New-Object Drawing.Bitmap($pg.w, $pg.h, [Drawing.Imaging.PixelFormat]::Format24bppRgb)
        $g = [Drawing.Graphics]::FromImage($bmp)
        $g.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $g.DrawImage($src, (New-Object Drawing.Rectangle(0, 0, $pg.w, $pg.h)), 0, 0, $pg.w * $S, $pg.h * $S, [Drawing.GraphicsUnit]::Pixel)
        $g.Dispose(); $src.Dispose()
        $bmp.Save((Join-Path $windows "$name.bmp"), [Drawing.Imaging.ImageFormat]::Bmp)
        $bmp.Dispose()
    }
    Get-ChildItem $windows -Filter *.bmp | Format-Table Name, @{ n = "KB"; e = { [math]::Round($_.Length / 1KB) } } -AutoSize
    if (-not $Screenshots) { return }
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
# The default second page, Desk.
foreach ($mode in "dark", "light") {
    Write-Host "Rendering widgets page ($mode)"
    Shot "$(Url $page)?palette=oriel&theme=$mode&page=2&$time" (Join-Path $raw "widgets-$mode.png") $W $H
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
Framed "widgets-dark" (Join-Path $raw "widgets-dark.png")
Framed "widgets-light" (Join-Path $raw "widgets-light.png")

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

# The picture links to the repo show (GitHub: Settings > General > Social preview, which wants
# a PNG under 1 MB at 1280x640). The dashboard runs off the right and bottom edges.
Write-Host "Rendering social preview"
$p = Join-Path $out "social-preview.html"
Write-Page $p ("<!doctype html><meta charset=`"utf-8`"><style>" +
    "html,body{margin:0;width:1280px;height:640px;overflow:hidden}" +
    "body{background:radial-gradient(circle at 88% 10%,#9085e938,transparent 50%),radial-gradient(circle at 0 100%,#3987e52e,transparent 45%),linear-gradient(#1a1a21,#0c0c10);" +
    "color:#f0efec;font-family:'Segoe UI Variable Text','Segoe UI',sans-serif}" +
    ".t{position:absolute;left:80px;top:0;bottom:0;width:460px;display:flex;flex-direction:column;justify-content:center}" +
    ".t img{width:104px;height:104px;margin-bottom:28px}" +
    "h1{margin:0 0 18px;font:600 100px/1 'Segoe UI Variable Display','Segoe UI',sans-serif;letter-spacing:-2px}" +
    "p{margin:0;font-size:33px;line-height:1.3;color:#b4b8c2}" +
    ".s{margin-top:30px;font-size:21px;color:#7c818d}" +
    ".d{position:absolute;left:600px;top:118px;width:960px;height:540px;border-radius:16px;box-shadow:0 30px 90px #000a,0 0 0 1px #ffffff14}" +
    "</style><img class=`"d`" src=`"$(Url (Join-Path $raw 'oriel-dark.png'))`">" +
    "<div class=`"t`"><img src=`"$(Url (Join-Path $root 'assets\oriel.svg'))`"><h1>Oriel</h1>" +
    "<p>A command center for your second screen, on Windows.</p><div class=`"s`">Free and open source &middot; Rust + Tauri</div></div>")
Shot (Url $p) (Join-Path $images "social-preview.png") 1280 640

Get-ChildItem $images | Format-Table Name, @{ n = "KB"; e = { [math]::Round($_.Length / 1KB) } } -AutoSize
