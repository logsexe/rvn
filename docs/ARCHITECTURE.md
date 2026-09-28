# RVN Architecture

## Overview

RVN is an offline-first field cyberdeck operating system shell.
It runs full-screen on a Raspberry Pi 5 inside a Pelican-style case.

```
┌─────────────────────────────────────────┐
│                 RVN UI                  │
│              (Slint shell)              │
├─────────────────────────────────────────┤
│              AppState                    │
│     (single source of truth)            │
├──────────────┬──────────────────────────┤
│   Surfaces   │      Adapters            │
│  NAV RADIO   │  GPS  SDR  Mesh  System  │
│  MESH TERM   │  (real or mock)          │
│  SYSTEM      │                          │
├──────────────┴──────────────────────────┤
│           Linux (Pi OS / Debian)        │
└─────────────────────────────────────────┘
```

## Core Principles

1. **Offline-first** — no network calls unless the operator explicitly enables them.
2. **Fail-soft** — missing hardware reports `NotPresent`, never crashes the UI.
3. **Explicit control** — radio TX, scans, shell commands are operator-initiated.
4. **Single source of truth** — `AppState` holds everything the UI needs.
5. **Adapter pattern** — real hardware and mocks implement the same interface.

## Module Map

```
src/
├── main.rs               # entry, UI wiring, ticker
├── core/
│   ├── mod.rs
│   ├── state.rs          # AppState, Surface enum
│   ├── hardware.rs       # PlatformStatus + readiness model
│   └── events.rs         # AppEvent definitions
├── adapters/
│   ├── mod.rs
│   └── mock.rs           # development / demo adapter
└── surfaces/             # per-surface logic (future)
```

## Hardware Readiness Model

Every subsystem reports one of:

| State       | Meaning                          | UI status |
|-------------|----------------------------------|-----------|
| NotPresent  | Not detected                     | offline  |
| Degraded    | Present but unhealthy            | warn      |
| Ready       | Healthy and idle                 | ok        |
| Active      | Currently streaming / in use     | active    |

## Surfaces (V0.1)

| ID | Surface   | Responsibility                     |
|----|-----------|------------------------------------|
| 0  | NAV       | GPS, offline maps, waypoints       |
| 1  | RADIO     | Receive-only SDR / spectrum        |
| 2  | MESH      | Meshtastic nodes + messaging       |
| 3  | TERMINAL  | Explicit operator shell            |
| 4  | SYSTEM    | Health, power, hardware inventory  |

## Next Evolution

- Shared state channel (tokio watch / broadcast) instead of UI-owned navigation
- Real adapters for gpsd, RTL-SDR, Meshtastic serial
- SYSTEM surface first (most useful while commissioning hardware)
- Full-screen kiosk launch on the Pi
