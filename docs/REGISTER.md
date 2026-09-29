# RVN register

The shell should still know the day after a reboot, with the network off. These eight items are that work, in the order they are built.

1. **Remember the operation.** Waypoints, the track, the selected mark, the map view, the dial, named frequencies, the mesh log, and the night posture are written to `operation.json` under the RVN data directory and read back at boot.
2. **Walk back to a mark.** Choosing a waypoint shows distance and bearing, and draws a line from the fix to that point.
3. **Save the map area.** AREA stores the street tiles around the fix. The fetch stays paced and local.
4. **Heard stations on the map.** Position packets from the mesh are plotted under their short name.
5. **Radio memory.** A scan peak or the current dial can be kept by name and tuned again. A short waterfall holds the last half-minute of the spectrum. Strength only.
6. **Home notices.** The posture cell shows a new mesh message, a stale fix, or a module that was here and then removed.
7. **Night posture.** N, or the NIGHT control, switches the shell to a dim red theme. The keys and layout stay the same.
8. **Measured endurance.** SYSTEM shows hours only when `endurance.json` sits beside the operation file and names a measured pack and a measured draw. Otherwise the line stays blank.

The data directory is `$XDG_DATA_HOME/rvn`, or `~/.local/share/rvn`, or `%LOCALAPPDATA%\rvn`.

`endurance.json` is written after a bench measurement, not by the shell:

```json
{ "pack_wh": 99.0, "draw_w": 8.5 }
```

Hours are `pack_wh / draw_w`. A missing file, or a zero draw, leaves the figure blank. Charge stays blank until a live gauge exists.
