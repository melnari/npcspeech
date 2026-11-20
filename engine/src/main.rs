use npcspeech_engine::control::{Controls, InputDevice, Shared, start_server};
use npcspeech_engine::dsp::{EffectChain, MAX_BLOCK, SAMPLE_RATE, VoiceProcessor};
use npcspeech_engine::ring::AudioRing;
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use spa::pod::Pod;
use std::cell::RefCell;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

const TRANSITION_FRAMES: usize = 240;

struct CaptureHandle {
    // Drop the listener before destroying the stream.
    _listener: pw::stream::StreamListener<Arc<AudioRing>>,
    _stream: pw::stream::StreamRc,
    target: String,
}

struct OutputState {
    ring: Arc<AudioRing>,
    controls: Arc<Controls>,
    chain: EffectChain,
    version: u64,
    input: Vec<f32>,
    processed: Vec<f32>,
    wet: bool,
    previous_wet: bool,
    transition: usize,
    warmup: usize,
    last_overruns: u64,
}

impl OutputState {
    fn new(ring: Arc<AudioRing>, controls: Arc<Controls>) -> Self {
        Self {
            ring,
            controls,
            chain: EffectChain::new(Default::default()),
            version: 0,
            input: vec![0.0; MAX_BLOCK],
            processed: vec![0.0; MAX_BLOCK],
            wet: false,
            previous_wet: false,
            transition: 0,
            warmup: 0,
            last_overruns: 0,
        }
    }

    fn fill(&mut self, bytes: &mut [u8]) {
        let overruns = self.ring.overruns();
        if overruns != self.last_overruns {
            self.ring.trim_to(0);
            self.last_overruns = overruns;
        } else {
            self.ring.trim_to(1024);
        }
        let mut effects_changed = false;
        if let Some((version, effects)) = self.controls.snapshot_if_changed(self.version) {
            self.chain.set_effects(effects);
            self.version = version;
            effects_changed = true;
            self.controls
                .latency_frames
                .store(self.chain.latency_frames() as u64, Ordering::Relaxed);
        }
        let desired_wet = self.controls.selected.load(Ordering::Acquire)
            && !self.controls.bypass.load(Ordering::Acquire);
        if desired_wet != self.wet || (desired_wet && effects_changed) {
            self.previous_wet = self.wet;
            self.wet = desired_wet;
            self.transition = TRANSITION_FRAMES;
            if desired_wet {
                self.chain.reset();
                self.warmup = self.chain.latency_frames();
                self.previous_wet = false;
            } else {
                self.warmup = 0;
            }
        }

        for block in bytes.chunks_mut(MAX_BLOCK * 4) {
            let frames = block.len() / 4;
            for sample in &mut self.input[..frames] {
                *sample = self.ring.pop();
            }
            let need_effect = self.wet || (self.previous_wet && self.transition > 0);
            if need_effect {
                self.chain
                    .process(&self.input[..frames], &mut self.processed[..frames]);
            }
            for (index, frame) in block.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let dry = self.input[index];
                let wet = if need_effect {
                    self.processed[index]
                } else {
                    dry
                };
                let sample = if self.wet && self.warmup > 0 {
                    self.warmup -= 1;
                    dry
                } else if self.transition > 0 {
                    let progress = 1.0 - self.transition as f32 / TRANSITION_FRAMES as f32;
                    self.transition -= 1;
                    if self.wet {
                        dry * (1.0 - progress) + wet * progress
                    } else {
                        wet * (1.0 - progress) + dry * progress
                    }
                } else if self.wet {
                    wet
                } else {
                    dry
                };
                frame.copy_from_slice(&sample.to_le_bytes());
            }
        }
    }
}

fn audio_format() -> Vec<u8> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(SAMPLE_RATE);
    info.set_channels(1);
    let mut positions = [0; spa::param::audio::MAX_CHANNELS];
    positions[0] = spa::sys::SPA_AUDIO_CHANNEL_MONO;
    info.set_position(positions);
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: spa::param::ParamType::EnumFormat.as_raw(),
            properties: info.into(),
        }),
    )
    .expect("audio format serialization failed")
    .0
    .into_inner()
}

fn create_capture(
    core: pw::core::CoreRc,
    target: String,
    ring: Arc<AudioRing>,
) -> Result<CaptureHandle, Box<dyn std::error::Error>> {
    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Communication",
        *pw::keys::NODE_NAME => "npcspeech.capture",
    };
    props.insert("target.object", target.as_str());
    props.insert(*pw::keys::NODE_LATENCY, "256/48000");
    let stream = pw::stream::StreamRc::new(core, "NPCSpeech capture", props)?;
    let listener = stream
        .add_local_listener_with_user_data(ring)
        .process(|stream, ring| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let offset = data.chunk().offset() as usize;
            let size = data.chunk().size() as usize;
            let Some(bytes) = data.data() else { return };
            let end = offset.saturating_add(size).min(bytes.len());
            if offset >= end {
                return;
            }
            for frame in bytes[offset..end].as_chunks::<4>().0 {
                ring.push(f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]));
            }
        })
        .register()?;
    let format = audio_format();
    let mut params = [Pod::from_bytes(&format).expect("format pod")];
    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;
    Ok(CaptureHandle {
        _listener: listener,
        _stream: stream,
        target,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let shared = Arc::new(Shared::new());

    let registry = core.get_registry_rc()?;
    let shared_for_registry = Arc::clone(&shared);
    let _registry_listener = registry
        .add_listener_local()
        .global(move |object| {
            if object.type_ != pw::types::ObjectType::Node {
                return;
            }
            let Some(props) = &object.props else {
                return;
            };
            let class = props.get(*pw::keys::MEDIA_CLASS).unwrap_or("");
            if !class.starts_with("Audio/Source") {
                return;
            }
            let Some(name) = props.get(*pw::keys::NODE_NAME) else {
                return;
            };
            if name.starts_with("npcspeech.") {
                return;
            }
            let description = props.get(*pw::keys::NODE_DESCRIPTION).unwrap_or(name);
            let mut devices = shared_for_registry.devices.lock().unwrap();
            devices.push(InputDevice {
                node_id: object.id,
                node_name: name.to_owned(),
                description: description.to_owned(),
            });
            if (name.starts_with("alsa_input") || name.starts_with("bluez_input"))
                && shared_for_registry.input.lock().unwrap().is_none()
            {
                *shared_for_registry.input.lock().unwrap() = Some(name.to_owned());
            }
        })
        .global_remove({
            let shared = Arc::clone(&shared);
            move |id| {
                let available = {
                    let mut devices = shared.devices.lock().unwrap();
                    devices.retain(|device| device.node_id != id);
                    devices
                        .iter()
                        .map(|device| device.node_name.clone())
                        .collect::<Vec<_>>()
                };
                let mut input = shared.input.lock().unwrap();
                if input.as_ref().is_some_and(|name| !available.contains(name)) {
                    *input = None;
                }
            }
        })
        .register();

    let output = pw::stream::StreamRc::new(
        core.clone(),
        "NPCSpeech microphone",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::MEDIA_CLASS => "Audio/Source",
            *pw::keys::NODE_NAME => "npcspeech.virtual_source",
            *pw::keys::NODE_DESCRIPTION => "NPCSpeech Microphone",
            *pw::keys::NODE_VIRTUAL => "true",
            *pw::keys::AUDIO_CHANNELS => "1",
            *pw::keys::NODE_LATENCY => "256/48000",
        },
    )?;
    let shared_for_output_state = Arc::clone(&shared);
    let _output_listener = output
        .add_local_listener_with_user_data(OutputState::new(
            Arc::clone(&shared.ring),
            Arc::clone(&shared.controls),
        ))
        .state_changed(move |_, state, _, new| {
            let active = new == pw::stream::StreamState::Streaming;
            shared_for_output_state
                .output_active
                .store(active, Ordering::Release);
            if !active {
                state.ring.trim_to(0);
            }
        })
        .process(|stream, state| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let requested = buffer.requested() as usize;
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let Some(bytes) = data.data() else { return };
            let size = if requested > 0 {
                bytes.len().min(requested.saturating_mul(4))
            } else {
                bytes.len()
            };
            let size = size - size % 4;
            state.fill(&mut bytes[..size]);
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = 4;
            *chunk.size_mut() = size as u32;
        })
        .register()?;
    let format = audio_format();
    let mut params = [Pod::from_bytes(&format).expect("format pod")];
    output.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    let path = start_server(Arc::clone(&shared))?;
    let loop_weak = mainloop.downgrade();
    let _sig_int = mainloop
        .loop_()
        .add_signal_local(pw::loop_::Signal::INT, move || {
            if let Some(loop_) = loop_weak.upgrade() {
                loop_.quit();
            }
        });
    let loop_weak = mainloop.downgrade();
    let _sig_term = mainloop
        .loop_()
        .add_signal_local(pw::loop_::Signal::TERM, move || {
            if let Some(loop_) = loop_weak.upgrade() {
                loop_.quit();
            }
        });

    let capture = RefCell::new(None::<CaptureHandle>);
    let loop_weak = mainloop.downgrade();
    let shared_for_timer = Arc::clone(&shared);
    let _timer = mainloop.loop_().add_timer(move |_| {
        if shared_for_timer.shutdown.load(Ordering::Acquire) {
            if let Some(loop_) = loop_weak.upgrade() {
                loop_.quit();
            }
            return;
        }
        let wanted = if shared_for_timer.output_active.load(Ordering::Acquire) {
            shared_for_timer.input.lock().unwrap().clone()
        } else {
            None
        };
        let mut current = capture.borrow_mut();
        if current.as_ref().map(|item| &item.target) != wanted.as_ref() {
            *current = None;
            if let Some(target) = wanted {
                match create_capture(core.clone(), target, Arc::clone(&shared_for_timer.ring)) {
                    Ok(handle) => *current = Some(handle),
                    Err(error) => eprintln!("Unable to open microphone: {error}"),
                }
            }
        }
    });
    _timer
        .update_timer(
            Some(Duration::from_millis(20)),
            Some(Duration::from_millis(20)),
        )
        .into_result()?;
    println!(
        "NPCSpeech virtual microphone is running. Control socket: {}",
        path.display()
    );
    mainloop.run();
    let _ = fs::remove_file(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use npcspeech_engine::preset::Effects;
    use std::f32::consts::TAU;

    fn output_block(state: &mut OutputState, ring: &AudioRing, input: &[f32]) -> Vec<f32> {
        for &sample in input {
            ring.push(sample);
        }
        let mut bytes = vec![0; input.len() * 4];
        state.fill(&mut bytes);
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|frame| f32::from_le_bytes(*frame))
            .collect()
    }

    #[test]
    fn original_preset_and_bypass_route_through_expected_paths() {
        let ring = Arc::new(AudioRing::new(16_384));
        let controls = Arc::new(Controls::new());
        let mut state = OutputState::new(Arc::clone(&ring), Arc::clone(&controls));

        let dry_input: Vec<f32> = (0..256)
            .map(|frame| (frame as f32 * TAU * 220.0 / SAMPLE_RATE as f32).sin() * 0.3)
            .collect();
        assert_eq!(output_block(&mut state, &ring, &dry_input), dry_input);

        controls.set_effects(Effects {
            pitch_semitones: -12.0,
            ..Effects::default()
        });
        controls.selected.store(true, Ordering::Release);
        let mut wet = Vec::new();
        for block in 0..188 {
            let input: Vec<f32> = (0..256)
                .map(|offset| {
                    let frame = (block + 1) * 256 + offset;
                    (frame as f32 * TAU * 220.0 / SAMPLE_RATE as f32).sin() * 0.3
                })
                .collect();
            let output = output_block(&mut state, &ring, &input);
            if block < 8 {
                assert_eq!(output, input, "dry signal must continue during warm-up");
            }
            wet.extend(output);
        }

        fn energy_at(samples: &[f32], hz: f32) -> f32 {
            let step = TAU * hz / SAMPLE_RATE as f32;
            let (real, imag) = samples.iter().enumerate().fold((0.0, 0.0), |sum, (i, x)| {
                let phase = i as f32 * step;
                (sum.0 + x * phase.cos(), sum.1 + x * phase.sin())
            });
            real * real + imag * imag
        }
        let steady = &wet[24_000..28_800];
        assert!(energy_at(steady, 110.0) > energy_at(steady, 220.0) * 4.0);

        controls.bypass.store(true, Ordering::Release);
        output_block(&mut state, &ring, &dry_input); // Crossfade block.
        assert_eq!(output_block(&mut state, &ring, &dry_input), dry_input);
    }
}
