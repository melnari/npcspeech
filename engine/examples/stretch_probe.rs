//! Compare pitch-shifter window sizes on a known 220 Hz tone.
use signalsmith_stretch::Stretch;
use std::f32::consts::TAU;

const RATE: f32 = 48_000.0;

fn energy_at(samples: &[f32], hz: f32) -> f32 {
    let step = TAU * hz / RATE;
    let (real, imag) = samples.iter().enumerate().fold((0.0, 0.0), |sum, (i, x)| {
        let phase = i as f32 * step;
        (sum.0 + x * phase.cos(), sum.1 + x * phase.sin())
    });
    real * real + imag * imag
}

fn main() {
    for (block, interval) in [
        (512, 128),
        (1024, 256),
        (1280, 320),
        (1536, 384),
        (1792, 448),
        (2048, 512),
        (3072, 768),
    ] {
        let mut stretch = Stretch::new(1, block, interval);
        stretch.signalsmith_stretch_set_formant_base(0.0);
        stretch.set_transpose_factor_semitones(-12.0, Some(8000.0 / RATE));
        stretch.set_formant_factor_semitones(0.0, true);
        let latency = stretch.input_latency() + stretch.output_latency();
        let mut output = vec![0.0; 48_000];
        for (block_index, frames) in output.chunks_mut(256).enumerate() {
            let input: Vec<f32> = (0..frames.len())
                .map(|offset| {
                    let i = block_index * 256 + offset;
                    (i as f32 * TAU * 220.0 / RATE).sin() * 0.3
                })
                .collect();
            stretch.process(&input, frames);
        }
        let steady = &output[24_000..28_800];
        let peak = (80..=240)
            .max_by(|a, b| energy_at(steady, *a as f32).total_cmp(&energy_at(steady, *b as f32)))
            .unwrap();
        let rms = (steady.iter().map(|x| x * x).sum::<f32>() / steady.len() as f32).sqrt();
        println!("block={block} interval={interval} latency={latency} peak={peak}Hz rms={rms:.3}");
    }
}
