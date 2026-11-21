use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nnnoiseless::DenoiseState;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use windows::core::{implement, IUnknown, Interface, Ref, GUID, HRESULT, HSTRING};
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDCLNT_S_BUFFER_EMPTY,
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, BLOB, CLSCTX_ALL};
use windows::Win32::System::Variant::VT_BLOB;

use super::devices::ComGuard;

const BUFFER_DURATION: i64 = 1_000_000;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const RETRY_DELAY: Duration = Duration::from_secs(2);

pub const MAX_DELAY_MS: u32 = 5000;
pub const PAN_MIN: i8 = -100;
pub const PAN_MAX: i8 = 100;
const MAX_GAIN: f32 = 16.0;
const MAX_DEVICE_ID_LEN: usize = 2048;

fn safe_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0.0, MAX_GAIN)
    } else {
        0.0
    }
}

fn device_id<'a>(id: &'a str, prefix: &str) -> windows::core::Result<&'a str> {
    let raw = id
        .strip_prefix(prefix)
        .filter(|raw| {
            !raw.is_empty()
                && raw.chars().count() <= MAX_DEVICE_ID_LEN
                && !raw.chars().any(char::is_control)
        })
        .ok_or_else(|| windows::core::Error::new(E_FAIL, "identifiant de périphérique invalide"))?;
    Ok(raw)
}

const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
const IEEE_FLOAT_SUBFORMAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

#[derive(Default)]
pub struct AudioEngine {
    routes: Vec<ActiveRoute>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSpec {
    pub id: String,
    pub gain: f32,
}

#[derive(Debug, PartialEq)]
pub struct RouteSpec {
    pub input: String,
    pub input_gain: f32,
    pub delay_ms: u32,
    pub pan: i8,
    pub noise_suppression: bool,
    pub outputs: Vec<OutputSpec>,
}

struct ActiveOutput {
    id: String,
    gain: Arc<AtomicU32>,
    level: Arc<AtomicU32>,
}

struct ActiveRoute {
    input: String,
    input_gain: Arc<AtomicU32>,
    delay_ms: Arc<AtomicU32>,
    pan: Arc<AtomicI8>,
    noise_suppression: Arc<AtomicBool>,
    outputs: Vec<ActiveOutput>,
    level: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    retry_at: Instant,
}

impl ActiveRoute {
    fn start(spec: RouteSpec) -> Option<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));
        let input_gain = Arc::new(AtomicU32::new(safe_gain(spec.input_gain).to_bits()));
        let delay_ms = Arc::new(AtomicU32::new(spec.delay_ms.min(MAX_DELAY_MS)));
        let pan = Arc::new(AtomicI8::new(spec.pan.clamp(PAN_MIN, PAN_MAX)));
        let noise_suppression = Arc::new(AtomicBool::new(spec.noise_suppression));
        let outputs: Vec<ActiveOutput> = spec
            .outputs
            .into_iter()
            .map(|output| ActiveOutput {
                id: output.id,
                gain: Arc::new(AtomicU32::new(safe_gain(output.gain).to_bits())),
                level: Arc::new(AtomicU32::new(0)),
            })
            .collect();
        let handle = spawn_route(
            &spec.input,
            &outputs,
            &input_gain,
            &delay_ms,
            &pan,
            &noise_suppression,
            &stop,
            &level,
        )?;
        Some(Self {
            input: spec.input,
            input_gain,
            delay_ms,
            pan,
            noise_suppression,
            outputs,
            level,
            stop,
            handle: Some(handle),
            retry_at: Instant::now() + RETRY_DELAY,
        })
    }

    fn set_params(&self, spec: &RouteSpec) {
        self.input_gain
            .store(safe_gain(spec.input_gain).to_bits(), Ordering::Relaxed);
        self.delay_ms
            .store(spec.delay_ms.min(MAX_DELAY_MS), Ordering::Relaxed);
        self.pan
            .store(spec.pan.clamp(PAN_MIN, PAN_MAX), Ordering::Relaxed);
        self.noise_suppression
            .store(spec.noise_suppression, Ordering::Relaxed);
        for output in &self.outputs {
            if let Some(wanted) = spec.outputs.iter().find(|wanted| wanted.id == output.id) {
                output
                    .gain
                    .store(safe_gain(wanted.gain).to_bits(), Ordering::Relaxed);
            }
        }
    }

    fn restart_if_dead(&mut self) {
        let now = Instant::now();
        if !should_restart(&self.handle, now, self.retry_at) {
            return;
        }
        self.handle = None;
        self.stop = Arc::new(AtomicBool::new(false));
        let spawned = spawn_route(
            &self.input,
            &self.outputs,
            &self.input_gain,
            &self.delay_ms,
            &self.pan,
            &self.noise_suppression,
            &self.stop,
            &self.level,
        );
        self.retry_at = now + RETRY_DELAY;
        crate::logging::log(
            "route",
            &format!(
                "relance {} ({})",
                self.input,
                if spawned.is_some() {
                    "ok"
                } else {
                    "échec thread"
                }
            ),
        );
        self.handle = spawned;
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_route(
    input: &str,
    outputs: &[ActiveOutput],
    input_gain: &Arc<AtomicU32>,
    delay_ms: &Arc<AtomicU32>,
    pan: &Arc<AtomicI8>,
    noise_suppression: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    level: &Arc<AtomicU32>,
) -> Option<std::thread::JoinHandle<()>> {
    let flag = Arc::clone(stop);
    let level = Arc::clone(level);
    let thread_input_gain = Arc::clone(input_gain);
    let thread_delay_ms = Arc::clone(delay_ms);
    let thread_pan = Arc::clone(pan);
    let thread_noise_suppression = Arc::clone(noise_suppression);
    let thread_input = input.to_string();
    let thread_outputs: Vec<(String, Arc<AtomicU32>, Arc<AtomicU32>)> = outputs
        .iter()
        .map(|output| {
            (
                output.id.clone(),
                Arc::clone(&output.gain),
                Arc::clone(&output.level),
            )
        })
        .collect();
    std::thread::Builder::new()
        .name("audiok-audio".to_string())
        .spawn(move || {
            if let Err(err) = run_route(
                &thread_input,
                &thread_outputs,
                &thread_input_gain,
                &thread_delay_ms,
                &thread_pan,
                &thread_noise_suppression,
                &flag,
                &level,
            ) {
                crate::logging::log(
                    "route",
                    &format!("route audio interrompue ({thread_input}): {err}"),
                );
            }
        })
        .ok()
}

fn should_restart(
    handle: &Option<std::thread::JoinHandle<()>>,
    now: Instant,
    retry_at: Instant,
) -> bool {
    handle.as_ref().is_none_or(|handle| handle.is_finished()) && now >= retry_at
}

fn scaled_peak<'a>(samples: impl Iterator<Item = &'a f32>, gain: f32) -> f32 {
    samples.fold(0.0, |peak, sample| peak.max((sample * gain).abs()))
}

impl AudioEngine {
    pub fn reconcile(&mut self, desired: Vec<RouteSpec>) {
        let mut kept = Vec::new();
        for mut route in std::mem::take(&mut self.routes) {
            let want = desired.iter().find(|spec| spec.input == route.input);
            match want {
                Some(spec) if same_outputs(&route.outputs, &spec.outputs) => {
                    route.set_params(spec);
                    route.restart_if_dead();
                    kept.push(route);
                }
                Some(_) => {
                    crate::logging::log(
                        "route",
                        &format!("arrêt {} (sorties modifiées)", route.input),
                    );
                    route.stop();
                }
                None => {
                    crate::logging::log("route", &format!("arrêt {}", route.input));
                    route.stop();
                }
            }
        }
        self.routes = kept;

        for spec in desired {
            if self.routes.iter().any(|route| route.input == spec.input) {
                continue;
            }
            let description = format!(
                "{} (entrée ×{:.2}) -> {}",
                spec.input,
                spec.input_gain,
                describe_outputs(&spec.outputs)
            );
            match ActiveRoute::start(spec) {
                Some(route) => {
                    crate::logging::log("route", &format!("démarrage {description}"));
                    self.routes.push(route);
                }
                None => crate::logging::log("route", &format!("échec thread {description}")),
            }
        }
    }

    fn stop_all(&mut self) {
        for route in std::mem::take(&mut self.routes) {
            route.stop();
        }
    }

    pub fn level(&self, input: &str) -> f32 {
        self.routes
            .iter()
            .find(|route| route.input == input)
            .map(|route| f32::from_bits(route.level.load(Ordering::Relaxed)))
            .unwrap_or(0.0)
    }

    pub fn output_level(&self, output: &str) -> f32 {
        self.routes
            .iter()
            .flat_map(|route| route.outputs.iter())
            .filter(|target| target.id == output)
            .map(|target| f32::from_bits(target.level.load(Ordering::Relaxed)))
            .sum()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop_all();
    }
}

fn same_outputs(active: &[ActiveOutput], wanted: &[OutputSpec]) -> bool {
    active.len() == wanted.len()
        && wanted
            .iter()
            .all(|spec| active.iter().any(|target| target.id == spec.id))
}

pub(crate) fn describe_outputs(outputs: &[OutputSpec]) -> String {
    if outputs.is_empty() {
        return "aucune sortie".to_string();
    }
    outputs
        .iter()
        .map(|output| format!("{}@{:.2}", output.id, output.gain))
        .collect::<Vec<_>>()
        .join(", ")
}

struct RenderTarget {
    client: IAudioClient,
    render: IAudioRenderClient,
    frames: u32,
    ring: VecDeque<f32>,
    gain: Arc<AtomicU32>,
    level: Arc<AtomicU32>,
}

struct CoTaskMem(*const core::ffi::c_void);

impl Drop for CoTaskMem {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0)) };
    }
}

struct FormatInfo {
    channels: usize,
    sample_rate: u32,
}

const RENDER_CHANNELS: usize = 2;

fn pan_gains(pan: i8) -> (f32, f32) {
    if pan <= PAN_MIN {
        return (1.0, 0.0);
    }
    if pan >= PAN_MAX {
        return (0.0, 1.0);
    }
    let value = pan.clamp(PAN_MIN, PAN_MAX) as f32 / 100.0;
    let angle = (value + 1.0) * std::f32::consts::FRAC_PI_4;
    (angle.cos(), angle.sin())
}

fn stereo_samples(samples: &[f32], channels: usize, gain: f32, pan: i8) -> Vec<f32> {
    let (left, right) = pan_gains(pan);
    let mut output = Vec::with_capacity((samples.len() / channels) * RENDER_CHANNELS);
    for frame in samples.chunks_exact(channels) {
        if channels == 1 {
            let value = frame[0] * gain;
            output.push(value * left);
            output.push(value * right);
        } else {
            output.push(frame[0] * gain * left);
            output.push(frame[1] * gain * right);
        }
    }
    output
}

fn delay_line_samples(delay_ms: u32, sample_rate: u32) -> usize {
    (delay_ms.min(MAX_DELAY_MS) as usize * sample_rate as usize / 1000) * RENDER_CHANNELS
}

fn resize_delay_line(line: &mut VecDeque<f32>, wanted: usize) {
    while line.len() < wanted {
        line.push_back(0.0);
    }
    while line.len() > wanted {
        line.pop_front();
    }
}

const NR_SAMPLE_RATE: u32 = 48_000;
const NR_FRAME: usize = DenoiseState::FRAME_SIZE;
const NR_SCALE: f32 = 32768.0;

struct ChannelResampler {
    resampler: SincFixedIn<f32>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    pending: Vec<VecDeque<f32>>,
}

impl ChannelResampler {
    fn new(from: u32, to: u32, channels: usize) -> Result<Self, String> {
        let ratio = to as f64 / from as f64;
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        };
        let chunk = 1024usize.min((from as usize / 10).max(64));
        let resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, chunk, channels)
            .map_err(|e| e.to_string())?;
        let input = resampler.input_buffer_allocate(true);
        let output = resampler.output_buffer_allocate(true);
        let mut pending = Vec::with_capacity(channels);
        for _ in 0..channels {
            pending.push(VecDeque::new());
        }
        Ok(Self {
            resampler,
            input,
            output,
            pending,
        })
    }

    fn process(&mut self, samples: &[f32], channels: usize) -> Result<Vec<f32>, String> {
        let channels = channels.max(1);
        if self.pending.len() != channels {
            return Err(format!(
                "channel count mismatch: resampler={} got={channels}",
                self.pending.len()
            ));
        }
        if samples.is_empty() {
            return Ok(Vec::new());
        }
        let frames = samples.len() / channels;
        for i in 0..frames {
            for ch in 0..channels {
                self.pending[ch].push_back(samples[i * channels + ch]);
            }
        }

        let mut out = Vec::new();
        loop {
            let needed = self.resampler.input_frames_next();
            if self.pending[0].len() < needed {
                break;
            }
            for ch in 0..channels {
                let buf = &mut self.input[ch];
                buf.clear();
                buf.extend(self.pending[ch].drain(..needed));
            }
            let (_, produced) = self
                .resampler
                .process_into_buffer(&self.input, &mut self.output, None)
                .map_err(|e| e.to_string())?;
            for i in 0..produced {
                for ch in 0..channels {
                    out.push(self.output[ch][i]);
                }
            }
        }
        Ok(out)
    }
}

struct NoiseSuppressor {
    states: Vec<Box<DenoiseState<'static>>>,
    in_resampler: Option<ChannelResampler>,
    out_resampler: Option<ChannelResampler>,
    pending_48k: Vec<VecDeque<f32>>,
    pending_out: VecDeque<f32>,
    channels: usize,
    sample_rate: u32,
    first_frame: Vec<bool>,
}

impl NoiseSuppressor {
    fn new(sample_rate: u32, channels: usize) -> Result<Self, String> {
        let channels = channels.max(1);
        let needs_resample = sample_rate != NR_SAMPLE_RATE;
        let (in_resampler, out_resampler) = if needs_resample {
            (
                Some(ChannelResampler::new(
                    sample_rate,
                    NR_SAMPLE_RATE,
                    channels,
                )?),
                Some(ChannelResampler::new(
                    NR_SAMPLE_RATE,
                    sample_rate,
                    channels,
                )?),
            )
        } else {
            (None, None)
        };
        let mut states = Vec::with_capacity(channels);
        let mut pending_48k = Vec::with_capacity(channels);
        let mut first_frame = Vec::with_capacity(channels);
        for _ in 0..channels {
            states.push(DenoiseState::new());
            pending_48k.push(VecDeque::new());
            first_frame.push(true);
        }
        Ok(Self {
            states,
            in_resampler,
            out_resampler,
            pending_48k,
            pending_out: VecDeque::new(),
            channels,
            sample_rate,
            first_frame,
        })
    }

    fn matches(&self, sample_rate: u32, channels: usize) -> bool {
        self.sample_rate == sample_rate && self.channels == channels.max(1)
    }

    fn process(&mut self, samples: &[f32], channels: usize) -> Result<Vec<f32>, String> {
        let channels = channels.max(1);
        if channels != self.channels {
            return Err(format!(
                "channel count mismatch: nr={} got={channels}",
                self.channels
            ));
        }
        let nr_input = if let Some(resampler) = self.in_resampler.as_mut() {
            resampler.process(samples, channels)?
        } else {
            samples.to_vec()
        };

        let frames = nr_input.len() / channels;
        for i in 0..frames {
            for ch in 0..channels {
                self.pending_48k[ch].push_back(nr_input[i * channels + ch] * NR_SCALE);
            }
        }

        let mut denoised_48k = Vec::new();
        while self.pending_48k[0].len() >= NR_FRAME {
            let mut produced = vec![0.0f32; NR_FRAME * channels];
            for ch in 0..channels {
                let mut input = [0.0f32; NR_FRAME];
                for (dst, src) in input.iter_mut().zip(self.pending_48k[ch].drain(..NR_FRAME)) {
                    *dst = src.clamp(-NR_SCALE, NR_SCALE);
                }
                let mut output = [0.0f32; NR_FRAME];
                self.states[ch].process_frame(&mut output, &input);
                if self.first_frame[ch] {
                    // keep first frame but attenuated fade-in is acceptable; avoid dropping timing
                    self.first_frame[ch] = false;
                }
                for i in 0..NR_FRAME {
                    produced[i * channels + ch] = (output[i] / NR_SCALE).clamp(-1.0, 1.0);
                }
            }
            denoised_48k.extend_from_slice(&produced);
        }

        if denoised_48k.is_empty() && self.pending_out.is_empty() {
            // not enough data yet; return silence of same length to preserve timing
            return Ok(vec![0.0; samples.len()]);
        }

        let out = if let Some(resampler) = self.out_resampler.as_mut() {
            if denoised_48k.is_empty() {
                Vec::new()
            } else {
                resampler.process(&denoised_48k, channels)?
            }
        } else {
            denoised_48k
        };
        self.pending_out.extend(out);

        let wanted = samples.len();
        if self.pending_out.len() < wanted {
            // keep timing stable; pad with zeros if resampler/output not ready
            let mut result = Vec::with_capacity(wanted);
            result.extend(self.pending_out.drain(..));
            result.resize(wanted, 0.0);
            return Ok(result);
        }
        Ok(self.pending_out.drain(..wanted).collect())
    }
}

#[allow(clippy::too_many_arguments)]
fn run_route(
    input: &str,
    outputs: &[(String, Arc<AtomicU32>, Arc<AtomicU32>)],
    input_gain: &Arc<AtomicU32>,
    delay_ms: &Arc<AtomicU32>,
    pan: &Arc<AtomicI8>,
    noise_suppression: &Arc<AtomicBool>,
    stop: &AtomicBool,
    level: &AtomicU32,
) -> windows::core::Result<()> {
    let _com = ComGuard::new();
    unsafe {
        run_route_inner(
            input,
            outputs,
            input_gain,
            delay_ms,
            pan,
            noise_suppression,
            stop,
            level,
        )
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn run_route_inner(
    input: &str,
    outputs: &[(String, Arc<AtomicU32>, Arc<AtomicU32>)],
    input_gain: &Arc<AtomicU32>,
    delay_ms: &Arc<AtomicU32>,
    pan: &Arc<AtomicI8>,
    noise_suppression: &Arc<AtomicBool>,
    stop: &AtomicBool,
    level: &AtomicU32,
) -> windows::core::Result<()> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

    let fixed_format: WAVEFORMATEX;
    let mut mix_format: Option<CoTaskMem> = None;
    let (capture_client, stream_flags, capture_format) =
        if let Some(pid) = input.strip_prefix("session:") {
            let pid: u32 = pid.parse().map_err(|_| {
                windows::core::Error::new(E_FAIL, "identifiant de session invalide")
            })?;
            fixed_format = fixed_process_format();
            (
                activate_process_loopback(pid)?,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                &fixed_format as *const WAVEFORMATEX,
            )
        } else {
            let input_raw = device_id(input, "input:")?;
            let input_device = enumerator.GetDevice(&HSTRING::from(input_raw))?;
            let client: IAudioClient = input_device.Activate(CLSCTX_ALL, None)?;
            let format = client.GetMixFormat()?;
            mix_format = Some(CoTaskMem(format as *const core::ffi::c_void));
            (client, 0, format as *const WAVEFORMATEX)
        };

    let info = parse_format(capture_format)?;
    let render_format = stereo_format(info.sample_rate);

    capture_client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        stream_flags,
        BUFFER_DURATION,
        0,
        capture_format,
        None,
    )?;

    let capture: IAudioCaptureClient = capture_client.GetService()?;

    let channels = info.channels;
    let max_ring = info.sample_rate as usize * RENDER_CHANNELS;
    let mut delay_line = VecDeque::new();
    let mut nr: Option<NoiseSuppressor> = None;
    let mut nr_failed = false;
    let mut targets = Vec::new();
    for (output, gain, output_level) in outputs {
        match open_render(&enumerator, output, &render_format) {
            Ok((client, render, frames)) => targets.push(RenderTarget {
                client,
                render,
                frames,
                ring: VecDeque::with_capacity(max_ring),
                gain: Arc::clone(gain),
                level: Arc::clone(output_level),
            }),
            Err(err) => crate::logging::log("route", &format!("sortie ignorée ({output}): {err}")),
        }
    }

    let mut started = Vec::new();
    for target in targets {
        match target.client.Start() {
            Ok(()) => started.push(target),
            Err(err) => {
                crate::logging::log("route", &format!("démarrage sortie impossible: {err}"))
            }
        }
    }
    let mut targets = started;
    if !outputs.is_empty() && targets.is_empty() {
        return Err(windows::core::Error::new(
            E_FAIL,
            "aucune sortie audio disponible",
        ));
    }

    capture_client.Start()?;

    while !stop.load(Ordering::Relaxed) {
        let gain = f32::from_bits(input_gain.load(Ordering::Relaxed));
        let wanted_delay = delay_line_samples(delay_ms.load(Ordering::Relaxed), info.sample_rate);
        resize_delay_line(&mut delay_line, wanted_delay);
        let pan_value = pan.load(Ordering::Relaxed);
        let mut peak = 0.0_f32;
        let mut got = false;
        loop {
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            match capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) {
                Ok(()) => {}
                Err(err) if err.code() == AUDCLNT_S_BUFFER_EMPTY => break,
                Err(err) => return Err(err),
            }
            if frames == 0 {
                break;
            }
            got = true;

            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            let source: Vec<f32> = if silent || data.is_null() {
                vec![0.0; frames as usize * channels]
            } else {
                let sample_count = frames as usize * channels;
                let samples = std::slice::from_raw_parts(data as *const f32, sample_count);
                samples.to_vec()
            };
            capture.ReleaseBuffer(frames)?;

            let nr_enabled =
                noise_suppression.load(Ordering::Relaxed) && input.starts_with("input:");
            let denoised = if nr_enabled && !nr_failed {
                if nr
                    .as_ref()
                    .is_none_or(|state| !state.matches(info.sample_rate, channels))
                {
                    nr = None;
                    match NoiseSuppressor::new(info.sample_rate, channels) {
                        Ok(state) => nr = Some(state),
                        Err(err) => {
                            nr_failed = true;
                            crate::logging::log(
                                "nr",
                                &format!("NR désactivée sur {input} (init): {err}"),
                            );
                        }
                    }
                }
                if let Some(state) = nr.as_mut() {
                    match state.process(&source, channels) {
                        Ok(processed) => processed,
                        Err(err) => {
                            nr = None;
                            nr_failed = true;
                            crate::logging::log(
                                "nr",
                                &format!("NR désactivée sur {input} (runtime): {err}"),
                            );
                            source
                        }
                    }
                } else {
                    source
                }
            } else {
                if !nr_enabled {
                    nr = None;
                    nr_failed = false;
                }
                source
            };

            peak = peak.max(scaled_peak(denoised.iter(), gain));
            let processed = stereo_samples(&denoised, channels, gain, pan_value);

            delay_line.extend(processed.iter().copied());
            let delayed: Vec<f32> = delay_line.drain(..processed.len()).collect();
            for target in &mut targets {
                target.ring.extend(delayed.iter().copied());
                while target.ring.len() > max_ring {
                    target.ring.pop_front();
                }
            }
        }
        if got {
            level.store(peak.to_bits(), Ordering::Relaxed);
        }

        for target in &mut targets {
            let padding = target.client.GetCurrentPadding()?;
            let available = target.frames.saturating_sub(padding);
            if available == 0 {
                continue;
            }

            let data = target.render.GetBuffer(available)?;
            let sample_count = available as usize * RENDER_CHANNELS;
            let output_samples = std::slice::from_raw_parts_mut(data as *mut f32, sample_count);
            let copied = target.ring.len().min(sample_count);
            let gain_value = f32::from_bits(target.gain.load(Ordering::Relaxed));
            let mut peak = 0.0_f32;
            for (slot, sample) in output_samples.iter_mut().zip(target.ring.drain(..copied)) {
                let value = sample * gain_value;
                *slot = value;
                peak = peak.max(value.abs());
            }
            for slot in output_samples.iter_mut().skip(copied) {
                *slot = 0.0;
            }
            target.level.store(peak.to_bits(), Ordering::Relaxed);
            let _ = target.render.ReleaseBuffer(available, 0);
        }

        std::thread::sleep(POLL_INTERVAL);
    }

    let _ = capture_client.Stop();
    for target in &targets {
        let _ = target.client.Stop();
        target.level.store(0.0_f32.to_bits(), Ordering::Relaxed);
    }
    level.store(0.0_f32.to_bits(), Ordering::Relaxed);
    drop(mix_format);
    Ok(())
}

unsafe fn open_render(
    enumerator: &IMMDeviceEnumerator,
    output: &str,
    format: *const WAVEFORMATEX,
) -> windows::core::Result<(IAudioClient, IAudioRenderClient, u32)> {
    let output_raw = device_id(output, "output:")?;
    let device = enumerator.GetDevice(&HSTRING::from(output_raw))?;
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
        BUFFER_DURATION,
        0,
        format,
        None,
    )?;
    let render: IAudioRenderClient = client.GetService()?;
    let frames = client.GetBufferSize()?;
    Ok((client, render, frames))
}

struct ActivationState {
    done: Mutex<bool>,
    signal: Condvar,
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    state: Arc<ActivationState>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        if let Ok(mut done) = self.state.done.lock() {
            *done = true;
            self.state.signal.notify_all();
        }
        Ok(())
    }
}

unsafe fn activate_process_loopback(pid: u32) -> windows::core::Result<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };

    let mut prop = ManuallyDrop::new(PROPVARIANT::default());
    prop.Anonymous = PROPVARIANT_0 {
        Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
            vt: VT_BLOB,
            wReserved1: 0,
            wReserved2: 0,
            wReserved3: 0,
            Anonymous: PROPVARIANT_0_0_0 {
                blob: BLOB {
                    cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                    pBlobData: &mut params as *mut _ as *mut u8,
                },
            },
        }),
    };

    let state = Arc::new(ActivationState {
        done: Mutex::new(false),
        signal: Condvar::new(),
    });
    let handler: IActivateAudioInterfaceCompletionHandler = ActivationHandler {
        state: Arc::clone(&state),
    }
    .into();

    let operation = ActivateAudioInterfaceAsync(
        VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        &IAudioClient::IID,
        Some(&*prop as *const PROPVARIANT),
        &handler,
    )?;

    let mut done = state
        .done
        .lock()
        .map_err(|_| windows::core::Error::new(E_FAIL, "verrou d'activation corrompu"))?;
    while !*done {
        done = state
            .signal
            .wait(done)
            .map_err(|_| windows::core::Error::new(E_FAIL, "activation interrompue"))?;
    }
    drop(done);

    let mut hr = HRESULT(0);
    let mut interface: Option<IUnknown> = None;
    operation.GetActivateResult(&mut hr, &mut interface)?;
    hr.ok()?;
    interface
        .ok_or_else(|| windows::core::Error::new(E_FAIL, "activation sans interface"))?
        .cast::<IAudioClient>()
}

fn stereo_format(sample_rate: u32) -> WAVEFORMATEX {
    const CHANNELS: u16 = 2;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT_TAG,
        nChannels: CHANNELS,
        nSamplesPerSec: sample_rate,
        nAvgBytesPerSec: sample_rate * CHANNELS as u32 * 4,
        nBlockAlign: CHANNELS * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

fn fixed_process_format() -> WAVEFORMATEX {
    const CHANNELS: u16 = 2;
    const SAMPLE_RATE: u32 = 48_000;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT_TAG,
        nChannels: CHANNELS,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * CHANNELS as u32 * 4,
        nBlockAlign: CHANNELS * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

unsafe fn parse_format(format: *const WAVEFORMATEX) -> windows::core::Result<FormatInfo> {
    let header = std::ptr::read_unaligned(format);
    let is_float = match header.wFormatTag {
        WAVE_FORMAT_IEEE_FLOAT_TAG => header.wBitsPerSample == 32,
        WAVE_FORMAT_EXTENSIBLE_TAG => {
            let extensible = format as *const WAVEFORMATEXTENSIBLE;
            let sub_format = std::ptr::read_unaligned(std::ptr::addr_of!((*extensible).SubFormat));
            sub_format == IEEE_FLOAT_SUBFORMAT && header.wBitsPerSample == 32
        }
        _ => false,
    };

    if !is_float || header.nChannels == 0 || header.nSamplesPerSec == 0 {
        return Err(windows::core::Error::new(
            E_FAIL,
            "format audio non supporté (f32 requis)",
        ));
    }

    Ok(FormatInfo {
        channels: header.nChannels as usize,
        sample_rate: header.nSamplesPerSec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished_handle() -> std::thread::JoinHandle<()> {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _ = tx.send(());
        });
        rx.recv().unwrap();
        while !handle.is_finished() {
            std::thread::yield_now();
        }
        handle
    }

    #[test]
    fn output_set_ignores_gain_changes() {
        fn active(id: &str, gain: f32) -> ActiveOutput {
            ActiveOutput {
                id: id.to_string(),
                gain: Arc::new(AtomicU32::new(gain.to_bits())),
                level: Arc::new(AtomicU32::new(0)),
            }
        }

        let active = vec![active("output:a", 0.5), active("output:b", 0.5)];
        let wanted = vec![
            OutputSpec {
                id: "output:a".into(),
                gain: 0.2,
            },
            OutputSpec {
                id: "output:b".into(),
                gain: 0.9,
            },
        ];
        assert!(same_outputs(&active, &wanted));
        assert!(!same_outputs(&active, &wanted[..1]));
    }

    #[test]
    fn scaled_peak_follows_source_gain() {
        let samples = [0.5_f32, -0.25, 0.1];
        assert!((super::scaled_peak(samples.iter(), 2.0) - 1.0).abs() < 1e-6);
        assert!((super::scaled_peak(samples.iter(), 0.5) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn pan_gains_follow_equal_power_law() {
        let (left, right) = super::pan_gains(0);
        assert!((left - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((right - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(super::pan_gains(-100), (1.0, 0.0));
        assert_eq!(super::pan_gains(100), (0.0, 1.0));
    }

    #[test]
    fn stereo_samples_upmix_mono_and_pan() {
        let center = super::stereo_samples(&[1.0], 1, 1.0, 0);
        assert!((center[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((center[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);

        let left = super::stereo_samples(&[0.5, 0.25], 2, 1.0, -100);
        assert_eq!(left, vec![0.5, 0.0]);
    }

    #[test]
    fn delay_line_holds_zeros_then_releases_aligned_samples() {
        assert_eq!(
            super::delay_line_samples(super::MAX_DELAY_MS, 48_000),
            480_000
        );
        assert_eq!(super::delay_line_samples(10_000, 48_000), 480_000);
        assert_eq!(super::delay_line_samples(500, 48_000), 48_000);

        let mut line = VecDeque::new();
        super::resize_delay_line(&mut line, 4);
        assert_eq!(line.len(), 4);
        assert!(line.iter().all(|sample| *sample == 0.0));

        line.extend([1.0, 2.0]);
        let delayed: Vec<f32> = line.drain(..2).collect();
        assert_eq!(delayed, vec![0.0, 0.0]);
        assert_eq!(
            line.iter().copied().collect::<Vec<_>>(),
            vec![0.0, 0.0, 1.0, 2.0]
        );

        super::resize_delay_line(&mut line, 2);
        assert_eq!(line.iter().copied().collect::<Vec<_>>(), vec![1.0, 2.0]);
    }

    #[test]
    fn set_params_updates_gains_delay_and_pan_without_new_outputs() {
        let route = ActiveRoute {
            input: "input:mic".into(),
            input_gain: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            delay_ms: Arc::new(AtomicU32::new(0)),
            pan: Arc::new(AtomicI8::new(0)),
            noise_suppression: Arc::new(AtomicBool::new(false)),
            outputs: vec![ActiveOutput {
                id: "output:a".into(),
                gain: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
                level: Arc::new(AtomicU32::new(0)),
            }],
            level: Arc::new(AtomicU32::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
            retry_at: Instant::now(),
        };
        let spec = RouteSpec {
            input: "input:mic".into(),
            input_gain: 2.0,
            delay_ms: 250,
            pan: 40,
            noise_suppression: true,
            outputs: vec![OutputSpec {
                id: "output:a".into(),
                gain: 0.5,
            }],
        };

        route.set_params(&spec);

        assert_eq!(
            f32::from_bits(route.input_gain.load(Ordering::Relaxed)),
            2.0
        );
        assert_eq!(route.delay_ms.load(Ordering::Relaxed), 250);
        assert_eq!(route.pan.load(Ordering::Relaxed), 40);
        assert!(route.noise_suppression.load(Ordering::Relaxed));
        assert_eq!(
            f32::from_bits(route.outputs[0].gain.load(Ordering::Relaxed)),
            0.5
        );
    }

    #[test]
    fn safe_gain_clamps_extremes_and_silences_non_finite_values() {
        assert_eq!(safe_gain(-1.0), 0.0);
        assert_eq!(safe_gain(100.0), MAX_GAIN);
        assert_eq!(safe_gain(f32::NAN), 0.0);
        assert_eq!(safe_gain(f32::INFINITY), 0.0);
        assert_eq!(safe_gain(0.5), 0.5);
        let full_trim = 10.0_f32.powf(24.0 / 20.0);
        assert_eq!(safe_gain(full_trim), full_trim);
    }

    #[test]
    fn device_id_requires_prefix_and_rejects_control_characters() {
        assert_eq!(device_id("output:speakers", "output:").unwrap(), "speakers");
        assert!(device_id("speakers", "output:").is_err());
        assert!(device_id("output:", "output:").is_err());
        assert!(device_id("output:a\0b", "output:").is_err());
    }

    #[test]
    fn dead_route_restarts_only_after_delay() {
        let now = Instant::now();
        let finished = Some(finished_handle());
        assert!(should_restart(&finished, now, now - RETRY_DELAY));
        assert!(!should_restart(&finished, now, now + RETRY_DELAY));

        let missing: Option<std::thread::JoinHandle<()>> = None;
        assert!(should_restart(&missing, now, now - RETRY_DELAY));

        let running = Some(std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(1));
        }));
        assert!(!should_restart(&running, now, now - RETRY_DELAY));
    }
}
