//! Local control protocol. The audio callback only reads atomics.

use crate::preset::{Effects, Preset};
use crate::ring::AudioRing;
use serde::Serialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const EFFECT_COUNT: usize = 16;

#[derive(Clone, Debug, Serialize)]
pub struct InputDevice {
    pub node_id: u32,
    pub node_name: String,
    pub description: String,
}

pub struct Controls {
    version: AtomicU64,
    values: [AtomicU32; EFFECT_COUNT],
    pub selected: AtomicBool,
    pub bypass: AtomicBool,
    pub latency_frames: AtomicU64,
}

impl Controls {
    pub fn new() -> Self {
        let defaults = Self::to_values(Effects::default());
        Self {
            version: AtomicU64::new(0),
            values: std::array::from_fn(|index| AtomicU32::new(defaults[index].to_bits())),
            selected: AtomicBool::new(false),
            bypass: AtomicBool::new(false),
            latency_frames: AtomicU64::new(0),
        }
    }

    fn to_values(e: Effects) -> [f32; EFFECT_COUNT] {
        [
            e.pitch_semitones,
            e.formant_semitones,
            e.high_pass_hz,
            e.low_pass_hz,
            e.body_hz,
            e.body_db,
            e.presence_hz,
            e.presence_db,
            e.drive,
            e.compression,
            e.breath,
            e.modulation_hz,
            e.modulation_mix,
            e.echo_mix,
            e.echo_delay_ms,
            e.gain_db,
        ]
    }

    fn from_values(v: [f32; EFFECT_COUNT]) -> Effects {
        Effects {
            pitch_semitones: v[0],
            formant_semitones: v[1],
            high_pass_hz: v[2],
            low_pass_hz: v[3],
            body_hz: v[4],
            body_db: v[5],
            presence_hz: v[6],
            presence_db: v[7],
            drive: v[8],
            compression: v[9],
            breath: v[10],
            modulation_hz: v[11],
            modulation_mix: v[12],
            echo_mix: v[13],
            echo_delay_ms: v[14],
            gain_db: v[15],
        }
    }

    pub fn set_effects(&self, effects: Effects) {
        self.version.fetch_add(1, Ordering::AcqRel); // odd: update in progress
        for (slot, value) in self.values.iter().zip(Self::to_values(effects)) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        self.version.fetch_add(1, Ordering::Release);
    }

    pub fn snapshot_if_changed(&self, previous_version: u64) -> Option<(u64, Effects)> {
        let first = self.version.load(Ordering::Acquire);
        if first == previous_version || !first.is_multiple_of(2) {
            return None;
        }
        let values =
            std::array::from_fn(|i| f32::from_bits(self.values[i].load(Ordering::Relaxed)));
        let last = self.version.load(Ordering::Acquire);
        if first == last {
            Some((last, Self::from_values(values)))
        } else {
            None
        }
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Shared {
    pub controls: Arc<Controls>,
    pub ring: Arc<AudioRing>,
    pub devices: Mutex<Vec<InputDevice>>,
    pub input: Mutex<Option<String>>,
    pub selected_id: Mutex<Option<String>>,
    pub output_active: AtomicBool,
    pub shutdown: AtomicBool,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            controls: Arc::new(Controls::new()),
            ring: Arc::new(AudioRing::new(16_384)),
            devices: Mutex::new(Vec::new()),
            input: Mutex::new(None),
            selected_id: Mutex::new(None),
            output_active: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
        }
    }

    pub fn execute(&self, request: Value) -> Value {
        match request.get("command").and_then(Value::as_str) {
            Some("status") => json!({
                "ok": true,
                "selected_id": *self.selected_id.lock().unwrap(),
                "bypass": self.controls.bypass.load(Ordering::Acquire),
                "input": *self.input.lock().unwrap(),
                "latency_frames": if self.controls.selected.load(Ordering::Acquire)
                    && !self.controls.bypass.load(Ordering::Acquire) {
                    self.controls.latency_frames.load(Ordering::Relaxed)
                } else {
                    0
                },
                "overruns": self.ring.overruns(),
                "underruns": self.ring.underruns(),
            }),
            Some("devices") => json!({"ok": true, "devices": *self.devices.lock().unwrap()}),
            Some("select") => match request.get("preset") {
                Some(Value::Null) => {
                    self.controls.selected.store(false, Ordering::Release);
                    self.controls.bypass.store(false, Ordering::Release);
                    *self.selected_id.lock().unwrap() = None;
                    json!({"ok": true})
                }
                Some(value) => match serde_json::from_value::<Preset>(value.clone()) {
                    Ok(preset) => match preset.validate() {
                        Ok(()) => {
                            self.controls.set_effects(preset.effects);
                            *self.selected_id.lock().unwrap() = Some(preset.id);
                            self.controls.bypass.store(false, Ordering::Release);
                            self.controls.selected.store(true, Ordering::Release);
                            json!({"ok": true})
                        }
                        Err(reason) => json!({"ok": false, "error": reason}),
                    },
                    Err(error) => json!({"ok": false, "error": error.to_string()}),
                },
                None => json!({"ok": false, "error": "missing preset"}),
            },
            Some("toggle_bypass") => {
                let previous = self.controls.bypass.fetch_xor(true, Ordering::AcqRel);
                json!({"ok": true, "bypass": !previous})
            }
            Some("set_input") => {
                let Some(target) = request.get("node_name").and_then(Value::as_str) else {
                    return json!({"ok": false, "error": "missing node_name"});
                };
                let devices = self.devices.lock().unwrap();
                if !devices.iter().any(|device| device.node_name == target) {
                    return json!({"ok": false, "error": "input device is not available"});
                }
                *self.input.lock().unwrap() = Some(target.to_owned());
                json!({"ok": true})
            }
            Some("shutdown") => {
                self.shutdown.store(true, Ordering::Release);
                json!({"ok": true})
            }
            _ => json!({"ok": false, "error": "unknown command"}),
        }
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("NPCSPEECH_SOCKET") {
        return PathBuf::from(path);
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime).join("npcspeech.sock");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    PathBuf::from(format!("/tmp/npcspeech-{user}.sock"))
}

fn handle_client(mut stream: UnixStream, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut line = String::new();
    let response = match BufReader::new(&stream).read_line(&mut line) {
        Ok(0) => json!({"ok": false, "error": "empty request"}),
        Ok(_) if line.len() <= 64_000 => match serde_json::from_str::<Value>(&line) {
            Ok(request) => shared.execute(request),
            Err(error) => json!({"ok": false, "error": error.to_string()}),
        },
        Ok(_) => json!({"ok": false, "error": "request too large"}),
        Err(error) => json!({"ok": false, "error": error.to_string()}),
    };
    let _ = writeln!(stream, "{response}");
}

pub fn start_server(shared: Arc<Shared>) -> std::io::Result<PathBuf> {
    let path = socket_path();
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "NPCSpeech is already running",
            ));
        }
        fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => handle_client(stream, &shared),
                Err(_) if shared.shutdown.load(Ordering::Acquire) => break,
                Err(_) => continue,
            }
        }
    });
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_and_bypass_preserve_selected_voice() {
        let shared = Shared::new();
        let preset = Preset {
            schema_version: 1,
            id: "elf".into(),
            name: "Elf".into(),
            description: String::new(),
            tags: vec![],
            effects: Effects::default(),
        };
        assert_eq!(
            shared.execute(json!({"command":"select","preset":preset}))["ok"],
            true
        );
        assert_eq!(
            shared.execute(json!({"command":"toggle_bypass"}))["bypass"],
            true
        );
        assert_eq!(
            shared.execute(json!({"command":"status"}))["selected_id"],
            "elf"
        );
        assert_eq!(
            shared.execute(json!({"command":"toggle_bypass"}))["bypass"],
            false
        );
    }

    #[test]
    fn selected_resonance_reaches_audio_controls() {
        let shared = Shared::new();
        let effects = Effects {
            body_hz: 300.0,
            body_db: -2.0,
            presence_hz: 1_600.0,
            presence_db: 2.0,
            ..Effects::default()
        };
        let preset = Preset {
            schema_version: 1,
            id: "goblin".into(),
            name: "Goblin".into(),
            description: String::new(),
            tags: vec![],
            effects,
        };
        assert_eq!(
            shared.execute(json!({"command":"select","preset":preset}))["ok"],
            true
        );
        let (_, received) = shared.controls.snapshot_if_changed(0).unwrap();
        assert_eq!(received, effects);
    }
}
