//! Real-time output through the default device (CoreAudio on macOS).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use crate::engine::Status;
use crate::session::{Command, Session};

pub struct Output {
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
}

impl Output {
    pub fn open() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| anyhow!("no audio output device found"))?;
        let config = device.default_output_config().context("could not query the output device")?;
        Ok(Output { device, config })
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate().0
    }

    pub fn device_name(&self) -> String {
        self.device.name().unwrap_or_else(|_| "default output".into())
    }

    /// Start playing `session`. Keep the returned stream alive for as long as audio should play.
    pub fn start(
        self,
        session: Session,
        commands: Receiver<Command>,
        status: Arc<Mutex<Status>>,
        finished: Arc<AtomicBool>,
    ) -> Result<cpal::Stream> {
        let cfg: cpal::StreamConfig = self.config.config();
        let latency = latency::output_frames().unwrap_or(0);
        let stream = match self.config.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&self.device, &cfg, latency, session, commands, status, finished)?,
            cpal::SampleFormat::I16 => build::<i16>(&self.device, &cfg, latency, session, commands, status, finished)?,
            cpal::SampleFormat::U16 => build::<u16>(&self.device, &cfg, latency, session, commands, status, finished)?,
            cpal::SampleFormat::I32 => build::<i32>(&self.device, &cfg, latency, session, commands, status, finished)?,
            other => bail!("unsupported sample format {other:?}"),
        };
        stream.play().context("could not start audio playback")?;
        Ok(stream)
    }
}

fn build<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    latency: u32,
    mut session: Session,
    commands: Receiver<Command>,
    status: Arc<Mutex<Status>>,
    finished: Arc<AtomicBool>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = cfg.channels as usize;
    let sr = cfg.sample_rate.0 as f64;
    let mut bl = vec![0f32; 4096];
    let mut br = vec![0f32; 4096];
    let mut since_status = usize::MAX / 2;
    let stream = device.build_output_stream(
        cfg,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let now = Instant::now();
            while let Ok(cmd) = commands.try_recv() {
                session.command(cmd);
            }
            let frames = data.len() / channels;
            let mut done = 0;
            while done < frames {
                let n = (frames - done).min(bl.len());
                session.render(&mut bl[..n], &mut br[..n]);
                for i in 0..n {
                    let frame = &mut data[(done + i) * channels..(done + i + 1) * channels];
                    if channels == 1 {
                        frame[0] = T::from_sample((bl[i] + br[i]) * 0.5);
                    } else {
                        frame[0] = T::from_sample(bl[i]);
                        frame[1] = T::from_sample(br[i]);
                        for s in frame.iter_mut().skip(2) {
                            *s = T::from_sample(0.0f32);
                        }
                    }
                }
                done += n;
            }
            since_status += frames;
            if since_status >= 2048 {
                if let Ok(mut st) = status.try_lock() {
                    session.fill_status(&mut st);
                    // The callback runs about a buffer ahead of the hardware, so the end of this
                    // buffer is heard two buffers plus the device's latency from now.
                    let ahead = (2 * frames) as f64 + latency as f64;
                    st.heard_at = Some(now + Duration::from_secs_f64(ahead / sr));
                    since_status = 0;
                }
            }
            if session.finished() {
                finished.store(true, Ordering::Relaxed);
            }
        },
        |err| eprintln!("audio error: {err}"),
        None,
    )?;
    Ok(stream)
}

/// Output latency the device reports beyond our buffer: its safety offset plus device and
/// stream latency, in frames. About 160 ms for AirPods, 15–30 ms for built-in speakers.
#[cfg(target_os = "macos")]
mod latency {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::null;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            object: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }

    const SYSTEM: u32 = 1;

    fn address(selector: &[u8; 4], scope: &[u8; 4]) -> Address {
        Address { selector: u32::from_be_bytes(*selector), scope: u32::from_be_bytes(*scope), element: 0 }
    }

    fn get_u32(object: u32, selector: &[u8; 4], scope: &[u8; 4]) -> Option<u32> {
        let a = address(selector, scope);
        let mut value = 0u32;
        let mut size = size_of::<u32>() as u32;
        let err = unsafe { AudioObjectGetPropertyData(object, &a, 0, null(), &mut size, &mut value as *mut u32 as *mut c_void) };
        (err == 0).then_some(value)
    }

    fn get_ids(object: u32, selector: &[u8; 4], scope: &[u8; 4]) -> Vec<u32> {
        let a = address(selector, scope);
        let mut size = 0u32;
        if unsafe { AudioObjectGetPropertyDataSize(object, &a, 0, null(), &mut size) } != 0 {
            return vec![];
        }
        let mut ids = vec![0u32; size as usize / size_of::<u32>()];
        let err = unsafe { AudioObjectGetPropertyData(object, &a, 0, null(), &mut size, ids.as_mut_ptr() as *mut c_void) };
        if err != 0 {
            return vec![];
        }
        ids.truncate(size as usize / size_of::<u32>());
        ids
    }

    pub fn output_frames() -> Option<u32> {
        let device = get_u32(SYSTEM, b"dOut", b"glob")?;
        let safety = get_u32(device, b"saft", b"outp").unwrap_or(0);
        let device_latency = get_u32(device, b"ltnc", b"outp").unwrap_or(0);
        let stream_latency = get_ids(device, b"stm#", b"outp")
            .first()
            .and_then(|&s| get_u32(s, b"ltnc", b"glob"))
            .unwrap_or(0);
        Some(safety + device_latency + stream_latency)
    }
}

#[cfg(not(target_os = "macos"))]
mod latency {
    pub fn output_frames() -> Option<u32> {
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn reports_a_plausible_output_latency() {
        // Machines without an output device report nothing; that's fine.
        if let Some(frames) = super::latency::output_frames() {
            assert!(frames < 48_000, "{frames} frames");
        }
    }
}
