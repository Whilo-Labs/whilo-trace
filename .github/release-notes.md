Whilo Trace watches the network path to several hosts at once: per-hop loss and latency in real time, a timeline from 5 minutes to 24 hours, local history and a report for your ISP.

## What's new in 0.4.0

The app now keeps watching with its window closed, tells you when something breaks, and collects better evidence for your ISP.

### In the background

- **Runs in the notification area.** Closing the window leaves the app in the tray, still probing, so the history has no holes. Click the icon to open the window; its menu has Quit. The settings can make closing quit instead.
- **Starts with the system.** Off by default. Turn it on in the settings, and the app starts in the tray when you sign in and resumes its targets.
- **Alerts.** A notification comes when a target does not answer for over a minute, or when the loss to it over the last 2 minutes reaches 10% (5, 10 or 20% in the settings). It names the hop where the loss starts, and another one comes when it is over. Only the destination counts, so a router that limits its own answers raises nothing, and an outage gives one alert rather than a stream of them. While a target has an alert, the tray icon has a red dot.

### Evidence for your ISP

- **Where each hop is.** The trace table, the report and `hops.csv` show where each hop is and whose network it is on, from the places database of the route map.
- **One router behind several targets.** When the loss of two or more targets starts at the same router, the summary, the trace findings and the report say so: the trouble is most likely at that router or before it.
- **Route changes.** The trace findings and the report list the changes of the route over the window: a new address of the target, another router at a hop, the destination at another hop. A load balancer that alternates between routers is not a change.
- **Before and after.** The trace screen compares the window, or the period you picked, with the same time a day or a week before: loss and latency to the target, and hop by hop what got worse, better or changed.
- **A week and a month of history.** The history strip above the timelines now shows the last day, week or month.
- **Safe CSV files.** A CSV cell that starts like a formula gets a leading quote, so a spreadsheet does not run it.

### Smaller things

- Shortcuts: Ctrl+N for a new target, Ctrl+1 to Ctrl+6 for the window, Ctrl+, for the settings, in any keyboard layout.
- The portable zip keeps the history, settings and the window's browser data in a `data` folder next to the program.
- `error.log` next to the history records what went wrong. Attaching it to an issue helps.
- Each release has `SHA256SUMS.txt` with the checksums of its files.
- The command line tool for Windows (in the portable zip) no longer needs the Visual C++ Redistributable.
- The status bar asks once whether to tell you about new versions. With yes, the app asks GitHub once a day and shows "Version X is out" with a link. Nothing is downloaded by itself.

The first start of 0.4.0 adds an index to the history file, so a large history may take a few seconds longer to open once.

**Updating from 0.2.x:** five minutes after the first start, history older than 30 days is deleted (this came in 0.3.0). To keep it, open the settings right after the start and choose "always" under History.

0.3.0 added a ceiling on probes for all targets together, kept rounds without a network out of the loss, and keeps the history for 30 days; see its [release notes](https://github.com/Whilo-Labs/whilo-trace/releases/tag/v0.3.0).

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

Tested by the author on Windows 11. The Linux and macOS builds come from the same automated build. The engine, history, places database and command line pass the automated tests on those systems, and the app's own tests run on Windows only; neither system has been tried on a real machine yet. The tray, the start with the system, the alerts, the route changes, the comparison and the week and month history were checked on Windows with the simulated network built into the app; how the notifications look in an installed copy, and the tray on Linux and macOS, have not been tried yet. The check for new versions has been tested only with made-up answers, not against GitHub itself. How Windows reports probes on a computer without a network, and sleep on a laptop, have not been tried on a real machine either. The route map has so far been checked only with made-up places, not on real traces. The in-app update of the places database has only been tested against a local test server. If something does not work, please [open an issue](https://github.com/Whilo-Labs/whilo-trace/issues/new/choose).
