//! Receive-only RTL-SDR spectrum. The worker owns the dongle. The UI thread only
//! posts a tune or a start/stop. No demodulator and no transmit path.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use num_complex::Complex;
use rtl_sdr_rs::{RtlSdr, TunerGain};
use rustfft::{Fft, FftPlanner};

use crate::core::hardware::{RadioStatus, Readiness, SPECTRUM_BINS};

const FFT_LEN: usize = 1024;
const READ_LEN: usize = 8192;
const SAMPLE_RATE: u32 = 2_048_000;

enum RadioCmd {
    Freq(f32),
    Stream(bool),
}

pub struct RadioAdapter {
    status: Arc<Mutex<RadioStatus>>,
    tx: Mutex<Option<Sender<RadioCmd>>>,
}

impl RadioAdapter {
    pub fn start() -> Self {
        let status = Arc::new(Mutex::new(RadioStatus {
            center_freq_mhz: 433.0,
            ..RadioStatus::default()
        }));
        let (tx, rx) = mpsc::channel();
        let worker = status.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("rvn-sdr".into())
            .spawn(move || radio_loop(worker, rx))
        {
            tracing::warn!("SDR thread: {err}");
        }
        Self {
            status,
            tx: Mutex::new(Some(tx)),
        }
    }

    pub fn snapshot(&self) -> RadioStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn set_freq(&self, mhz: f32) {
        self.send(RadioCmd::Freq(mhz));
    }

    pub fn set_streaming(&self, on: bool) {
        self.send(RadioCmd::Stream(on));
    }

    fn send(&self, cmd: RadioCmd) {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(cmd);
        }
    }
}

fn radio_loop(status: Arc<Mutex<RadioStatus>>, rx: Receiver<RadioCmd>) {
    let mut streaming = false;
    let mut freq_mhz = 433.0_f32;
    let mut device: Option<RtlSdr> = None;
    let mut last_scan = Instant::now() - Duration::from_secs(10);
    let mut last_fft = Instant::now() - Duration::from_secs(1);
    let mut buf = vec![0u8; READ_LEN];
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_LEN);

    loop {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                RadioCmd::Freq(mhz) => {
                    freq_mhz = mhz.max(0.1);
                    if let Some(sdr) = device.as_mut() {
                        let hz = mhz_to_hz(freq_mhz);
                        if let Err(err) = sdr.set_center_freq(hz) {
                            tracing::warn!("SDR tune: {err}");
                        }
                    }
                    status.lock().unwrap().center_freq_mhz = freq_mhz;
                }
                RadioCmd::Stream(on) => {
                    if !on {
                        close_device(&mut device);
                        let mut state = status.lock().unwrap();
                        state.spectrum = vec![0.0; SPECTRUM_BINS];
                        if state.readiness == Readiness::Active {
                            state.readiness = Readiness::Ready;
                        }
                    }
                    streaming = on;
                }
            }
        }

        if streaming {
            if device.is_none() {
                match open_sdr(freq_mhz) {
                    Ok((sdr, name)) => {
                        device = Some(sdr);
                        let mut state = status.lock().unwrap();
                        state.readiness = Readiness::Active;
                        state.device = name;
                        state.sample_rate = SAMPLE_RATE;
                        state.center_freq_mhz = freq_mhz;
                        state.simulated = false;
                    }
                    Err(err) => {
                        tracing::warn!("SDR open: {err}");
                        let mut state = status.lock().unwrap();
                        state.readiness = if err == "no RTL-SDR" {
                            Readiness::NotPresent
                        } else {
                            Readiness::Degraded
                        };
                        state.spectrum = vec![0.0; SPECTRUM_BINS];
                        std::thread::sleep(Duration::from_millis(700));
                        continue;
                    }
                }
            }
            let Some(sdr) = device.as_mut() else {
                continue;
            };
            match sdr.read_sync(&mut buf) {
                Ok(n) if n >= FFT_LEN * 2 && last_fft.elapsed() >= Duration::from_millis(100) => {
                    let bins = spectrum_from_iq_with(fft.as_ref(), &buf[..n]);
                    let mut state = status.lock().unwrap();
                    state.spectrum = bins;
                    state.readiness = Readiness::Active;
                    state.center_freq_mhz = freq_mhz;
                    state.sample_rate = SAMPLE_RATE;
                    state.simulated = false;
                    last_fft = Instant::now();
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!("SDR read: {err}");
                    close_device(&mut device);
                    let mut state = status.lock().unwrap();
                    state.readiness = Readiness::Degraded;
                    state.spectrum = vec![0.0; SPECTRUM_BINS];
                    std::thread::sleep(Duration::from_millis(400));
                }
            }
        } else {
            if last_scan.elapsed() >= Duration::from_secs(2) {
                last_scan = Instant::now();
                scan_devices(&status, freq_mhz);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

fn scan_devices(status: &Mutex<RadioStatus>, freq_mhz: f32) {
    match list_devices() {
        Ok(devices) => {
            let mut state = status.lock().unwrap();
            if let Some(device) = devices.first() {
                state.readiness = Readiness::Ready;
                state.device = sdr_name(&device.product, device.vendor_id, device.product_id);
                state.sample_rate = SAMPLE_RATE;
                state.center_freq_mhz = freq_mhz;
                state.simulated = false;
            } else {
                state.readiness = Readiness::NotPresent;
                state.device = "—".into();
                state.sample_rate = 0;
                state.spectrum = vec![0.0; SPECTRUM_BINS];
            }
        }
        Err(err) => {
            tracing::warn!("SDR scan: {err}");
            let mut state = status.lock().unwrap();
            state.readiness = Readiness::Degraded;
            if state.device == "—" {
                state.device = "RTL-SDR".into();
            }
        }
    }
}

fn open_sdr(freq_mhz: f32) -> Result<(RtlSdr, String), String> {
    let devices = list_devices()?;
    let Some(device) = devices.first() else {
        return Err("no RTL-SDR".into());
    };
    let name = sdr_name(&device.product, device.vendor_id, device.product_id);
    let mut sdr = RtlSdr::open_first_available().map_err(|err| err.to_string())?;
    sdr.set_sample_rate(SAMPLE_RATE).map_err(|err| err.to_string())?;
    sdr.set_center_freq(mhz_to_hz(freq_mhz)).map_err(|err| err.to_string())?;
    let _ = sdr.set_tuner_gain(TunerGain::Auto);
    sdr.reset_buffer().map_err(|err| err.to_string())?;
    Ok((sdr, name))
}

fn list_devices() -> Result<Vec<rtl_sdr_rs::DeviceDescriptor>, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(RtlSdr::list_devices)) {
        Ok(Ok(devices)) => Ok(devices),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => Err("SDR enumeration failed".into()),
    }
}

fn close_device(device: &mut Option<RtlSdr>) {
    if let Some(mut sdr) = device.take() {
        let _ = sdr.close();
    }
}

fn sdr_name(product: &str, vid: u16, pid: u16) -> String {
    let product = product.trim();
    if product.is_empty() {
        format!("RTL-SDR {vid:04x}:{pid:04x}")
    } else {
        product.to_string()
    }
}

fn mhz_to_hz(mhz: f32) -> u32 {
    (mhz * 1_000_000.0).round().clamp(0.0, u32::MAX as f32) as u32
}

pub fn spectrum_from_iq(iq: &[u8]) -> Vec<f32> {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_LEN);
    spectrum_from_iq_with(fft.as_ref(), iq)
}

fn spectrum_from_iq_with(fft: &dyn Fft<f32>, iq: &[u8]) -> Vec<f32> {
    if iq.len() < FFT_LEN * 2 {
        return vec![0.0; SPECTRUM_BINS];
    }
    let mut samples = Vec::with_capacity(FFT_LEN);
    for i in 0..FFT_LEN {
        let i_s = (iq[i * 2] as f32 - 127.5) / 127.5;
        let q_s = (iq[i * 2 + 1] as f32 - 127.5) / 127.5;
        let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / FFT_LEN as f32).cos());
        samples.push(Complex::new(i_s * window, q_s * window));
    }
    fft.process(&mut samples);

    let mut shifted = vec![0.0_f32; FFT_LEN];
    for i in 0..FFT_LEN {
        let src = (i + FFT_LEN / 2) % FFT_LEN;
        shifted[i] = samples[src].norm();
    }
    let width = FFT_LEN / SPECTRUM_BINS;
    let mut bins = vec![0.0_f32; SPECTRUM_BINS];
    for (bin, chunk) in bins.iter_mut().zip(shifted.chunks(width)) {
        let avg = chunk.iter().sum::<f32>() / width as f32;
        let db = (avg + 1e-6).log10();
        *bin = ((db + 3.0) / 3.3).clamp(0.0, 1.0);
    }
    bins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_buffer_is_silent() {
        let bins = spectrum_from_iq(&[127, 127, 128, 128]);
        assert_eq!(bins.len(), SPECTRUM_BINS);
        assert!(bins.iter().all(|bin| *bin == 0.0));
    }

    #[test]
    fn a_tone_peaks_in_one_region() {
        let mut iq = vec![127u8; FFT_LEN * 2];
        let tone = 200.0_f32;
        for i in 0..FFT_LEN {
            let phase = 2.0 * std::f32::consts::PI * tone * i as f32 / FFT_LEN as f32;
            iq[i * 2] = (phase.cos() * 100.0 + 127.5) as u8;
            iq[i * 2 + 1] = (phase.sin() * 100.0 + 127.5) as u8;
        }
        let bins = spectrum_from_iq(&iq);
        assert_eq!(bins.len(), SPECTRUM_BINS);
        let (peak_at, peak) = bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert!(*peak > 0.7, "{peak}");
        assert!((32..=35).contains(&peak_at), "peak at {peak_at}");
    }
}
