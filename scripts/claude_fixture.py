"""Owned loopback Anthropic-protocol fixture; no cloud model or real credentials."""
import http.server
import json
import socketserver
import threading


class ModelFixture(http.server.ThreadingHTTPServer):
    def __init__(self, target, tool='Edit', old='before confirmation', new='after confirmation'):
        self.target, self.tool, self.old, self.new = target, tool, old, new
        self.requests = 0
        self.offered_tools = set()
        super().__init__(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name = '127.0.0.1'
        self.server_port = self.server_address[1]

    def start(self):
        self.thread.start()
        return self

    def stop(self):
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=5)


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Length', '2')
        self.end_headers()
        self.wfile.write(b'{}')

    def do_POST(self):
        length = int(self.headers.get('Content-Length', 0))
        if not 0 <= length < 1048576:
            self.send_error(400)
            return
        body = json.loads(self.rfile.read(length))
        model = self.server
        model.offered_tools.update(tool['name'] for tool in body.get('tools', []) if isinstance(tool.get('name'), str))
        if self.path.split('?')[0] != '/v1/messages':
            return self.do_GET()
        model.requests += 1
        index = model.requests
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Connection', 'close')
        self.end_headers()

        def send(kind, data):
            self.wfile.write(('event: ' + kind + '\ndata: ' + json.dumps(data) + '\n\n').encode())
            self.wfile.flush()

        send('message_start', {'type': 'message_start', 'message': {'id': 'msg_wk_fixture_' + str(index), 'type': 'message', 'role': 'assistant', 'content': [], 'model': 'claude-sonnet-4-6', 'stop_reason': None, 'stop_sequence': None, 'usage': {'input_tokens': 10, 'output_tokens': 0}}})
        if index <= 2:
            name = 'Read' if index == 1 else model.tool
            data = {'file_path': str(model.target)}
            if name == 'Edit':
                data.update(old_string=model.old, new_string=model.new)
            elif name == 'Write':
                data['content'] = model.new + '\n'
            elif name == 'MultiEdit':
                data['edits'] = [{'old_string': model.old, 'new_string': model.new}]
            send('content_block_start', {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'tool_use', 'id': 'tool_wk_' + str(index), 'name': name, 'input': {}}})
            send('content_block_delta', {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'input_json_delta', 'partial_json': json.dumps(data)}})
            stop = 'tool_use'
        else:
            send('content_block_start', {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}})
            send('content_block_delta', {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': 'Synthetic host fixture completed.'}})
            stop = 'end_turn'
        send('content_block_stop', {'type': 'content_block_stop', 'index': 0})
        send('message_delta', {'type': 'message_delta', 'delta': {'stop_reason': stop, 'stop_sequence': None}, 'usage': {'output_tokens': 10}})
        send('message_stop', {'type': 'message_stop'})
