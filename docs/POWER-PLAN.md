# RVN Power Measurement Plan

Status: **blocking battery selection**  
Goal: measure real draw of the lid-open cyberdeck before buying a pack.

This is a bench plan. No field endurance claims until these numbers exist.

---

## Why this comes first

RVN V1 is a personal cyberdeck in a Tactix / Pelican-style case.

The unit is only honest if:

- it boots and looks right when the lid opens
- attached modules do not brown-out the Pi
- the eventual battery is sized from measurements, not guesswork

The old Project-RAV3N BOM already marked power as TBD. That stays true until this plan is executed.

---

## Test platform (locked)

| Item | Current baseline |
|------|------------------|
| Compute | Raspberry Pi 5 8 GB + Active Cooler |
| Display | Freenove 7" 800×480 touch |
| Case | Tactix Tough Case — Medium |
| USB | Anker 4-port data hub + JESWO powered 7-port hub |
| Modules under test | USB GNSS, NESDR SMArt v5, Heltec LoRa/Meshtastic, Picade audio |

Do not add the AI HAT+ until PCIe / NVMe conflict is resolved. It will distort the power story.

---

## What to buy for measurement (small, cheap)

You need a way to see current at 5 V (Pi) and at the powered-hub input.

Minimum kit:

1. **USB-C PD power meter** in line with the Pi supply  
   (shows volts, amps, watts)
2. **DC inline meter or bench PSU** on the powered hub input  
   (hub often has its own 12 V / 5 V brick — measure that separately)
3. Notebook or a simple log file  
   timestamp · scenario · volts · amps · watts · notes (throttle? undervoltage?)

Optional but useful:

- Second meter so Pi and hub can be logged at the same time
- IR thermometer or just watch `vcgencmd measure_temp`
- Stopwatch for 10-minute soaks

If you already have a bench PSU with current display, use that instead of extra meters.

---

## Safety / setup rules

- Measure on the bench, lid open, case not sealed.
- One new load at a time. Never jump from “Pi only” to “everything”.
- Watch for lightning-bolt / undervoltage and for `throttled=yes`.
- If the Pi browns out, stop. Record the last stable combination.
- RADIO stays receive-only. MESH TX stays disarmed unless a scenario explicitly says ARM TX.
- Do not run long TX tests inside the closed case.

---

## Scenarios

Run each scenario for **2 minutes settle + 10 minutes soak**.  
Record min / typical / peak watts.

| ID | Name | What’s on | Why |
|----|------|-----------|-----|
| P0 | Idle compute | Pi + cooler + OS only. No display, no hubs, no modules | Floor |
| P1 | Display idle | P0 + 7" display + RVN full-screen on HOME | Show-piece baseline |
| P2 | + keyboard | P1 + 65% keyboard | Real lid-open kit |
| P3 | + powered hub | P2 + powered 7-port hub, no peripherals | Hub overhead |
| P4 | + GNSS | P3 + one USB GPS, NAV open, waiting for fix | First field module |
| P5 | + SDR idle | P4 + NESDR attached, RADIO open, not streaming | Dongle tax |
| P6 | + SDR RX | P5 + Start RX | Peak radio path |
| P7 | + Mesh RX | P6 + Heltec connected, MESH open, TX SAFE | Comms RX |
| P8 | + Mesh TX burst | P7 + ARM TX, send 3 short messages, then DISARM | Worst short spike |
| P9 | + Audio idle | P8-level kit + Picade enumerated, no playback | Audio tax |
| P10 | Show piece | Display + keyboard + hub + GPS + SDR idle + Mesh RX + RVN on HOME/SYSTEM | The photo |
| P11 | Workshop max | P10 + SDR streaming + SYSTEM open + TERMINAL running `uptime` | Worst continuous |

Skip P8 if you are not ready to transmit. Note “not run”.

---

## Log sheet

Copy this table into a notebook or `docs/POWER-LOG.md`.

```
date:
meter:
supply:

id | volts | amps_min | amps_typ | amps_peak | watts_typ | temp_c | throttled | notes
P0 |
P1 |
P2 |
P3 |
P4 |
P5 |
P6 |
P7 |
P8 |
P9 |
P10|
P11|
```

Also write:

- Did the display flicker?
- Did USB devices drop?
- Did RVN stay up?
- Which port / cable was used?

---

## How to read the numbers

After P10 and P11 you can size a pack.

Use **typical watts from P10** as the show-piece load  
and **typical watts from P11** as the workshop load.

Rough runtime:

```
hours ≈ (battery_wh × 0.85) / load_watts
```

0.85 is a blunt allowance for conversion loss. Replace it later if you measure the pack path.

Example only (do not treat as real):

- P10 typical = 12 W  
- 99 Wh pack × 0.85 / 12 ≈ 7 hours lid-open demo

Do not buy the pack until P10 exists.

Also record **peak from P11**. The supply and cables must survive the peak, not just the average.

---

## Pass / fail for V1 cyberdeck

V1 power is acceptable when:

- [ ] P1 runs 10 minutes with no undervoltage
- [ ] P10 runs 10 minutes, all enumerated devices stay up
- [ ] P11 does not throttle into uselessness (temp noted)
- [ ] You know whether the powered hub **must** have its own brick
- [ ] Battery capacity is chosen from P10/P11, not from a product page

---

## Order of work this week

1. Get meters in line (Pi supply first).
2. Run P0 → P1 → P2. Confirm the show-piece display load.
3. Add the powered hub (P3). Decide if unpowered Anker is only for low-draw items.
4. Commission GNSS on the bench (P4) while measuring.
5. Add SDR (P5/P6), then Heltec (P7).
6. Fill P10. That is the number that matters for the case demo.
7. Only then look at USB-C PD power banks / packs.

---

## Out of scope for this plan

- Final pack brand
- Closed-case thermal soak (do that after P11 on the bench)
- AI HAT+
- Monitor-mode Wi-Fi adapter
- Multi-hour field runtime claims
