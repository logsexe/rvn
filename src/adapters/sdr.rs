//! Receive-only RTL-SDR spectrum. The worker owns the dongle. The UI thread only
//! posts a tune or a start/stop. No demodulator and no transmit path.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use num_complex::Complex;
use rtl_sdr_rs::{RtlSdr, TunerGain};
use rustfft::{Fft, FftPlanner};

use crate::core::bands::power_peaks;
use crate::core::hardware::{RadioStatus, Readiness, SPECTRUM_BINS};

const FFT_LEN: usize = 1024;
const READ_LEN: usize = 8192;
const SAMPLE_RATE: u32 = 2_048_000;

enum RadioCmd {
    Freq(f32),
    Stream(bool),
    Sweep { points: Vec<f32>, label: String },
    CancelSweep,
}

struct SweepRun {
    points: Vec<f32>,
    index: usize,
    samples: Vec<(f32, f32)>,
    label: String,
    return_mhz: f32,
}

/// How soon the worker may touch USB again. A missing dongle is logged once,
/// then left alone so the shell is not stuck reopening it.
struct ProbePace {
    next: Instant,
    misses: u32,
    told: bool,
    seen: bool,
}

impl ProbePace {
    fn new() -> Self {
        Self {
            next: Instant::now(),
            misses: 0,
            told: false,
            seen: false,
        }
    }

    fn due(&self) -> bool {
        Instant::now() >= self.next
    }

    fn ask(&mut self) {
        self.told = false;
        self.next = Instant::now();
    }

    fn found(&mut self) -> bool {
        let came_back = self.told;
        self.misses = 0;
        self.told = false;
        self.seen = true;
        self.next = Instant::now() + Duration::from_secs(2);
        came_back
    }

    fn lost(&mut self, err: &str) {
        self.announce(err, false);
    }

    fn quiet_absent(&mut self) {
        if self.seen {
            self.announce("", true);
        } else {
            self.arm(true);
        }
    }

    fn announce(&mut self, err: &str, clean: bool) {
        if !self.told {
            tracing::warn!("{}", absence_line(self.seen, err));
            self.told = true;
        }
        self.seen = false;
        self.arm(clean);
    }

    fn arm(&mut self, clean: bool) {
        self.misses = self.misses.saturating_add(1);
        self.next = Instant::now() + reconnect_pause(self.misses, clean);
    }
}

fn reconnect_pause(misses: u32, listed_empty: bool) -> Duration {
    if listed_empty {
        if misses <= 1 {
            Duration::from_secs(3)
        } else {
            Duration::from_secs(8)
        }
    } else if misses <= 1 {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(15)
    }
}

fn absence_line(seen: bool, err: &str) -> String {
    let plain = err.is_empty() || err == "no RTL-SDR";
    if seen {
        if plain {
            "SDR receiver removed".into()
        } else {
            format!("SDR receiver removed: {err}")
        }
    } else if plain {
        "SDR receiver unavailable".into()
    } else {
        format!("SDR receiver unavailable: {err}")
    }
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

    pub fn start_sweep(&self, points: Vec<f32>, label: String) {
        self.send(RadioCmd::Sweep { points, label });
    }

    pub fn cancel_sweep(&self) {
        self.send(RadioCmd::CancelSweep);
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
    let mut sweep: Option<SweepRun> = None;
    let mut pace = ProbePace::new();
    let mut last_fft = Instant::now() - Duration::from_secs(1);
    let mut buf = vec![0u8; READ_LEN];
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_LEN);

    loop {
        let mut tune: Option<f32> = None;
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                RadioCmd::Freq(mhz) => {
                    tune = Some(mhz.clamp(24.0, 1700.0));
                    if sweep.take().is_some() {
                        let mut state = status.lock().unwrap();
                        state.scanning = false;
                    }
                }
                RadioCmd::Stream(on) => {
                    if !on {
                        close_device(&mut device);
                        let mut state = status.lock().unwrap();
                        state.spectrum = vec![0.0; SPECTRUM_BINS];
                        if state.readiness == Readiness::Active {
                            state.readiness = Readiness::Ready;
                        }
                    } else {
                        pace.ask();
                    }
                    streaming = on;
                }
                RadioCmd::Sweep { points, label } => {
                    pace.ask();
                    let mut state = status.lock().unwrap();
                    state.scanning = true;
                    state.scan_progress = 0.0;
                    state.scan_hits.clear();
                    state.scan_label = label.clone();
                    sweep = Some(SweepRun {
                        points,
                        index: 0,
                        samples: Vec::new(),
                        label,
                        return_mhz: freq_mhz,
                    });
                }
                RadioCmd::CancelSweep => {
                    if let Some(run) = sweep.take() {
                        tune = Some(run.return_mhz);
                        let mut state = status.lock().unwrap();
                        state.scanning = false;
                        state.scan_progress = if run.points.is_empty() {
                            0.0
                        } else {
                            run.index as f32 / run.points.len() as f32
                        };
                    }
                }
            }
        }
        if let Some(mhz) = tune {
            freq_mhz = mhz;
            if let Some(sdr) = device.as_mut() {
                if let Err(err) = sdr.set_center_freq(mhz_to_hz(freq_mhz)) {
                    tracing::warn!("SDR tune: {err}");
                }
            }
            status.lock().unwrap().center_freq_mhz = freq_mhz;
        }

        let step = sweep.as_mut().map(|run| sweep_step(&status, &mut device, run, &mut buf));
        if let Some(step) = step {
            match step {
                SweepStep::Continue => continue,
                SweepStep::Finished(back) => {
                    sweep = None;
                    freq_mhz = back;
                    if let Some(sdr) = device.as_mut() {
                        let _ = sdr.set_center_freq(mhz_to_hz(freq_mhz));
                    }
                    status.lock().unwrap().center_freq_mhz = freq_mhz;
                    if !streaming {
                        close_device(&mut device);
                    }
                    continue;
                }
                SweepStep::Failed(err) => {
                    pace.lost(&err);
                    sweep = None;
                    let mut state = status.lock().unwrap();
                    state.scanning = false;
                    state.scan_label = "No receiver".into();
                    mark_radio_down(&mut state, err == "no RTL-SDR");
                    drop(state);
                    close_device(&mut device);
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
            }
        }

        if streaming {
            if device.is_none() {
                if !pace.due() {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
                match open_sdr(freq_mhz) {
                    Ok((sdr, name)) => {
                        device = Some(sdr);
                        if pace.found() {
                            tracing::info!("SDR receiver back");
                        }
                        let mut state = status.lock().unwrap();
                        state.readiness = Readiness::Active;
                        state.device = name;
                        state.sample_rate = SAMPLE_RATE;
                        state.center_freq_mhz = freq_mhz;
                        state.simulated = false;
                    }
                    Err(err) => {
                        pace.lost(&err);
                        let mut state = status.lock().unwrap();
                        mark_radio_down(&mut state, err == "no RTL-SDR");
                        drop(state);
                        std::thread::sleep(Duration::from_millis(200));
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
                    pace.seen = true;
                }
                Ok(_) => std::thread::sleep(Duration::from_millis(20)),
                Err(err) => {
                    pace.lost(&err.to_string());
                    close_device(&mut device);
                    mark_radio_down(&mut status.lock().unwrap(), false);
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        } else if pace.due() {
            match scan_devices(&status, freq_mhz) {
                Listed::Present => {
                    if pace.found() {
                        tracing::info!("SDR receiver back");
                    }
                }
                Listed::Absent => pace.quiet_absent(),
                Listed::Error(err) => pace.lost(&err),
            }
            std::thread::sleep(Duration::from_millis(200));
        } else {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

fn mark_radio_down(state: &mut RadioStatus, missing: bool) {
    state.readiness = if missing {
        Readiness::NotPresent
    } else {
        Readiness::Degraded
    };
    state.spectrum = vec![0.0; SPECTRUM_BINS];
    if missing {
        state.device = "—".into();
        state.sample_rate = 0;
    }
}

enum Listed {
    Present,
    Absent,
    Error(String),
}

enum SweepStep {
    Continue,
    Finished(f32),
    Failed(String),
}

fn sweep_step(
    status: &Mutex<RadioStatus>,
    device: &mut Option<RtlSdr>,
    run: &mut SweepRun,
    buf: &mut [u8],
) -> SweepStep {
    if run.points.is_empty() || run.index >= run.points.len() {
        finish_sweep(status, run);
        return SweepStep::Finished(run.return_mhz);
    }
    let mhz = run.points[run.index];
    if device.is_none() {
        match open_sdr(mhz) {
            Ok((sdr, name)) => {
                *device = Some(sdr);
                let mut state = status.lock().unwrap();
                state.readiness = Readiness::Active;
                state.device = name;
                state.sample_rate = SAMPLE_RATE;
                state.simulated = false;
            }
            Err(err) => return SweepStep::Failed(err),
        }
    }
    let Some(sdr) = device.as_mut() else {
        return SweepStep::Failed("no RTL-SDR".into());
    };
    if let Err(err) = sdr.set_center_freq(mhz_to_hz(mhz)) {
        return SweepStep::Failed(err.to_string());
    }
    let _ = sdr.reset_buffer();
    std::thread::sleep(Duration::from_millis(18));
    let _ = sdr.read_sync(buf);
    let n = match sdr.read_sync(buf) {
        Ok(n) => n,
        Err(err) => return SweepStep::Failed(err.to_string()),
    };
    run.samples.push((mhz, iq_power(&buf[..n])));
    run.index += 1;
    let hits = power_peaks(&run.samples);
    let done = run.index >= run.points.len();
    {
        let mut state = status.lock().unwrap();
        state.scanning = !done;
        state.scan_progress = run.index as f32 / run.points.len() as f32;
        state.scan_hits = hits;
        state.scan_label = if done {
            run.label.clone()
        } else {
            format!("{} · {:.3} MHz", run.label, mhz)
        };
    }
    if done {
        SweepStep::Finished(run.return_mhz)
    } else {
        SweepStep::Continue
    }
}

fn finish_sweep(status: &Mutex<RadioStatus>, run: &SweepRun) {
    let mut state = status.lock().unwrap();
    state.scanning = false;
    state.scan_progress = 1.0;
    state.scan_hits = power_peaks(&run.samples);
    state.scan_label = run.label.clone();
}

fn iq_power(iq: &[u8]) -> f32 {
    if iq.is_empty() {
        return 0.0;
    }
    let n = iq.len().min(4096);
    let mut acc = 0.0_f32;
    for &sample in &iq[..n] {
        let v = (sample as f32 - 127.5) / 127.5;
        acc += v * v;
    }
    (acc / n as f32).sqrt()
}

fn scan_devices(status: &Mutex<RadioStatus>, freq_mhz: f32) -> Listed {
    match list_devices() {
        Ok(devices) => {
            let mut state = status.lock().unwrap();
            if let Some(device) = devices.first() {
                state.readiness = Readiness::Ready;
                state.device = sdr_name(&device.product, device.vendor_id, device.product_id);
                state.sample_rate = SAMPLE_RATE;
                state.center_freq_mhz = freq_mhz;
                state.simulated = false;
                Listed::Present
            } else {
                state.readiness = Readiness::NotPresent;
                state.device = "—".into();
                state.sample_rate = 0;
                state.spectrum = vec![0.0; SPECTRUM_BINS];
                Listed::Absent
            }
        }
        Err(err) => {
            let mut state = status.lock().unwrap();
            state.readiness = Readiness::Degraded;
            if state.device == "—" {
                state.device = "RTL-SDR".into();
            }
            Listed::Error(err)
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
    fn a_missing_receiver_waits_longer_each_miss() {
        assert_eq!(reconnect_pause(1, true), Duration::from_secs(3));
        assert_eq!(reconnect_pause(2, true), Duration::from_secs(8));
        assert_eq!(reconnect_pause(1, false), Duration::from_secs(5));
        assert_eq!(reconnect_pause(4, false), Duration::from_secs(15));
    }

    #[test]
    fn removal_is_one_line() {
        assert_eq!(absence_line(true, "no RTL-SDR"), "SDR receiver removed");
        assert_eq!(
            absence_line(true, "device disconnected"),
            "SDR receiver removed: device disconnected"
        );
        assert_eq!(absence_line(false, "no RTL-SDR"), "SDR receiver unavailable");
    }

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
