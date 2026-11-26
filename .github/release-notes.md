Whilo Trace watches the network path to several hosts at once: per-hop loss and latency in real time, a timeline from 5 minutes to 24 hours, local history and a report for your ISP.

## What's new in 0.3.0

This release keeps the app from distorting what it measures.

- **A ceiling on probes.** All targets together send at most 50 packets per second (25, 50, 100 or 200 in the settings), split evenly between the targets. Too many probes can make a home router drop packets, and the app would then report loss it caused itself. Rounds over the ceiling are skipped and show as gaps, not as loss, and the status bar says when that happens.
- **Fewer probes to silent targets.** A target that has never answered is probed a little past the last router that answered, and the full 30 hops once a minute, instead of 30 probes every round. Targets no longer start their rounds all at the same moment.
- **The real packet rate.** The status bar shows the packets actually sent over the last 10 seconds.
- **No network is not loss.** When the computer cannot send a probe at all (a cable out, Wi-Fi off), those rounds are no longer counted as loss from the first hop: the timeline shows a gap and the target says "no network". Rounds cut short by sleep or by a jump of the clock are dropped.
- **History for 30 days.** Rounds older than 30 days are deleted in the background; the settings offer 7, 30, 90 or 365 days, or always. They also show the size of the history file and can clear it.
- **Command line.** `watch` takes `--max-pps` (50 by default) for all its hosts together.

**Before you update:** five minutes after the first start of 0.3.0, history older than 30 days is deleted. To keep it, open the settings right after the start and choose "always" under History.

## Downloads

| System | File | Notes |
|---|---|---|
| Windows 10 and 11 (x64) | `whilo-trace-*-windows-x64-setup.exe` | Installer for the current user, no administrator rights needed |
| | `whilo-trace-*-windows-x64-portable.zip` | No installation: unpack and run `whilo-trace.exe`; the command line tool is inside too |
| Linux (x86_64) | `whilo-trace-*-linux-x86_64.AppImage` | Make it executable and run it |
| | `whilo-trace_*_amd64.deb` | Debian, Ubuntu and derivatives |
| macOS (Apple silicon and Intel) | `whilo-trace-*-macos-universal.dmg` | Drag the app to Applications |
| Command line, Linux and macOS | `whilo-trace-cli-*.tar.gz` | `whilo-trace-cli watch 1.1.1.1` |

The app files include the places database of the route map (DB-IP Lite, about 130 MB unpacked); the command line files do not.

## Before the first start

- **Windows.** The files are not signed yet, so SmartScreen may say "Windows protected your PC". Choose "More info", then "Run anyway".
- **macOS.** The app is not notarized yet. Open it once, then choose **Open Anyway** in System Settings, Privacy & Security. On macOS 14 and earlier, right-click the app and choose Open instead.
- **Linux.** Probes use unprivileged ICMP sockets. If the app reports that probes are unavailable, allow them for your group, for example `sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"`.

## Tested

Tested by the author on Windows 11. The Linux and macOS builds come from the same automated build. The engine, history, places database and command line pass the automated tests on those systems, and the app's own tests run on Windows only; neither system has been tried on a real machine yet. The probe ceiling, the "no network" handling and the history clean-up of this release were checked on the simulated network built into the app; how Windows reports probes on a computer without a network, and sleep on a laptop, have not been tried on a real machine yet. The route map has so far been checked only with made-up places, not on real traces. The in-app update of the places database has only been tested against a local test server. If something does not work, please [open an issue](https://github.com/Whilo-Labs/whilo-trace/issues/new/choose).
