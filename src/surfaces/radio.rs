//! Spectrum plot for the RADIO surface.
//! Bins are a deterministic stand-in until an SDR adapter is commissioned.
//! The UI marks the plot SIM so it is not read as live RF energy.

use crate::core::hardware::SPECTRUM_BINS;

const BINS: usize = SPECTRUM_BINS;

pub fn spectrum_bins(elapsed_s: f32, streaming: bool, present: bool, center_mhz: f32) -> Vec<f32> {
    if !present {
        return vec![0.0; BINS];
    }

    let shift = ((center_mhz - 433.0) / 40.0).clamp(-0.3, 0.3);
    (0..BINS)
        .map(|i| {
            let x = i as f32 / (BINS as f32 - 1.0);
            let noise = ((elapsed_s * 2.3 + i as f32 * 0.73).sin() * 0.5 + 0.5) * 0.07;
            if !streaming {
                return noise;
            }
            let peak = |at: f32, width: f32, amp: f32| (-((x - at).powi(2)) * width).exp() * amp;
            let wander = (elapsed_s * 0.35).sin() * 0.06;
            (noise
                + peak(0.33 + shift, 90.0, 0.9)
                + peak(0.58 + shift * 0.4, 160.0, 0.5)
                + peak(0.22 + wander, 70.0, 0.35))
            .clamp(0.02, 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_device_is_silent() {
        let bins = spectrum_bins(1.0, true, false, 433.0);
        assert_eq!(bins.len(), BINS);
        assert!(bins.iter().all(|bin| *bin == 0.0));
    }

    #[test]
    fn streaming_raises_a_peak() {
        let bins = spectrum_bins(2.0, true, true, 433.0);
        let peak = bins.iter().copied().fold(0.0_f32, f32::max);
        assert!(peak > 0.6, "{peak}");
    }

    #[test]
    fn tuning_moves_energy() {
        let low = spectrum_bins(1.0, true, true, 420.0);
        let high = spectrum_bins(1.0, true, true, 446.0);
        let centroid = |bins: &[f32]| {
            let (sum, energy) = bins.iter().enumerate().fold((0.0_f32, 0.0_f32), |(s, e), (i, v)| {
                (s + i as f32 * v, e + v)
            });
            sum / energy
        };
        assert!(centroid(&high) > centroid(&low));
    }
}
