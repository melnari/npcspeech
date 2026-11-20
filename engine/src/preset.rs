use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effects {
    pub pitch_semitones: f32,
    pub formant_semitones: f32,
    pub high_pass_hz: f32,
    pub low_pass_hz: f32,
    #[serde(default = "default_body_hz")]
    pub body_hz: f32,
    #[serde(default)]
    pub body_db: f32,
    #[serde(default = "default_presence_hz")]
    pub presence_hz: f32,
    #[serde(default)]
    pub presence_db: f32,
    pub drive: f32,
    pub compression: f32,
    pub breath: f32,
    pub modulation_hz: f32,
    pub modulation_mix: f32,
    pub echo_mix: f32,
    pub echo_delay_ms: f32,
    pub gain_db: f32,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            pitch_semitones: 0.0,
            formant_semitones: 0.0,
            high_pass_hz: 70.0,
            low_pass_hz: 12_000.0,
            body_hz: default_body_hz(),
            body_db: 0.0,
            presence_hz: default_presence_hz(),
            presence_db: 0.0,
            drive: 0.0,
            compression: 0.0,
            breath: 0.0,
            modulation_hz: 30.0,
            modulation_mix: 0.0,
            echo_mix: 0.0,
            echo_delay_ms: 90.0,
            gain_db: 0.0,
        }
    }
}

impl Effects {
    pub fn validate(&self) -> Result<(), &'static str> {
        let checks = [
            (self.pitch_semitones, -12.0, 12.0, "pitch_semitones"),
            (self.formant_semitones, -8.0, 8.0, "formant_semitones"),
            (self.high_pass_hz, 20.0, 800.0, "high_pass_hz"),
            (self.low_pass_hz, 800.0, 20_000.0, "low_pass_hz"),
            (self.body_hz, 80.0, 1_200.0, "body_hz"),
            (self.body_db, -12.0, 12.0, "body_db"),
            (self.presence_hz, 700.0, 8_000.0, "presence_hz"),
            (self.presence_db, -12.0, 12.0, "presence_db"),
            (self.drive, 0.0, 1.0, "drive"),
            (self.compression, 0.0, 1.0, "compression"),
            (self.breath, 0.0, 1.0, "breath"),
            (self.modulation_hz, 1.0, 120.0, "modulation_hz"),
            (self.modulation_mix, 0.0, 1.0, "modulation_mix"),
            (self.echo_mix, 0.0, 0.6, "echo_mix"),
            (self.echo_delay_ms, 20.0, 350.0, "echo_delay_ms"),
            (self.gain_db, -18.0, 18.0, "gain_db"),
        ];
        for (value, min, max, name) in checks {
            if !value.is_finite() || value < min || value > max {
                return Err(name);
            }
        }
        if self.high_pass_hz >= self.low_pass_hz {
            return Err("high_pass_hz must be below low_pass_hz");
        }
        Ok(())
    }
}

fn default_body_hz() -> f32 {
    220.0
}

fn default_presence_hz() -> f32 {
    2_500.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub effects: Effects,
}

impl Preset {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("unsupported schema_version");
        }
        if self.id.is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            || self.id.len() > 64
        {
            return Err("id");
        }
        if self.name.trim().is_empty() || self.name.len() > 100 {
            return Err("name");
        }
        if self.description.len() > 2_000 {
            return Err("description");
        }
        if self.tags.len() > 16
            || self
                .tags
                .iter()
                .any(|tag| tag.trim().is_empty() || tag.len() > 40)
        {
            return Err("tags");
        }
        self.effects.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_effects_are_valid() {
        assert_eq!(Effects::default().validate(), Ok(()));
    }

    #[test]
    fn rejects_nan_and_bad_filter_order() {
        let mut e = Effects {
            pitch_semitones: f32::NAN,
            ..Effects::default()
        };
        assert_eq!(e.validate(), Err("pitch_semitones"));
        e.pitch_semitones = 0.0;
        e.high_pass_hz = e.low_pass_hz;
        assert!(e.validate().is_err());
    }

    #[test]
    fn old_preset_json_uses_neutral_resonance() {
        let old = serde_json::json!({
            "pitch_semitones": 3.0,
            "formant_semitones": 1.0,
            "high_pass_hz": 100.0,
            "low_pass_hz": 10000.0,
            "drive": 0.1,
            "compression": 0.2,
            "breath": 0.0,
            "modulation_hz": 30.0,
            "modulation_mix": 0.0,
            "echo_mix": 0.0,
            "echo_delay_ms": 90.0,
            "gain_db": 0.0
        });
        let effects: Effects = serde_json::from_value(old).unwrap();
        assert_eq!(effects.body_hz, 220.0);
        assert_eq!(effects.body_db, 0.0);
        assert_eq!(effects.presence_hz, 2_500.0);
        assert_eq!(effects.presence_db, 0.0);
        assert!(effects.validate().is_ok());
    }
}
