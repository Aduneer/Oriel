# Security

If you find a security problem in Oriel, please report it privately rather than in a public
issue: on GitHub, open the **Security** tab and choose **Report a vulnerability**. You'll get a
reply as soon as possible, and credit in the fix's release notes if you'd like it.

Useful things to include: what an attacker could do, the steps to reproduce it, and the Oriel
and Windows versions you tried.

## What Oriel does and doesn't do

- It doesn't connect to the internet unless you add the weather or calendar widget to a page, and
  then only while that widget is on a page and the dashboard is on screen (`src-tauri/src/net.rs`,
  `feed.rs`). Weather asks Open-Meteo for a forecast, sending the chosen place's coordinates
  rounded to two decimals, and, when you search for a place, the words you typed
  (`weather.rs`). Calendar fetches the https:// links you paste into Settings (`calendar.rs`).
  Nothing else is sent, apart from what any request carries: your IP address, and headers
  including a `User-Agent` naming Oriel and its version.
- Those requests go through Windows' own HTTP client, so they use your PC's certificates and proxy
  settings, and no extra code is linked in for them. Only https:// is fetched, redirects
  included (a webcal:// link is read as https://). No command lets the page fetch an address of its
  own choosing: weather and place search go to fixed Open-Meteo addresses, and calendars to the
  links in the settings. Each reply is cut off at a size limit (256 KB for weather, 5 MB for a
  calendar) and each request has a deadline. No cookies are sent or kept.
- A calendar link is a secret: whoever has it can read the calendar. It's stored in plain text in
  `%APPDATA%\Oriel\settings.json`, like the rest of the settings, so anyone who can read your
  files can read it. Oriel shows it only in the settings panel (hidden until you press *Show*),
  never writes it to a log, and keeps it out of error messages. The dashboard's calendar tile is
  sent each calendar's name and color, never its link.
- Whatever weather and calendar fetch is only ever read as data: calendar text is shown as text, and
  a calendar file can't make Oriel do anything. Fetched data is kept in memory, not on disk.
- The one other network request is to LibreHardwareMonitor's web server, at the address in
  Settings (your own PC by default).
- The dashboard page can't run programs. It can ask Windows to open Task Manager, Resource
  Monitor or a Settings page, and nothing else (`run_action` in `src-tauri/src/main.rs`).
- It can end a process you pick in the process list, after you confirm, with your own rights
  (`end_processes`). It checks the process is still the one you saw, since Windows reuses
  process ids.
- With the Now playing widget on the dashboard, it reads what's playing from Windows' media
  controls and can play, pause or skip it, like a keyboard's media keys (`media_control`). Without
  the widget it doesn't look. What's playing never leaves your PC.
- With the volume mixer on the dashboard, it reads and changes the volume of your default output
  and of the apps playing through it (`set_volume`). Without the widget it doesn't look.
- With the Open windows widget on the dashboard, it lists the titles of your open windows and
  can bring one to the front or ask it to close, as its × button does (`focus_window`,
  `close_window`). It never ends a process, and checks the window is still the one you saw before
  it acts, since Windows reuses window handles. Window titles stay on your PC. Without the widget
  it doesn't look.
- It runs as you, without admin rights, and installs per user.
- Releases are built by GitHub Actions from the tagged commit. Each release lists the SHA-256 of
  its files in `SHA256SUMS.txt`. They aren't code-signed yet.
