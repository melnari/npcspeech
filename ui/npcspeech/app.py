"""GTK desktop UI for NPCSpeech on Ubuntu."""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, GLib, Gtk

from . import client, presets, settings, state


ROOT = Path(__file__).resolve().parents[2]
EFFECT_LABELS = {
    "pitch_semitones": "Pitch (semitones)",
    "formant_semitones": "Formants (semitones)",
    "high_pass_hz": "High-pass (Hz)",
    "low_pass_hz": "Low-pass (Hz)",
    "body_hz": "Body resonance (Hz)",
    "body_db": "Body resonance (dB)",
    "presence_hz": "Presence resonance (Hz)",
    "presence_db": "Presence resonance (dB)",
    "drive": "Drive",
    "compression": "Compression",
    "breath": "Breath",
    "modulation_hz": "Modulation frequency (Hz)",
    "modulation_mix": "Modulation mix",
    "echo_mix": "Echo mix",
    "echo_delay_ms": "Echo delay (ms)",
    "gain_db": "Output gain (dB)",
}
DEFAULT_EFFECTS = {
    "pitch_semitones": 0.0, "formant_semitones": 0.0,
    "high_pass_hz": 70.0, "low_pass_hz": 12000.0,
    "body_hz": 220.0, "body_db": 0.0,
    "presence_hz": 2500.0, "presence_db": 0.0,
    "drive": 0.0, "compression": 0.0, "breath": 0.0,
    "modulation_hz": 30.0, "modulation_mix": 0.0,
    "echo_mix": 0.0, "echo_delay_ms": 90.0, "gain_db": 0.0,
}


class NPCSpeechWindow(Gtk.ApplicationWindow):
    def __init__(self, application: Gtk.Application):
        super().__init__(application=application, title="NPCSpeech")
        self.set_default_size(980, 720)
        self.presets: list[dict] = []
        self.devices: list[dict] = []
        self.editing_id: str | None = None
        self.loading = False
        self.current_settings = settings.load()
        self.saved_input_was_available = False
        presets.install_examples(ROOT / "presets")

        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        outer.set_margin_top(16)
        outer.set_margin_bottom(16)
        outer.set_margin_start(16)
        outer.set_margin_end(16)
        self.set_child(outer)

        header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        outer.append(header)
        title = Gtk.Label(label="NPCSpeech")
        title.add_css_class("title-1")
        header.append(title)
        self.status = Gtk.Label(label="Audio engine is not running")
        self.status.set_hexpand(True)
        self.status.set_halign(Gtk.Align.END)
        header.append(self.status)
        start = Gtk.Button(label="Start audio")
        start.connect("clicked", self.start_engine)
        header.append(start)

        columns = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=18)
        columns.set_vexpand(True)
        outer.append(columns)
        sidebar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        sidebar.set_size_request(250, -1)
        columns.append(sidebar)
        editor_scroll = Gtk.ScrolledWindow()
        editor_scroll.set_hexpand(True)
        editor_scroll.set_vexpand(True)
        columns.append(editor_scroll)
        editor = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        editor.set_margin_end(8)
        editor_scroll.set_child(editor)

        sidebar.append(self._section_label("Microphone input"))
        self.input_dropdown = Gtk.DropDown.new_from_strings(["Loading devices…"])
        self.input_dropdown.connect("notify::selected", self.on_input_selected)
        sidebar.append(self.input_dropdown)
        refresh = Gtk.Button(label="Refresh devices")
        refresh.connect("clicked", lambda *_: self.refresh_engine())
        sidebar.append(refresh)

        sidebar.append(self._section_label("Selected voice preset"))
        self.voice_dropdown = Gtk.DropDown.new_from_strings(["Original"])
        self.voice_dropdown.connect("notify::selected", self.on_voice_selected)
        sidebar.append(self.voice_dropdown)
        self.active_output = Gtk.Label(label="Output: checking audio engine…", xalign=0)
        self.active_output.set_wrap(True)
        self.active_output.add_css_class("title-3")
        sidebar.append(self.active_output)
        self.output_detail = Gtk.Label(label="", xalign=0)
        self.output_detail.set_wrap(True)
        sidebar.append(self.output_detail)
        recorder_hint = Gtk.Label(
            label="Your recording app must use NPCSpeech Microphone. In Audacity with ALSA, choose 'default' or 'pulse' as Recording Device; 'sysdefault' can use a physical microphone directly.",
            xalign=0,
        )
        recorder_hint.set_wrap(True)
        recorder_hint.add_css_class("dim-label")
        sidebar.append(recorder_hint)
        self.bypass_button = Gtk.Button(label="Use original microphone (bypass)")
        self.bypass_button.set_sensitive(False)
        self.bypass_button.connect("clicked", self.toggle_bypass)
        sidebar.append(self.bypass_button)
        hint = Gtk.Label(label="Bypass toggles with one click or keypress. You do not need to hold a key. For a system-wide key, bind the 'bypass' command in Ubuntu keyboard settings.")
        hint.set_wrap(True)
        hint.set_xalign(0)
        hint.add_css_class("dim-label")
        sidebar.append(hint)

        sidebar.append(self._section_label("Preset library"))
        for label, handler in [
            ("New voice", self.new_preset), ("Duplicate voice", self.duplicate_preset),
            ("Import JSON", self.import_preset), ("Export JSON", self.export_preset),
            ("Delete voice", self.delete_preset),
        ]:
            button = Gtk.Button(label=label)
            button.connect("clicked", handler)
            sidebar.append(button)

        editor.append(self._section_label("Preset editor"))
        editor.append(Gtk.Label(label="Short name", xalign=0))
        self.name_entry = Gtk.Entry()
        editor.append(self.name_entry)
        editor.append(Gtk.Label(label="Description", xalign=0))
        self.description = Gtk.TextView()
        self.description.set_wrap_mode(Gtk.WrapMode.WORD)
        self.description.set_size_request(-1, 70)
        editor.append(self.description)
        editor.append(Gtk.Label(label="Tags (comma separated)", xalign=0))
        self.tags_entry = Gtk.Entry()
        editor.append(self.tags_entry)

        grid = Gtk.Grid(column_spacing=16, row_spacing=8)
        editor.append(grid)
        self.scales: dict[str, Gtk.Scale] = {}
        for row, (key, (minimum, maximum)) in enumerate(presets.EFFECT_LIMITS.items()):
            label = Gtk.Label(label=EFFECT_LABELS[key], xalign=0)
            grid.attach(label, 0, row, 1, 1)
            step = 1.0 if maximum > 100 else 0.05
            scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, minimum, maximum, step)
            scale.set_hexpand(True)
            scale.set_digits(2 if step < 1 else 0)
            grid.attach(scale, 1, row, 1, 1)
            self.scales[key] = scale

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        editor.append(actions)
        save_button = Gtk.Button(label="Save preset")
        save_button.add_css_class("suggested-action")
        save_button.connect("clicked", self.save_preset)
        actions.append(save_button)
        preview_button = Gtk.Button(label="Preview on virtual microphone")
        preview_button.connect("clicked", self.preview_preset)
        actions.append(preview_button)
        self.message = Gtk.Label(label="", xalign=0)
        self.message.set_wrap(True)
        editor.append(self.message)

        self.reload_presets(select_id=self.current_settings.get("selected_id"))
        GLib.timeout_add_seconds(2, self.poll_engine)
        GLib.idle_add(self.initialize_engine)

    @staticmethod
    def _section_label(text: str) -> Gtk.Label:
        label = Gtk.Label(label=text, xalign=0)
        label.add_css_class("title-3")
        return label

    def notify(self, text: str) -> None:
        self.message.set_text(text)

    def reload_presets(self, select_id: str | None = None) -> None:
        try:
            self.presets = presets.load_all()
        except ValueError as error:
            self.notify(str(error))
            self.presets = []
        labels = ["Original"] + [preset["name"] for preset in self.presets]
        self.loading = True
        self.voice_dropdown.set_model(Gtk.StringList.new(labels))
        selected = next((i + 1 for i, preset in enumerate(self.presets) if preset["id"] == select_id), 0)
        self.voice_dropdown.set_selected(selected)
        self.loading = False
        if selected > 0:
            self.load_editor(self.presets[selected - 1])
        else:
            self.new_preset()

    def load_editor(self, preset: dict) -> None:
        self.editing_id = preset["id"]
        self.name_entry.set_text(preset["name"])
        self.description.get_buffer().set_text(preset["description"])
        self.tags_entry.set_text(", ".join(preset["tags"]))
        for key, scale in self.scales.items():
            scale.set_value(preset["effects"][key])

    def editor_preset(self) -> dict:
        buffer = self.description.get_buffer()
        description = buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), False)
        name = self.name_entry.get_text().strip()
        preset = {
            "schema_version": 1,
            "id": self.editing_id or presets.slugify(name),
            "name": name,
            "description": description,
            "tags": [tag.strip() for tag in self.tags_entry.get_text().split(",") if tag.strip()],
            "effects": {key: float(scale.get_value()) for key, scale in self.scales.items()},
        }
        return presets.validate(preset)

    def new_preset(self, *_args) -> None:
        self.editing_id = None
        self.name_entry.set_text("New Voice")
        self.description.get_buffer().set_text("")
        self.tags_entry.set_text("")
        for key, value in DEFAULT_EFFECTS.items():
            self.scales[key].set_value(value)

    def duplicate_preset(self, *_args) -> None:
        index = self.voice_dropdown.get_selected()
        if index == 0:
            self.notify("Select a saved preset to duplicate.")
            return
        source = self.presets[index - 1]
        self.load_editor(source)
        self.editing_id = None
        self.name_entry.set_text(f"{source['name']} Copy")

    def save_preset(self, *_args) -> None:
        try:
            preset = self.editor_preset()
            if self.editing_id is None:
                existing = {item["id"] for item in self.presets}
                base = preset["id"][:58]
                count = 2
                while preset["id"] in existing:
                    preset["id"] = f"{base}-{count}"
                    count += 1
            presets.save(preset)
            self.reload_presets(select_id=preset["id"])
            self.current_settings["selected_id"] = preset["id"]
            settings.save(self.current_settings)
            try:
                client.send("select", preset=preset)
                self.refresh_engine()
                self.notify(f"Saved and selected {preset['name']}.")
            except (OSError, RuntimeError):
                self.notify(f"Saved {preset['name']}. Start audio to use it.")
        except (ValueError, OSError, RuntimeError, ConnectionError) as error:
            self.notify(str(error))

    def preview_preset(self, *_args) -> None:
        try:
            preset = self.editor_preset()
            client.send("select", preset=preset)
            self.refresh_engine()
            self.notify("Preview is active on the NPCSpeech microphone. Save to keep these changes.")
        except (ValueError, OSError, RuntimeError, ConnectionError) as error:
            self.notify(str(error))

    def delete_preset(self, *_args) -> None:
        index = self.voice_dropdown.get_selected()
        if index == 0:
            self.notify("Select a saved preset to delete.")
            return
        preset = self.presets[index - 1]
        try:
            (presets.config_dir() / "presets" / f"{preset['id']}.json").unlink()
            try:
                client.send("select", preset=None)
            except (OSError, RuntimeError):
                pass
            self.current_settings["selected_id"] = None
            settings.save(self.current_settings)
            self.reload_presets()
            self.notify(f"Deleted {preset['name']}.")
        except (OSError, RuntimeError) as error:
            self.notify(str(error))

    def on_voice_selected(self, *_args) -> None:
        if self.loading:
            return
        index = self.voice_dropdown.get_selected()
        preset = self.presets[index - 1] if index > 0 else None
        if preset:
            self.load_editor(preset)
        self.current_settings["selected_id"] = preset["id"] if preset else None
        try:
            settings.save(self.current_settings)
            client.send("select", preset=preset)
            self.refresh_engine()
            self.notify(f"Active voice: {preset['name'] if preset else 'Original'}")
        except (OSError, RuntimeError):
            self.notify(f"Selected {preset['name'] if preset else 'Original'}. Start audio to use it.")
        except ValueError as error:
            self.notify(str(error))

    def on_input_selected(self, *_args) -> None:
        if self.loading:
            return
        index = self.input_dropdown.get_selected()
        if index >= len(self.devices):
            return
        target = self.devices[index]["node_name"]
        try:
            client.send("set_input", node_name=target)
            self.current_settings["input"] = target
            settings.save(self.current_settings)
            self.notify(f"Microphone: {self.devices[index]['description']}")
        except (OSError, RuntimeError) as error:
            self.notify(str(error))

    def toggle_bypass(self, *_args) -> None:
        try:
            current = client.send("status")
            if current.get("selected_id") is None:
                self.notify("Choose a voice preset before using bypass.")
                return
            client.send("toggle_bypass")
            self.refresh_engine()
        except (OSError, RuntimeError) as error:
            self.notify(str(error))

    def refresh_engine(self) -> bool:
        try:
            status = client.send("status")
            response = client.send("devices")
        except (OSError, RuntimeError):
            self.status.set_text("Audio engine is not running")
            self.active_output.set_text("Output: unavailable")
            self.output_detail.set_text("Start audio to activate the virtual microphone.")
            self.bypass_button.set_sensitive(False)
            return False
        new_devices = response["devices"]
        saved_input = self.current_settings.get("input")
        saved_available = bool(saved_input) and any(
            device["node_name"] == saved_input for device in new_devices
        )
        if saved_available and not self.saved_input_was_available and status.get("input") != saved_input:
            try:
                client.send("set_input", node_name=saved_input)
                status = client.send("status")
            except (OSError, RuntimeError):
                pass
        self.saved_input_was_available = saved_available
        self.status.set_text(
            "Preferred microphone disconnected" if saved_input and not saved_available
            else "Virtual microphone ready"
        )
        self.loading = True
        if new_devices != self.devices:
            self.devices = new_devices
            self.input_dropdown.set_model(Gtk.StringList.new([item["description"] for item in self.devices] or ["No input devices found"]))
        input_name = status.get("input")
        index = next((i for i, item in enumerate(self.devices) if item["node_name"] == input_name), 0)
        self.input_dropdown.set_selected(index)
        selected_id = status.get("selected_id")
        selected_index = next((i + 1 for i, item in enumerate(self.presets) if item["id"] == selected_id), 0 if selected_id is None else None)
        if selected_index is not None:
            self.voice_dropdown.set_selected(selected_index)
        self.loading = False
        view = state.route_view(status, self.presets)
        self.active_output.set_text(view["headline"])
        self.output_detail.set_text(view["detail"])
        self.bypass_button.set_label(view["button"])
        self.bypass_button.set_sensitive(view["can_toggle"])
        return False

    def poll_engine(self) -> bool:
        self.refresh_engine()
        return True

    def initialize_engine(self) -> bool:
        try:
            client.send("status")
        except (OSError, RuntimeError):
            self.refresh_engine()
        else:
            self.refresh_engine()
        return False

    def start_engine(self, *_args) -> None:
        try:
            client.send("status")
            self.refresh_engine()
            return
        except (OSError, RuntimeError):
            pass
        binary = os.environ.get("NPCSPEECH_ENGINE") or shutil.which("npcspeech-engine")
        if binary is None:
            binary = str(ROOT / "engine" / "target" / "release" / "npcspeech-engine")
        if not Path(binary).is_file():
            self.notify("Build the engine first: cargo build --release --manifest-path engine/Cargo.toml")
            return
        log_path = presets.config_dir() / "engine.log"
        log_path.parent.mkdir(parents=True, exist_ok=True)
        with log_path.open("a", encoding="utf-8") as log:
            subprocess.Popen([binary], stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        self.restore_attempts = 0
        GLib.timeout_add(400, self.restore_engine)

    def restore_engine(self) -> bool:
        try:
            client.send("status")
        except (OSError, RuntimeError):
            self.restore_attempts = getattr(self, "restore_attempts", 0) + 1
            if self.restore_attempts >= 25:
                self.notify("Audio engine did not start. See the engine log in the NPCSpeech config directory.")
                return False
            return True
        try:
            saved_input = self.current_settings.get("input")
            available = client.send("devices")["devices"]
            if saved_input and any(device["node_name"] == saved_input for device in available):
                client.send("set_input", node_name=saved_input)
            selected_id = self.current_settings.get("selected_id")
            selected = next((p for p in self.presets if p["id"] == selected_id), None)
            client.send("select", preset=selected)
            self.refresh_engine()
            return False
        except (OSError, RuntimeError) as error:
            self.notify(str(error))
            return False

    def import_preset(self, *_args) -> None:
        dialog = Gtk.FileDialog(title="Import NPCSpeech preset")
        dialog.open(self, None, self._finish_import)

    def _finish_import(self, dialog: Gtk.FileDialog, result) -> None:
        try:
            source = dialog.open_finish(result)
            if source:
                path = presets.import_file(Path(source.get_path()))
                self.reload_presets(select_id=path.stem)
                self.notify(f"Imported {path.name}.")
        except GLib.Error:
            pass
        except (OSError, ValueError) as error:
            self.notify(str(error))

    def export_preset(self, *_args) -> None:
        index = self.voice_dropdown.get_selected()
        if index == 0:
            self.notify("Select a saved preset to export.")
            return
        self._export_target = self.presets[index - 1]
        dialog = Gtk.FileDialog(title="Export NPCSpeech preset")
        dialog.set_initial_name(f"{self._export_target['id']}.json")
        dialog.save(self, None, self._finish_export)

    def _finish_export(self, dialog: Gtk.FileDialog, result) -> None:
        try:
            target = dialog.save_finish(result)
            if target:
                presets.export_file(self._export_target, Path(target.get_path()))
                self.notify(f"Exported {target.get_basename()}.")
        except GLib.Error:
            pass
        except (OSError, ValueError) as error:
            self.notify(str(error))


class NPCSpeechApp(Gtk.Application):
    def __init__(self):
        super().__init__(application_id="org.npcspeech.NPCSpeech", flags=Gio.ApplicationFlags.DEFAULT_FLAGS)

    def do_activate(self):
        window = self.get_active_window()
        if window is None:
            window = NPCSpeechWindow(self)
        window.present()


def main() -> int:
    return NPCSpeechApp().run(None)


if __name__ == "__main__":
    raise SystemExit(main())

