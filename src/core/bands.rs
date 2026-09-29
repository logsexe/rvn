//! Public Australian spectrum notes and the receive-only sweep plans.
//! These are allocation names, not a channel list and not a demodulator.

use super::hardware::ScanHit;

pub struct BandChoice {
    pub id: &'static str,
    pub label: &'static str,
}

pub struct BandNote {
    pub range: &'static str,
    pub name: &'static str,
    pub purpose: &'static str,
}

pub struct ScanPlan {
    pub label: String,
    pub points: Vec<f32>,
}

const CHOICES: &[BandChoice] = &[
    BandChoice { id: "area", label: "This area" },
    BandChoice { id: "survey", label: "Common bands" },
    BandChoice { id: "fm", label: "FM broadcast" },
    BandChoice { id: "air", label: "Aviation" },
    BandChoice { id: "ham2m", label: "Amateur 2 m" },
    BandChoice { id: "marine", label: "Marine" },
    BandChoice { id: "ham70", label: "Amateur 70 cm" },
    BandChoice { id: "ism433", label: "ISM 433" },
    BandChoice { id: "uhfcb", label: "UHF CB" },
    BandChoice { id: "mesh915", label: "915 ISM" },
];

const NOTES: &[BandNote] = &[
    BandNote {
        range: "87.5–108 MHz",
        name: "FM broadcast",
        purpose: "Commercial, community, and national radio.",
    },
    BandNote {
        range: "108–137 MHz",
        name: "Aviation",
        purpose: "Civil aircraft communication.",
    },
    BandNote {
        range: "144–148 MHz",
        name: "Amateur 2 m",
        purpose: "Licensed amateur radio.",
    },
    BandNote {
        range: "156–162 MHz",
        name: "Marine",
        purpose: "Ship and coastal radio.",
    },
    BandNote {
        range: "174–230 MHz",
        name: "VHF broadcast",
        purpose: "DAB+ digital radio and television.",
    },
    BandNote {
        range: "403–430 MHz",
        name: "Land mobile",
        purpose: "Licensed business radio.",
    },
    BandNote {
        range: "430–450 MHz",
        name: "Amateur 70 cm",
        purpose: "Licensed amateur radio.",
    },
    BandNote {
        range: "433.05–434.79 MHz",
        name: "LIPD",
        purpose: "Short-range devices, sensors, and telemetry.",
    },
    BandNote {
        range: "476.425–477.4125 MHz",
        name: "UHF CB",
        purpose: "Licence-free 80-channel citizen band.",
    },
    BandNote {
        range: "520–694 MHz",
        name: "Television",
        purpose: "UHF television broadcasting.",
    },
    BandNote {
        range: "703–960 MHz",
        name: "Mobile",
        purpose: "Public mobile telephone services.",
    },
    BandNote {
        range: "915–928 MHz",
        name: "ISM",
        purpose: "Short-range devices. Australian Meshtastic uses this segment.",
    },
    BandNote {
        range: "2400–2483.5 MHz",
        name: "2.4 GHz",
        purpose: "Wi-Fi, Bluetooth, and other short-range gear. Above this receiver.",
    },
    BandNote {
        range: "26.965–27.405 MHz",
        name: "27 MHz CB",
        purpose: "Licence-free HF citizen band. At the edge of what an RTL-SDR can tune.",
    },
    BandNote {
        range: "3.5–3.8 · 7.0–7.3 · 14.0–14.35 MHz",
        name: "Amateur HF",
        purpose: "Long-range amateur radio. Needs an HF receiver.",
    },
];

pub fn band_choices() -> &'static [BandChoice] {
    CHOICES
}

pub fn au_notes() -> &'static [BandNote] {
    NOTES
}

pub fn scan_plan(id: &str, center_mhz: f32) -> ScanPlan {
    let center = center_mhz.clamp(24.0, 1700.0);
    match id {
        "fm" => plan("FM broadcast", &[(87.5, 108.0, 0.2)]),
        "air" => plan("Aviation", &[(118.0, 137.0, 0.5)]),
        "ham2m" => plan("Amateur 2 m", &[(144.0, 148.0, 0.1)]),
        "marine" => plan("Marine", &[(156.0, 162.025, 0.1)]),
        "ham70" => plan("Amateur 70 cm", &[(430.0, 450.0, 0.2)]),
        "ism433" => plan("ISM 433", &[(433.05, 434.79, 0.1)]),
        "uhfcb" => plan("UHF CB", &[(476.425, 477.4125, 0.025)]),
        "mesh915" => plan("915 ISM", &[(915.0, 928.0, 0.2)]),
        "survey" => plan(
            "Common bands",
            &[
                (87.5, 108.0, 0.5),
                (118.0, 137.0, 1.0),
                (144.0, 148.0, 0.25),
                (156.0, 162.0, 0.25),
                (430.0, 450.0, 1.0),
                (433.05, 434.79, 0.2),
                (476.425, 477.4125, 0.05),
                (915.0, 928.0, 0.5),
            ],
        ),
        _ => {
            let start = (center - 15.0).clamp(24.0, 1700.0);
            let end = (center + 15.0).clamp(start, 1700.0);
            plan("This area", &[(start, end, 1.0)])
        }
    }
}

fn plan(label: &str, spans: &[(f32, f32, f32)]) -> ScanPlan {
    let mut points = Vec::new();
    for &(start, end, step) in spans {
        if points.len() >= 240 {
            break;
        }
        points.extend(mhz_points(start, end, step, 240 - points.len()));
    }
    ScanPlan {
        label: label.into(),
        points,
    }
}

pub fn mhz_points(start: f32, end: f32, step: f32, cap: usize) -> Vec<f32> {
    let step = step.max(0.0125);
    let end = end.max(start);
    let mut out = Vec::new();
    let mut f = start;
    while f <= end + step * 0.01 && out.len() < cap {
        out.push((f * 1000.0).round() / 1000.0);
        f += step;
    }
    out
}

/// Local maxima that stand above the quiet part of a sweep.
pub fn power_peaks(samples: &[(f32, f32)]) -> Vec<ScanHit> {
    if samples.len() < 3 {
        return Vec::new();
    }
    let mut powers: Vec<f32> = samples.iter().map(|sample| sample.1).collect();
    powers.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let quiet = powers[powers.len() / 5];
    let median = powers[powers.len() / 2];
    let cut = (quiet * 2.8).max(median + 0.03).max(0.06);
    let mut hits = Vec::new();
    for i in 1..samples.len() - 1 {
        let (mhz, power) = samples[i];
        if power >= samples[i - 1].1 && power > samples[i + 1].1 && power > cut {
            hits.push(ScanHit {
                mhz,
                power: (power / (cut * 2.5)).clamp(0.2, 1.0),
            });
        }
    }
    hits.sort_by(|a, b| b.power.partial_cmp(&a.power).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(16);
    hits
}

/// Stand-in energy for the simulated receiver. A few stable tones, quiet elsewhere.
pub fn mock_power(mhz: f32) -> f32 {
    const TONES: [f32; 6] = [97.3, 105.7, 146.5, 433.5, 476.55, 918.0];
    let mut power: f32 = 0.02;
    for tone in TONES {
        let distance = (mhz - tone).abs();
        if distance < 0.45 {
            power = power.max(0.28 * (1.0 - distance / 0.45));
        }
    }
    power
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_name_uhf_cb_and_fm() {
        let notes = au_notes();
        assert!(notes.len() >= 10);
        assert!(notes.iter().any(|note| note.name == "UHF CB"));
        assert!(notes.iter().any(|note| note.name == "FM broadcast"));
    }

    #[test]
    fn area_scan_stays_near_the_dial() {
        let plan = scan_plan("area", 433.0);
        assert!(plan.points.len() >= 10);
        assert!(plan.points.iter().all(|mhz| (418.0..=448.0).contains(mhz)));
    }

    #[test]
    fn survey_covers_fm_and_uhf_cb() {
        let plan = scan_plan("survey", 433.0);
        assert!(plan.points.iter().any(|mhz| (87.0..109.0).contains(mhz)));
        assert!(plan.points.iter().any(|mhz| (476.0..478.0).contains(mhz)));
        assert!(plan.points.len() <= 240);
    }

    #[test]
    fn peaks_keep_the_loud_step() {
        let samples = vec![
            (100.0, 0.02),
            (101.0, 0.02),
            (102.0, 0.22),
            (103.0, 0.03),
            (104.0, 0.02),
            (105.0, 0.02),
        ];
        let hits = power_peaks(&samples);
        assert!(hits.iter().any(|hit| (hit.mhz - 102.0).abs() < 0.01));
    }

    #[test]
    fn mock_fm_tone_is_a_peak() {
        let points = mhz_points(87.5, 108.0, 0.2, 200);
        let samples: Vec<(f32, f32)> = points.iter().map(|mhz| (*mhz, mock_power(*mhz))).collect();
        let hits = power_peaks(&samples);
        assert!(hits.iter().any(|hit| (hit.mhz - 97.3).abs() < 0.25));
    }
}
