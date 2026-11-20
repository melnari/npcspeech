"""Control client for the NPCSpeech audio process."""

from __future__ import annotations

import json
import os
import socket
from pathlib import Path


def socket_path() -> Path:
    override = os.environ.get("NPCSPEECH_SOCKET")
    if override:
        return Path(override)
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    if runtime:
        return Path(runtime) / "npcspeech.sock"
    return Path(f"/tmp/npcspeech-{os.environ.get('USER', 'unknown')}.sock")


def send(command: str, **arguments) -> dict:
    payload = json.dumps({"command": command, **arguments}, allow_nan=False) + "\n"
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(2)
        connection.connect(str(socket_path()))
        connection.sendall(payload.encode("utf-8"))
        result = b""
        while not result.endswith(b"\n"):
            chunk = connection.recv(65536)
            if not chunk:
                raise ConnectionError("NPCSpeech closed the control connection")
            result += chunk
            if len(result) > 65536:
                raise ValueError("NPCSpeech response is too large")
    response = json.loads(result)
    if not response.get("ok"):
        raise RuntimeError(response.get("error", "NPCSpeech command failed"))
    return response

