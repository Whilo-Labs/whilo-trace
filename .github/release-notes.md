Whilo Trace watches the network path to several hosts at once: per-hop loss and latency in real time, a timeline from 5 minutes to 24 hours, local history and a report for your ISP.

## What's new in 0.2.0

- **Route map.** On the trace screen, switch the hop card from **Table** to **Map** to see the path on a world map, with the hops numbered as in the table. A click on a hop shows its place, its network (AS) and a speed-of-light check that marks places the measured round trip rules out. After a round of the target a packet runs along the route, to the target or to the hop where that round was lost. You can zoom, drag the map and open it full screen.
- **Long routes.** A route that crosses an ocean can open on its main part, with a pointer at the edge of the map to the rest, when the whole route would leave that part small. **Main part** and **Whole route** switch between the two.
- **Places, offline.** Router places come from the DB-IP Lite databases, now included with the app. There are no online maps: lookups read only files on your computer, and the map sends no address of a target or a hop anywhere.
- **Monthly database updates.** DB-IP publishes new databases every month. When a trace screen first shows the map, the app checks for a newer month, at most once a day, and asks before downloading anything. The check can be turned off in the settings, **Places database**.
- **Picture for publishing.** The **Save picture** button makes a dark PNG of the map to share, 2400×1350 or 2160×2160 pixels. It hides the addresses, names and places of your own network and of your provider's first hops, draws home at one point for your whole country and leaves out distances from home. A hop that the database puts in your home city or within 50 km of your home shows only its country, without a name, and keeps its address.
- **Map in the report.** A report for your ISP saved from the trace screen now includes the map, light and with nothing hidden.
- **Simulated network.** To try the app without sending anything, start it with the environment variable `WHILO_TRACE_SIMULATE` set to `1`. It then runs on a made-up network, sends no probes or DNS queries and keeps a separate history. A word in the host name, such as `loss`, `down` or `unreachable`, picks what the path does; the README lists them.
- **New hop status.** A router that answers that it cannot reach the destination is now shown as **says the destination is unreachable**, and the path ends there. The findings and the report name that router.
- **Fixed: rounds that end early.** A round that ends before the destination now counts as sent and lost at every hop from where it ends to the destination, so the destination row of the trace shows the same lost rounds as the target row on the main screen. In 0.1.0 those rounds were missing from the hops past their end: loss on the path could look like a hop that answers only some probes, and a router that says the destination is unreachable passed for a destination without loss. History saved by 0.1.0 is counted the new way too.

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
- **macOS.** The app is not notarized yet. Open it with right-click, Open, the first time.
- **Linux.** Probes use unprivileged ICMP sockets. If the app reports that probes are unavailable, allow them for your group, for example `sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"`.

## Tested

Tested by the author on Windows 11. The Linux and macOS builds come from the same automated build, and the core of the app passes the automated tests on those systems, but they have not been tried on real machines yet. The route map is new: so far it has been checked only with made-up places, not on real traces. The in-app update of the places database has only been tested against a local test server. If something does not work, please [open an issue](https://github.com/Whilo-Labs/whilo-trace/issues/new/choose).
