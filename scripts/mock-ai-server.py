"""Local stand-in for an AI API, used by proxy-test.sh to exercise Friction's
spend metering end to end. Speaks the OpenAI chat format (JSON and streamed)
and the Anthropic messages format (streamed), with fixed token counts."""

import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

OPENAI_USAGE = {"prompt_tokens": 1200, "completion_tokens": 300, "prompt_tokens_details": {"cached_tokens": 200}}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send_json(self, obj):
        body = json.dumps(obj).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def sse(self, events):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        for event, data in events:
            if event:
                self.wfile.write(f"event: {event}\n".encode())
            payload = data if isinstance(data, str) else json.dumps(data)
            self.wfile.write(f"data: {payload}\n\n".encode())
            self.wfile.flush()
            time.sleep(0.03)

    def do_POST(self):
        length = int(self.headers.get("content-length") or 0)
        body = json.loads(self.rfile.read(length) or b"{}")
        model = body.get("model", "mock")
        if self.path.endswith("/chat/completions"):
            if body.get("stream"):
                chunk = lambda text: {"object": "chat.completion.chunk", "model": model,
                                      "choices": [{"index": 0, "delta": {"content": text}}]}
                self.sse([
                    (None, chunk("Hel")),
                    (None, chunk("lo")),
                    (None, {"object": "chat.completion.chunk", "model": model, "choices": [], "usage": OPENAI_USAGE}),
                    (None, "[DONE]"),
                ])
            else:
                self.send_json({"object": "chat.completion", "model": model,
                                "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello"}}],
                                "usage": OPENAI_USAGE})
        elif self.path.endswith("/v1/messages"):
            self.sse([
                ("message_start", {"type": "message_start", "message": {"id": "msg_1", "model": model,
                    "usage": {"input_tokens": 1000, "output_tokens": 1, "cache_read_input_tokens": 0}}}),
                ("content_block_delta", {"type": "content_block_delta", "index": 0,
                    "delta": {"type": "text_delta", "text": "Hello"}}),
                ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                    "usage": {"output_tokens": 250}}),
                ("message_stop", {"type": "message_stop"}),
            ])
        else:
            self.send_response(404)
            self.end_headers()


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
