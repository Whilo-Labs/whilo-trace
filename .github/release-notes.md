Whilo Trace watches the network path to several hosts at once: per-hop loss and latency in real time, a timeline from 5 minutes to 24 hours, local history and a report for your ISP.

## What's new in 0.2.1

This release fixes figures that could be wrong in the trace and in the report for your ISP.

- **Where the loss starts.** A single round that found the destination one hop deeper than usual no longer turns real loss into "answers some probes" and hides where it starts. A router that limits its own answers is no longer named as the start of the loss. When only such routers stand before the destination, the app says that the start cannot be told from the hops.
- **A route that got longer.** A live destination no longer shows 100% loss until a restart when the route grew behind a router that stays silent: after 5 lost rounds in a row, a round goes the full depth once a minute. A round that stopped short of a longer route no longer counts as lost.
- **Reports over an outage.** For a period where no round reached the destination, the hop table goes on to the destination and no longer calls the hop of the outage the destination. A window inside a long outage on the screen does the same.
- **Findings for a picked period.** The report's findings now name the hops of that period, not of the whole window.
- **Pinned hops in the report.** On the chart in `report.html`, a hop that misses probes while the destination answers gets amber marks, as on the screen, instead of red loss.
- **Numbers in the findings.** A tiny loss no longer reads as 0% and a loss just under 100% no longer reads as 100%. The period of a report ends no later than the moment it is made, and the method line takes the interval from the rounds of the period.
- **One running copy.** Starting the app a second time brings up its window instead of probing every target twice.
- **Command line.** `watch` no longer adds a target to the app's main screen or resumes a paused one.
- **Picture for publishing.** Your home city and your own network stay hidden in a few more cases, and a far hop in a district that shares its name with one at home keeps its city.

## Downloads

| System | File | Notes |
|---|---|---|
| Windows 10 and 11 (x64) | `whilo-trace-*-windows-x64-setup.exe` | Installer for the current user, no administrator rights needed |
| | `whilo-trace-*-windows-x64-portable.zip` | No installation: unpack and run `whilo-trace.exe`; the command line tool is inside too |
| Linux (x86_64) | `whilo-trace-*-linux-x86_64.AppImage` | Make it executable and run it |
| | `whilo-trace_*_amd64.deb` | Debian, Ubuntu and derivatives |
| macOS (Apple silicon and Intel) | `whilo-trace-*-macos-universal.dmg` | Drag the app to Applications |
| Command line, Linux and macOS | `whilo-trace-cli-*.tar.gz` | `whilo-trace-cli watch 1.1.1.1` |

The app files now include the places database of the route map (DB-IP Lite, about 130 MB unpacked), so they are larger than before. The command line files do not include it.

## Before the first start

- **Windows.** The files are not signed yet, so SmartScreen may say "Windows protected your PC". Choose "More info", then "Run anyway".
- **macOS.** The app is not notarized yet. Open it once, then choose **Open Anyway** in System Settings, Privacy & Security. On macOS 14 and earlier, right-click the app and choose Open instead.
- **Linux.** Probes use unprivileged ICMP sockets. If the app reports that probes are unavailable, allow them for your group, for example `sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"`.

## Tested

Tested by the author on Windows 11. The Linux and macOS builds come from the same automated build. The engine, history, places database and command line pass the automated tests on those systems, and the app's own tests run on Windows only; neither system has been tried on a real machine yet. The route map is new: so far it has been checked only with made-up places, not on real traces. The in-app update of the places database has only been tested against a local test server. If something does not work, please [open an issue](https://github.com/Whilo-Labs/whilo-trace/issues/new/choose).
