# Security

If you find a security problem in Oriel, please report it privately rather than in a public
issue: on GitHub, open the **Security** tab and choose **Report a vulnerability**. You'll get a
reply as soon as possible, and credit in the fix's release notes if you'd like it.

Useful things to include: what an attacker could do, the steps to reproduce it, and the Oriel
and Windows versions you tried.

## What Oriel does and doesn't do

- It doesn't connect to the internet. The only network request it makes is to
  LibreHardwareMonitor's web server, at the address in Settings (your own PC by default).
- The dashboard page can't run programs. It can ask Windows to open Task Manager, Resource
  Monitor or a Settings page, and nothing else (`run_action` in `src-tauri/src/main.rs`).
- It runs as you, without admin rights, and installs per user.
- Releases are built by GitHub Actions from the tagged commit. Each release lists the SHA-256 of
  its files in `SHA256SUMS.txt`. They aren't code-signed yet.
