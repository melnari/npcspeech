"""Small control command suitable for desktop-wide keyboard shortcuts."""

from __future__ import annotations

import argparse
import json
import sys

from . import client, presets


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Control the running NPCSpeech engine")
    parser.add_argument("command", choices=["status", "devices", "original", "select", "bypass", "input", "shutdown"])
    parser.add_argument("value", nargs="?", help="Preset ID or PipeWire input node name")
    args = parser.parse_args(argv)
    try:
        if args.command == "original":
            result = client.send("select", preset=None)
        elif args.command == "select":
            if not args.value:
                parser.error("select requires a preset ID")
            selected = next((preset for preset in presets.load_all() if preset["id"] == args.value), None)
            if selected is None:
                parser.error(f"Unknown preset: {args.value}")
            result = client.send("select", preset=selected)
        elif args.command == "bypass":
            result = client.send("toggle_bypass")
        elif args.command == "input":
            if not args.value:
                parser.error("input requires a PipeWire node name")
            result = client.send("set_input", node_name=args.value)
        else:
            result = client.send(args.command)
        print(json.dumps(result, indent=2))
        return 0
    except (ConnectionError, OSError, RuntimeError, ValueError) as error:
        print(f"NPCSpeech: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

