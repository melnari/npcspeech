import json
import os
import socket
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ui"))
from npcspeech import client


class ClientTests(unittest.TestCase):
    def test_json_line_request_and_response(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "control.sock"
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(str(path))
            listener.listen(1)
            received = []

            def serve():
                connection, _ = listener.accept()
                with connection:
                    received.append(json.loads(connection.recv(4096)))
                    connection.sendall(b'{"ok":true,"devices":[]}\n')

            worker = threading.Thread(target=serve)
            worker.start()
            try:
                with patch.dict(os.environ, {"NPCSPEECH_SOCKET": str(path)}):
                    self.assertEqual(client.send("devices"), {"ok": True, "devices": []})
                self.assertEqual(received, [{"command": "devices"}])
            finally:
                worker.join(timeout=2)
                listener.close()


if __name__ == "__main__":
    unittest.main()
