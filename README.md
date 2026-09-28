# RVN

**Off-grid field cyberdeck · FIELD//OS**

Modern, professional, futuristic operator interface for a Raspberry Pi 5 cyberdeck in a Pelican-style case.

## V0.1 Surfaces

| Surface   | Purpose                          |
|-----------|----------------------------------|
| **NAV**   | GPS · offline maps · waypoints   |
| **RADIO** | Receive-only SDR · spectrum      |
| **MESH**  | Meshtastic nodes · messaging     |
| **TERMINAL** | Explicit operator shell       |
| **SYSTEM** | Health · power · hardware status |

## Design Goals

- Modern, professional, dynamic UI
- Offline-first / air-gap by default
- Explicit operator control
- Fail-soft hardware adapters
- Keyboard + touch optimised for field use

## Quick Start (development)

```bash
# Requires Rust 1.92+
cargo run
```

Runs windowed. On the Pi appliance it will later launch full-screen.

## Project Layout

```
rvn/
├── ui/                 # Spint UI (theme, components, app)
├── src/
│   ├── main.rs
│   ├── core/           # domain, state, events (coming)
│   ├── adapters/       # hardware abstractions (coming)
│   └── surfaces/       # surface-specific logic (coming)
├── assets/
└── docs/
```

## Stack

- **Rust** 2024 edition
- **Slint 1.18** — declarative, high-performance UI
- Tokio · tracing · serde

## License

Apache-2.0
