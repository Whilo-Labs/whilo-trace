Whilo Trace watches the network path to several hosts at once: per-hop loss and latency in real time, a timeline from 5 minutes to 24 hours, local history and a report for your ISP.

## Downloads

| System | File | Notes |
|---|---|---|
| Windows 10 and 11 (x64) | `whilo-trace-*-windows-x64-setup.exe` | Installer for the current user, no administrator rights needed |
| | `whilo-trace-*-windows-x64-portable.zip` | No installation: unpack and run `whilo-trace.exe`; the command line tool is inside too |
| Linux (x86_64) | `whilo-trace-*-linux-x86_64.AppImage` | Make it executable and run it |
| | `whilo-trace_*_amd64.deb` | Debian, Ubuntu and derivatives |
| macOS (Apple silicon and Intel) | `whilo-trace-*-macos-universal.dmg` | Drag the app to Applications |
| Command line, Linux and macOS | `whilo-trace-cli-*.tar.gz` | `whilo-trace-cli watch 1.1.1.1` |

## Before the first start

- **Windows.** The files are not signed yet, so SmartScreen may say "Windows protected your PC". Choose "More info", then "Run anyway".
- **macOS.** The app is not notarized yet. Open it with right-click, Open, the first time.
- **Linux.** Probes use unprivileged ICMP sockets. If the app reports that probes are unavailable, allow them for your group, for example `sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"`.

## Tested

Tested by the author on Windows 11. The Linux and macOS builds come from the same automated build, and the core of the app passes the automated tests on those systems, but they have not been tried on real machines yet. If something does not work, please [open an issue](https://github.com/Whilo-Labs/whilo-trace/issues/new/choose).
