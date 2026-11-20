"""Human-readable output state derived from the running audio engine."""

from __future__ import annotations


def route_view(status: dict, presets: list[dict]) -> dict:
    """Describe the signal currently sent to the virtual microphone."""
    selected_id = status.get("selected_id")
    if selected_id is None:
        return {
            "headline": "Output: Original microphone",
            "detail": "No preset is selected. Choose a voice above to process the microphone.",
            "button": "Original is active",
            "can_toggle": False,
        }

    preset = next((item for item in presets if item["id"] == selected_id), None)
    name = preset["name"] if preset else f"Preview ({selected_id})"
    if status.get("bypass", False):
        return {
            "headline": "Output: Original microphone — BYPASS ON",
            "detail": f"{name} is selected but its effects are disabled. Click once to enable them.",
            "button": f"Enable {name} effects",
            "can_toggle": True,
        }
    return {
        "headline": f"Output: {name} effects ACTIVE",
        "detail": "The selected voice is being processed. Click once to use the original microphone.",
        "button": "Use original microphone (bypass)",
        "can_toggle": True,
    }
