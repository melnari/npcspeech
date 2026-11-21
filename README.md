# NPCSpeech

NPCSpeech turns a physical microphone into a selectable virtual microphone with live character voice effects. The first implementation targets **Ubuntu 26.04 with PipeWire**. It contains a Rust audio engine and a Python/GTK 4 preset editor. Windows and neural voice engines are extension points, not implemented platforms or effects in this release.

The engine publishes **NPCSpeech Microphone** as a PipeWire audio source. Select it as the microphone in Discord, Teams, Webex, or another application. The engine defaults to the unprocessed microphone. A preset remains selected until you choose another voice or **Original**. The UI shows the selected preset and the **actual output mode** separately. The bypass button switches an active preset to Original with one click and switches back with another; it is not a press-and-hold control. The bypass is temporary; the chosen preset is restored on the next start.

## Features

- Choose a physical input, including laptop and headset microphones exposed by PipeWire.
- Switch between Original and saved voices in the desktop UI.
- Edit, preview, save, duplicate, delete, import, and export versioned JSON presets with names, descriptions, and tags.
- Start with six editable examples: wood elf, orc, goblin, ghost, golem, and dragon.
- Use pitch shifting, formant shifting, two adjustable resonance bands, filters, drive, compression, breath texture, modulation, echo, and output gain. These are **DSP effects**, without speech recognition or a cloud service.
- Bind the `bypass` command to a desktop-wide keyboard shortcut in Ubuntu Settings.

## Build and run on Ubuntu 26.04

Install the development dependencies:

```sh
sudo apt update
sudo apt install rustc cargo build-essential pkg-config clang libclang-dev \
  libpipewire-0.3-dev libspa-0.2-dev python3-gi gir1.2-gtk-4.0
```

Rust 1.85 or newer is required by the 2024 edition. Build the engine from the repository root:

```sh
cargo build --release --locked --manifest-path engine/Cargo.toml
```

Start the desktop application:

```sh
./scripts/npcspeech-ui
```

Press **Start audio**, select your physical microphone, and choose a voice. In your calling application, select **NPCSpeech Microphone** as the input. To use it as the system default input, select that source in Ubuntu Sound settings. Keep NPCSpeech running while using the virtual microphone. The UI starts the built engine and writes its log to `~/.config/npcspeech/engine.log` (or `$XDG_CONFIG_HOME/npcspeech/engine.log`).

The engine can also run without the UI:

```sh
./engine/target/release/npcspeech-engine
```

In a separate terminal, use `./scripts/npcspeechctl status`, `./scripts/npcspeechctl devices`, `./scripts/npcspeechctl select orc`, or `./scripts/npcspeechctl original`. Use `./scripts/npcspeechctl shutdown` to stop the engine.

### Tuning voice quality

Start with a clean physical microphone and compare the same spoken phrase in **Original** and a preset. Pitch changes the fundamental voice register; formants and the two resonance bands change vocal color. Large simultaneous pitch and formant shifts can sound metallic or cartoonish. The retuned Goblin example uses a moderate pitch lift, a small formant lift, a gentle presence boost, and almost no drive. Increase one control at a time in the preset editor and use **Preview on virtual microphone** to audition it before saving. Drive now keeps quiet-signal gain near unity, and compression follows the speech envelope to avoid abrupt changes.

The live DSP chain keeps the speaker's words, articulation, and speaking pace. It can create character coloration, but it cannot guarantee a natural, entirely different creature voice from every source voice. Acting the rhythm and articulation remains part of the result; a future optional voice-conversion engine would be needed for broader identity changes. The example presets are copied into your configuration on first run and are not automatically overwritten if already present, so your edits remain yours.

## System-wide bypass key

In **Ubuntu Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts**, create a shortcut with the command set to the **absolute path** of `scripts/npcspeechctl`, followed by `bypass`. Example:

```text
/absolute/path/to/NPCspeech/scripts/npcspeechctl bypass
```

Choose a key combination that does not conflict with your calling application. One press routes the physical microphone through unchanged; another restores the selected preset. If **Original** is already selected, the output stays original. The UI button offers the same control. The shortcut uses Ubuntu's desktop keyboard settings; NPCSpeech does not install a global key listener.

## Presets and controls

The user library lives in `~/.config/npcspeech/presets/`, or under `$XDG_CONFIG_HOME/npcspeech/presets/`. Bundled examples are copied there on the first UI launch and existing files are preserved. The selection and input choice are stored in `settings.json` in the same config directory. Use **Import JSON** and **Export JSON** in the editor to exchange presets. Imported IDs are renamed when they would overwrite an existing voice.

Pitch and formants affect voice height and perceived size; filters, drive, compression, breath, modulation, and echo add different creature textures. Extreme combinations can sound synthetic. The effect chain changes the *impression* of a heavy or quick voice, but it does not alter word timing or speaking rate. A natural result depends on the source voice, microphone, and moderate settings. Presets cannot guarantee a natural voice for every creature without a more advanced voice model.

## Test

```sh
cargo test --locked --manifest-path engine/Cargo.toml
PYTHONPATH=ui python3 -m unittest discover -s tests -v
python3 -m compileall -q ui
```

CI runs these checks and Rust formatting/lint checks. A live PipeWire smoke test requires a user session and microphone: start the engine, inspect `./scripts/npcspeechctl devices`, select **NPCSpeech Microphone** in an audio application, and compare Original, a preset, and bypass. The automated suite tests DSP invariants, control state, preset storage, and the socket client; it does not claim measured end-to-end latency or perceptual voice quality.

## Architecture and roadmap

See [Development documentation](docs/DEVELOPMENT.md) for the signal path, control protocol, preset schema, real-time considerations, testing, and the Windows and optional neural-engine extension points.

NPCSpeech is licensed under Apache 2.0. Rust dependencies have their own licenses; the pitch/formant component uses the `signalsmith-stretch` crate and its bundled library.
