//! Allocation-free block processing after construction.

use crate::preset::Effects;
use signalsmith_stretch::Stretch;
use std::f32::consts::TAU;

pub const SAMPLE_RATE: u32 = 48_000;
pub const MAX_BLOCK: usize = 8192;

/// Boundary reserved for future voice engines. A neural implementation may
/// process on a worker thread and report a larger latency than this DSP engine.
pub trait VoiceProcessor {
    fn process(&mut self, input: &[f32], output: &mut [f32]);
    fn latency_frames(&self) -> usize;
}

/// Two state variables and five coefficients per band; no allocation in audio processing.
struct PeakingEq {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl PeakingEq {
    fn new(hz: f32, gain_db: f32, q: f32) -> Self {
        let amplitude = 10.0_f32.powf(gain_db / 40.0);
        let angle = TAU * hz / SAMPLE_RATE as f32;
        let alpha = angle.sin() / (2.0 * q);
        let a0 = 1.0 + alpha / amplitude;
        Self {
            b0: (1.0 + alpha * amplitude) / a0,
            b1: -2.0 * angle.cos() / a0,
            b2: (1.0 - alpha * amplitude) / a0,
            a1: -2.0 * angle.cos() / a0,
            a2: (1.0 - alpha / amplitude) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * output + self.z2;
        self.z2 = self.b2 * input - self.a2 * output;
        output
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

pub struct EffectChain {
    effects: Effects,
    stretch: Stretch,
    spectral: Vec<f32>,
    delay: Vec<f32>,
    delay_cursor: usize,
    highpass_x: f32,
    highpass_y: f32,
    lowpass_y: f32,
    body_eq: PeakingEq,
    presence_eq: PeakingEq,
    envelope: f32,
    modulation_phase: f32,
    noise_state: u32,
}

impl EffectChain {
    pub fn new(effects: Effects) -> Self {
        assert!(effects.validate().is_ok());
        let mut stretch = Stretch::new(1, 2048, 512);
        stretch.signalsmith_stretch_set_formant_base(0.0);
        stretch.set_transpose_factor_semitones(
            effects.pitch_semitones,
            Some(8000.0 / SAMPLE_RATE as f32),
        );
        stretch.set_formant_factor_semitones(effects.formant_semitones, true);
        Self {
            effects,
            stretch,
            spectral: vec![0.0; MAX_BLOCK],
            delay: vec![0.0; (SAMPLE_RATE as f32 * 0.35) as usize + 1],
            delay_cursor: 0,
            highpass_x: 0.0,
            highpass_y: 0.0,
            lowpass_y: 0.0,
            body_eq: PeakingEq::new(effects.body_hz, effects.body_db, 0.9),
            presence_eq: PeakingEq::new(effects.presence_hz, effects.presence_db, 1.1),
            envelope: 0.0,
            modulation_phase: 0.0,
            noise_state: 0x1234_5678,
        }
    }

    pub fn effects(&self) -> Effects {
        self.effects
    }

    /// Called at a block boundary when a preset changes. Filter state is kept
    /// to avoid clicks; the pitch shifter is already allocated.
    pub fn set_effects(&mut self, effects: Effects) {
        assert!(effects.validate().is_ok());
        if effects.pitch_semitones != self.effects.pitch_semitones {
            self.stretch.set_transpose_factor_semitones(
                effects.pitch_semitones,
                Some(8000.0 / SAMPLE_RATE as f32),
            );
        }
        if effects.formant_semitones != self.effects.formant_semitones {
            self.stretch
                .set_formant_factor_semitones(effects.formant_semitones, true);
        }
        if effects.body_hz != self.effects.body_hz || effects.body_db != self.effects.body_db {
            self.body_eq = PeakingEq::new(effects.body_hz, effects.body_db, 0.9);
        }
        if effects.presence_hz != self.effects.presence_hz
            || effects.presence_db != self.effects.presence_db
        {
            self.presence_eq = PeakingEq::new(effects.presence_hz, effects.presence_db, 1.1);
        }
        self.effects = effects;
    }

    /// Clear old voice data before re-enabling a processor after bypass.
    pub fn reset(&mut self) {
        self.stretch.reset();
        self.delay.fill(0.0);
        self.delay_cursor = 0;
        self.highpass_x = 0.0;
        self.highpass_y = 0.0;
        self.lowpass_y = 0.0;
        self.body_eq.reset();
        self.presence_eq.reset();
        self.envelope = 0.0;
        self.modulation_phase = 0.0;
    }

    fn uses_spectral(&self) -> bool {
        self.effects.pitch_semitones != 0.0 || self.effects.formant_semitones != 0.0
    }

    fn process_chunk(&mut self, input: &[f32], output: &mut [f32]) {
        let source = if self.uses_spectral() {
            self.stretch
                .process(input, &mut self.spectral[..input.len()]);
            &self.spectral[..input.len()]
        } else {
            input
        };

        let high_rc = 1.0 / (TAU * self.effects.high_pass_hz);
        let low_rc = 1.0 / (TAU * self.effects.low_pass_hz);
        let dt = 1.0 / SAMPLE_RATE as f32;
        let high_alpha = high_rc / (high_rc + dt);
        let low_alpha = dt / (low_rc + dt);
        let gain = 10.0_f32.powf(self.effects.gain_db / 20.0);
        let delay_len = (self.effects.echo_delay_ms * SAMPLE_RATE as f32 / 1000.0) as usize;
        let delay_len = delay_len.clamp(1, self.delay.len() - 1);

        for (out, &sample) in output.iter_mut().zip(source) {
            let hp = high_alpha * (self.highpass_y + sample - self.highpass_x);
            self.highpass_x = sample;
            self.highpass_y = hp;
            self.lowpass_y += low_alpha * (hp - self.lowpass_y);
            let mut value = self.lowpass_y;
            if self.effects.body_db != 0.0 {
                value = self.body_eq.process(value);
            }
            if self.effects.presence_db != 0.0 {
                value = self.presence_eq.process(value);
            }

            if self.effects.drive > 0.0 {
                // Keep the small-signal gain at unity. Normalising by tanh(factor)
                // instead amplified quiet microphone noise and clipped speech.
                let factor = 1.0 + self.effects.drive * 4.0;
                value = (value * factor).tanh() / factor;
            }
            let magnitude = value.abs();
            let envelope_rate = if magnitude > self.envelope {
                0.02 // about 1 ms attack at 48 kHz
            } else {
                0.0004 // about 52 ms release
            };
            self.envelope += envelope_rate * (magnitude - self.envelope);
            if self.effects.compression > 0.0 && self.envelope > 0.18 {
                let ratio = 1.0 + self.effects.compression * 7.0;
                let target = 0.18 + (self.envelope - 0.18) / ratio;
                value *= target / self.envelope;
            }

            if self.effects.breath > 0.0 {
                self.noise_state ^= self.noise_state << 13;
                self.noise_state ^= self.noise_state >> 17;
                self.noise_state ^= self.noise_state << 5;
                let noise = (self.noise_state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                value += noise * self.envelope * self.effects.breath * 0.18;
            }

            if self.effects.modulation_mix > 0.0 {
                let carrier = self.modulation_phase.sin();
                value *=
                    (1.0 - self.effects.modulation_mix) + carrier * self.effects.modulation_mix;
                self.modulation_phase += TAU * self.effects.modulation_hz / SAMPLE_RATE as f32;
                if self.modulation_phase >= TAU {
                    self.modulation_phase -= TAU;
                }
            }

            let read = (self.delay_cursor + self.delay.len() - delay_len) % self.delay.len();
            let echo = self.delay[read];
            self.delay[self.delay_cursor] = value + echo * 0.25;
            self.delay_cursor = (self.delay_cursor + 1) % self.delay.len();
            value += echo * self.effects.echo_mix;

            *out = (value * gain).clamp(-1.0, 1.0);
        }
    }
}

impl VoiceProcessor for EffectChain {
    fn process(&mut self, input: &[f32], output: &mut [f32]) {
        assert_eq!(input.len(), output.len());
        for (src, dst) in input.chunks(MAX_BLOCK).zip(output.chunks_mut(MAX_BLOCK)) {
            self.process_chunk(src, dst);
        }
    }

    fn latency_frames(&self) -> usize {
        if self.uses_spectral() {
            self.stretch.input_latency() + self.stretch.output_latency()
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_chain_stays_silent_on_silence() {
        let mut chain = EffectChain::new(Effects::default());
        let mut output = [1.0; 256];
        chain.process(&[0.0; 256], &mut output);
        assert!(output.iter().all(|value| *value == 0.0));
        assert_eq!(chain.latency_frames(), 0);
    }

    #[test]
    fn active_chain_stays_finite_and_bounded() {
        let effects = Effects {
            pitch_semitones: -5.0,
            formant_semitones: -3.0,
            drive: 0.4,
            echo_mix: 0.2,
            ..Effects::default()
        };
        let mut chain = EffectChain::new(effects);
        let input: Vec<f32> = (0..4096)
            .map(|n| ((n as f32) * TAU * 180.0 / SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        let mut output = vec![0.0; input.len()];
        chain.process(&input, &mut output);
        assert!(
            output
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 1.0)
        );
        assert!(chain.latency_frames() > 0);
    }

    #[test]
    fn pitch_shift_moves_a_tone_down_one_octave() {
        let effects = Effects {
            pitch_semitones: -12.0,
            ..Effects::default()
        };
        let mut chain = EffectChain::new(effects);
        let mut output = vec![0.0; SAMPLE_RATE as usize];
        for (block_index, block) in output.chunks_mut(256).enumerate() {
            let input: Vec<f32> = (0..block.len())
                .map(|offset| {
                    let frame = block_index * 256 + offset;
                    (frame as f32 * TAU * 220.0 / SAMPLE_RATE as f32).sin() * 0.3
                })
                .collect();
            chain.process(&input, block);
        }

        fn energy_at(samples: &[f32], hz: f32) -> f32 {
            let step = TAU * hz / SAMPLE_RATE as f32;
            let (real, imag) = samples.iter().enumerate().fold((0.0, 0.0), |sum, (i, x)| {
                let phase = i as f32 * step;
                (sum.0 + x * phase.cos(), sum.1 + x * phase.sin())
            });
            real * real + imag * imag
        }

        let steady = &output[24_000..28_800];
        let peak = (80_i32..=240)
            .max_by(|a, b| energy_at(steady, *a as f32).total_cmp(&energy_at(steady, *b as f32)))
            .unwrap();
        assert!((peak - 110).abs() <= 2, "unexpected peak: {peak} Hz");
    }

    #[test]
    fn drive_does_not_amplify_quiet_input() {
        let mut clean = EffectChain::new(Effects::default());
        let mut driven = EffectChain::new(Effects {
            drive: 1.0,
            ..Effects::default()
        });
        let input: Vec<f32> = (0..SAMPLE_RATE as usize)
            .map(|frame| (frame as f32 * TAU * 250.0 / SAMPLE_RATE as f32).sin() * 0.02)
            .collect();
        let mut clean_output = vec![0.0; input.len()];
        let mut driven_output = vec![0.0; input.len()];
        clean.process(&input, &mut clean_output);
        driven.process(&input, &mut driven_output);
        let rms = |samples: &[f32]| {
            (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
        };
        let ratio = rms(&driven_output[24_000..]) / rms(&clean_output[24_000..]);
        assert!(ratio > 0.95 && ratio <= 1.01, "quiet-signal gain: {ratio}");
    }

    #[test]
    fn peaking_eq_boosts_its_selected_band() {
        fn steady_rms(hz: f32, eq_gain_db: f32) -> f32 {
            let mut eq = PeakingEq::new(1_600.0, eq_gain_db, 1.1);
            let mut sum = 0.0;
            for frame in 0..SAMPLE_RATE as usize {
                let input = (frame as f32 * TAU * hz / SAMPLE_RATE as f32).sin() * 0.1;
                let output = eq.process(input);
                if frame >= 24_000 {
                    sum += output * output;
                }
            }
            (sum / 24_000.0).sqrt()
        }
        let mid_gain = steady_rms(1_600.0, 6.0) / steady_rms(1_600.0, 0.0);
        let low_gain = steady_rms(200.0, 6.0) / steady_rms(200.0, 0.0);
        assert!(mid_gain > 1.9, "mid gain: {mid_gain}");
        assert!(low_gain < 1.2, "low gain: {low_gain}");
    }
}
