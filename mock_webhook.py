#!/usr/bin/env python3
"""Mock webhook：模拟业务服务，接收本服务转发的 SAP 调用。

用法：python mock_webhook.py
监听 9000 端口，处理 POST /ping：
  收到 {"func":"Z_REST_PING","inputs":{"INPUT":"hello"}}
  返回 {"outputs":{"OUTPUT":"processed by webhook: hello"}}

用标准库 http.server，无需安装 Flask。
能看到每次 SAP 调用的完整请求体，便于调试。
"""
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from datetime import datetime


class WebhookHandler(BaseHTTPRequestHandler):
    def _send_json(self, code, body):
        data = json.dumps(body).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b""
        try:
            req = json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            self._send_json(400, {"error": "invalid json"})
            return

        func = req.get("func", "?")
        inputs = req.get("inputs", {})
        print(f"[{datetime.now().strftime('%H:%M:%S')}] 收到调用: func={func} inputs={inputs}")

        # 路由：根据 path 和 func 返回不同结果
        if self.path == "/ping" or func == "Z_REST_PING":
            inp = inputs.get("INPUT", "")
            out = f"processed by webhook @ {datetime.now().strftime('%H:%M:%S')}: {inp}"
            self._send_json(200, {"outputs": {"OUTPUT": out}})
        elif self.path == "/echo" or func == "Z_REST_ECHO":
            inp = inputs.get("TEXT_IN", "")
            self._send_json(200, {"outputs": {"TEXT_OUT": f"echo: {inp}"}})
        else:
            # 默认：把输入原样回填到同名的 OUTPUT 字段
            outputs = {k.replace("IN", "OUT"): v for k, v in inputs.items()}
            if not outputs:
                outputs = {"OUTPUT": "ok"}
            self._send_json(200, {"outputs": outputs})

    def do_GET(self):
        # 简单健康检查
        self._send_json(200, {"status": "webhook running"})

    def log_message(self, fmt, *args):
        # 静默默认访问日志（我们用自己的 print）
        pass


if __name__ == "__main__":
    port = 9000
    print(f"Mock webhook 启动，监听 http://localhost:{port}")
    print("  POST /ping   - 处理 Z_REST_PING")
    print("  POST /echo   - 处理 Z_REST_ECHO")
    print("等待 SAP 调用（通过本服务转发）...\n")
    HTTPServer(("0.0.0.0", port), WebhookHandler).serve_forever()
