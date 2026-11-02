use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::E_NOINTERFACE;
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    AUDCLNT_STREAMOPTIONS_NONE, AUDIO_EFFECT, AUDIO_EFFECT_STATE_ON, AudioCategory_Communications,
    AudioClientProperties, IAcousticEchoCancellationControl, IAudioCaptureClient, IAudioClient2,
    IAudioEffectsManager, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eCapture,
    eMultimedia, eRender,
};
use windows::Win32::Media::KernelStreaming::AUDIO_EFFECT_TYPE_ACOUSTIC_ECHO_CANCELLATION;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::core::PCWSTR;

use super::ComApartment;

const RATE: u32 = 16_000;

/// Availability means an initialized communications stream reports AEC as active. Having a
/// reference-endpoint control alone is insufficient evidence that processing is being applied.
pub fn echo_cancellation_available() -> Result<bool, String> {
    std::thread::Builder::new()
        .name("pennel-aec-probe".into())
        .spawn(|| {
            let _apartment = ComApartment::new()?;
            configured_client().map(|client| client.is_some())
        })
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| "Echo cancellation could not be checked".to_owned())?
}

pub struct EchoCapture {
    stop: Sender<()>,
    worker: Option<JoinHandle<Result<(), String>>>,
    error: Arc<Mutex<Option<String>>>,
}

impl EchoCapture {
    pub fn request_stop(&self) {
        let _ = self.stop.send(());
    }
    /// `sink` receives owned-thread, ephemeral 16 kHz mono Float32 slices. No PCM buffers or COM
    /// objects cross the callback lifetime; stop joins the worker before releasing the closure.
    pub fn start_timed(sink: impl FnMut(&[f32], Instant) + Send + 'static) -> Result<Self, String> {
        let (stop, stopping) = mpsc::channel();
        let (ready_sender, ready) = mpsc::sync_channel(1);
        let error = Arc::new(Mutex::new(None));
        let worker_error = error.clone();
        let worker = std::thread::Builder::new()
            .name("pennel-aec-capture".into())
            .spawn(move || {
                let result = (|| {
                    let _apartment = ComApartment::new()?;
                    let client = configured_client()?.ok_or_else(|| {
                        "This microphone does not provide confirmed echo cancellation".to_owned()
                    })?;
                    let capture: IAudioCaptureClient =
                        unsafe { client.GetService() }.map_err(|error| error.to_string())?;
                    let clock = QpcClock::new()?;
                    unsafe { client.Start() }.map_err(|error| error.to_string())?;
                    let started = StartedClient {
                        client,
                        capture,
                        clock,
                    };
                    let _ = ready_sender.send(Ok(()));
                    started.record(stopping, sink)
                })();
                if let Err(error) = &result {
                    *worker_error.lock().unwrap() = Some(error.clone());
                    let _ = ready_sender.send(Err(error.clone()));
                }
                result
            })
            .map_err(|error| error.to_string())?;
        match ready.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                worker: Some(worker),
                error,
            }),
            result => {
                let _ = stop.send(());
                let _ = worker.join();
                Err(match result {
                    Ok(Err(error)) => error,
                    _ => "The microphone with echo cancellation could not start".into(),
                })
            }
        }
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone().or_else(|| {
            self.worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
                .then(|| "Microphone stopped unexpectedly".to_owned())
        })
    }

    pub fn stop(mut self) -> Result<(), String> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), String> {
        let _ = self.stop.send(());
        self.worker.take().map_or(Ok(()), |worker| {
            worker
                .join()
                .map_err(|_| "Microphone stopped unexpectedly".to_owned())?
        })
    }
}

impl Drop for EchoCapture {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn configured_client() -> Result<Option<IAudioClient2>, String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| error.to_string())?;
        // Match CPAL's default multimedia devices so enabling AEC does not silently switch mics.
        let endpoint = enumerator
            .GetDefaultAudioEndpoint(eCapture, eMultimedia)
            .map_err(|error| error.to_string())?;
        let client: IAudioClient2 = endpoint
            .Activate(CLSCTX_ALL, None)
            .map_err(|error| error.to_string())?;
        client
            .SetClientProperties(&AudioClientProperties {
                cbSize: std::mem::size_of::<AudioClientProperties>() as u32,
                bIsOffload: false.into(),
                eCategory: AudioCategory_Communications,
                Options: AUDCLNT_STREAMOPTIONS_NONE,
            })
            .map_err(|error| error.to_string())?;
        let format = WAVEFORMATEX {
            wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
            nChannels: 1,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * 4,
            nBlockAlign: 4,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                1_000_000,
                0,
                &format,
                None,
            )
            .map_err(|error| error.to_string())?;
        match client.GetService::<IAcousticEchoCancellationControl>() {
            Ok(control) => {
                let render = enumerator
                    .GetDefaultAudioEndpoint(eRender, eMultimedia)
                    .map_err(|error| error.to_string())?;
                let id = render.GetId().map_err(|error| error.to_string())?;
                let configured = control.SetEchoCancellationRenderEndpoint(PCWSTR(id.0));
                CoTaskMemFree(Some(id.0.cast()));
                configured.map_err(|error| error.to_string())?;
            }
            Err(error) if error.code() == E_NOINTERFACE => {}
            Err(error) => return Err(error.to_string()),
        }
        let effects = match client.GetService::<IAudioEffectsManager>() {
            Ok(effects) => effects,
            Err(error) if error.code() == E_NOINTERFACE => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let observed = audio_effects(&effects)?;
        let Some(aec) = observed
            .iter()
            .find(|effect| effect.id == AUDIO_EFFECT_TYPE_ACOUSTIC_ECHO_CANCELLATION)
        else {
            return Ok(None);
        };
        if aec.state != AUDIO_EFFECT_STATE_ON && aec.canSetState.as_bool() {
            effects
                .SetAudioEffectState(aec.id, AUDIO_EFFECT_STATE_ON)
                .map_err(|error| error.to_string())?;
        }
        let active = audio_effects(&effects)?.iter().any(|effect| {
            effect.id == AUDIO_EFFECT_TYPE_ACOUSTIC_ECHO_CANCELLATION
                && effect.state == AUDIO_EFFECT_STATE_ON
        });
        Ok(active.then_some(client))
    }
}

fn audio_effects(manager: &IAudioEffectsManager) -> Result<Vec<AUDIO_EFFECT>, String> {
    let mut effects = std::ptr::null_mut();
    let mut count = 0u32;
    unsafe {
        manager
            .GetAudioEffects(&mut effects, &mut count)
            .map_err(|error| error.to_string())?;
        let result = if count == 0 {
            Ok(Vec::new())
        } else if effects.is_null() || count > 1024 {
            Err("Windows returned an invalid audio effect list".into())
        } else {
            Ok(std::slice::from_raw_parts(effects, count as usize).to_vec())
        };
        CoTaskMemFree(Some(effects.cast()));
        result
    }
}

struct StartedClient {
    client: IAudioClient2,
    capture: IAudioCaptureClient,
    clock: QpcClock,
}

impl StartedClient {
    fn record(
        &self,
        stop: Receiver<()>,
        mut sink: impl FnMut(&[f32], Instant),
    ) -> Result<(), String> {
        loop {
            if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                return self.stop_and_drain(&mut sink);
            }
            self.drain(&mut sink)?;
            match stop.recv_timeout(Duration::from_millis(10)) {
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                _ => return self.stop_and_drain(&mut sink),
            }
        }
    }

    fn stop_and_drain(&self, sink: &mut impl FnMut(&[f32], Instant)) -> Result<(), String> {
        unsafe { self.client.Stop() }.map_err(|error| error.to_string())?;
        self.drain(sink)
    }

    fn drain(&self, sink: &mut impl FnMut(&[f32], Instant)) -> Result<(), String> {
        unsafe {
            while self
                .capture
                .GetNextPacketSize()
                .map_err(|error| error.to_string())?
                > 0
            {
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                let mut qpc = 0;
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))
                    .map_err(|error| error.to_string())?;
                let packet = Packet {
                    capture: &self.capture,
                    frames,
                };
                let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    vec![0.0; frames as usize]
                } else {
                    if data.is_null() && frames != 0 {
                        return Err("Windows returned an invalid microphone buffer".into());
                    }
                    if frames == 0 {
                        Vec::new()
                    } else {
                        let bytes = std::slice::from_raw_parts(data, frames as usize * 4);
                        bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|bytes| f32::from_le_bytes(*bytes))
                            .collect()
                    }
                };
                drop(packet);
                if !samples.is_empty() {
                    let captured_at =
                        if qpc != 0 && flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 == 0 {
                            self.clock.instant(qpc)
                        } else {
                            None
                        }
                        .unwrap_or_else(|| {
                            let now = Instant::now();
                            now.checked_sub(Duration::from_secs_f64(
                                f64::from(frames) / f64::from(RATE),
                            ))
                            .unwrap_or(now)
                        });
                    sink(&samples, captured_at);
                }
            }
        }
        Ok(())
    }
}

struct QpcClock {
    ticks_100ns: u128,
    instant: Instant,
}

impl QpcClock {
    fn new() -> Result<Self, String> {
        let mut frequency = 0;
        let mut counter = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(|error| error.to_string())?;
        let before = Instant::now();
        unsafe { QueryPerformanceCounter(&mut counter) }.map_err(|error| error.to_string())?;
        let after = Instant::now();
        if frequency <= 0 || counter < 0 {
            return Err("Invalid audio clock".into());
        }
        Ok(Self {
            ticks_100ns: counter as u128 * 10_000_000 / frequency as u128,
            instant: before + after.duration_since(before) / 2,
        })
    }

    fn instant(&self, ticks_100ns: u64) -> Option<Instant> {
        let ticks_100ns = u128::from(ticks_100ns);
        let nanoseconds = ticks_100ns.abs_diff(self.ticks_100ns).checked_mul(100)?;
        let duration = Duration::from_nanos(u64::try_from(nanoseconds).ok()?);
        if ticks_100ns >= self.ticks_100ns {
            self.instant.checked_add(duration)
        } else {
            self.instant.checked_sub(duration)
        }
    }
}

impl Drop for StartedClient {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

struct Packet<'a> {
    capture: &'a IAudioCaptureClient,
    frames: u32,
}

impl Drop for Packet<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = self.capture.ReleaseBuffer(self.frames);
        }
    }
}
