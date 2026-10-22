#!/usr/bin/env python3
"""Minimal CDP driver — stdlib only. Drive a Chrome instance started with
--remote-debugging-port (default 9223).

Commands:
  nav <url>            navigate the page target
  shot <png> [delay]   screenshot after delay seconds (default 1.5)
  eval <js>            Runtime.evaluate with awaitPromise, prints result JSON
  cons <url> [secs]    navigate and dump console/exception/log events for secs
  url                  print the page target ws url
"""
import base64, json, os, socket, struct, sys, urllib.request

PORT = os.environ.get("CDP_PORT", "9223")

def pick(sel=None):
    targets = []
    for host in ("[::1]", "127.0.0.1"):
        try:
            t = json.load(urllib.request.urlopen(f"http://{host}:{PORT}/json"))
            if any(x["type"] == "page" for x in t):
                targets = t
                break
        except Exception:
            pass
    page = next(t for t in targets if t["type"] == "page")
    return page["webSocketDebuggerUrl"]

def connect(url):
    rest = url.split("://", 1)[1]
    hostport, path = rest.split("/", 1)
    if hostport.startswith("["):
        host, port = hostport[1:].split("]:")
    else:
        host, port = hostport.rsplit(":", 1)
    s = socket.create_connection((host, int(port)), timeout=30)
    s.settimeout(60)
    key = base64.b64encode(os.urandom(16)).decode()
    s.sendall(
        f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\n"
        f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode()
    )
    resp = b""
    while b"\r\n\r\n" not in resp:
        resp += s.recv(4096)
    assert b" 101" in resp.split(b"\r\n", 1)[0], resp[:200]
    return s

def _rd(s, n):
    buf = b""
    while len(buf) < n:
        c = s.recv(n - len(buf))
        if not c:
            raise EOFError
        buf += c
    return buf

def send(s, data):
    header = bytearray(b"\x81")
    ln = len(data)
    if ln < 126:
        header.append(0x80 | ln)
    elif ln < 65536:
        header.append(0x80 | 126)
        header += struct.pack("!H", ln)
    else:
        header.append(0x80 | 127)
        header += struct.pack("!Q", ln)
    mask = os.urandom(4)
    header += mask
    s.sendall(bytes(header) + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

def recvmsg(s):
    data = b""
    while True:
        b1, b2 = _rd(s, 2)
        ln = b2 & 0x7F
        if ln == 126:
            ln = struct.unpack("!H", _rd(s, 2))[0]
        elif ln == 127:
            ln = struct.unpack("!Q", _rd(s, 8))[0]
        if b2 & 0x80:
            _rd(s, 4)
        data += _rd(s, ln)
        if b1 & 0x80:
            return data

_id = 0
def call(s, method, params=None):
    global _id
    _id += 1
    send(s, json.dumps({"id": _id, "method": method, "params": params or {}}).encode())
    while True:
        msg = json.loads(recvmsg(s))
        if msg.get("id") == _id:
            return msg.get("result", msg)

def main():
    cmd, sel = sys.argv[1], None
    rest = sys.argv[2:]
    if rest and rest[-2:-1] == ["sel"]:
        sel = rest.pop(); rest.pop()
    s = connect(pick(sel))
    call(s, "Page.enable")
    call(s, "Runtime.enable")
    if cmd == "cons":
        import time
        call(s, "Log.enable")
        call(s, "Page.navigate", {"url": rest[0]})
        s.settimeout(8)
        end = time.time() + float(rest[1] if len(rest) > 1 else 6)
        while time.time() < end:
            try:
                msg = json.loads(recvmsg(s))
            except Exception:
                break
            m = msg.get("method", "")
            if m == "Runtime.consoleAPICalled":
                args = [str(a.get("value", a.get("description", "")))[:200] for a in msg["params"].get("args", [])]
                print(f"[{msg['params'].get('type')}]", " ".join(args)[:300])
            elif m == "Runtime.exceptionThrown":
                d = msg["params"]["exceptionDetails"]
                print("[EXC]", str(d.get("exception", {}).get("description", d.get("text")))[:300])
            elif m == "Log.entryAdded":
                e = msg["params"]["entry"]
                print(f"[{e.get('level')}:{e.get('source')}]", str(e.get("text"))[:300])
        return
    if cmd == "nav":
        call(s, "Page.navigate", {"url": rest[0]})
    elif cmd == "msize":
        call(s, "Emulation.setDeviceMetricsOverride", {
            "width": int(rest[0]), "height": int(rest[1]),
            "deviceScaleFactor": 1, "mobile": True,
        })
    elif cmd == "mclear":
        call(s, "Emulation.clearDeviceMetricsOverride")
    elif cmd == "shot":
        import time
        time.sleep(float(rest[1]) if len(rest) > 1 else 1.5)
        r = call(s, "Page.captureScreenshot", {"format": "png"})
        open(rest[0], "wb").write(base64.b64decode(r["data"]))
    elif cmd == "mshot":
        import time
        call(s, "Emulation.setDeviceMetricsOverride", {
            "width": int(rest[1]), "height": int(rest[2]),
            "deviceScaleFactor": 2, "mobile": True,
        })
        time.sleep(float(rest[3]) if len(rest) > 3 else 1.5)
        r = call(s, "Page.captureScreenshot", {"format": "png"})
        open(rest[0], "wb").write(base64.b64decode(r["data"]))
        call(s, "Emulation.clearDeviceMetricsOverride")
    elif cmd == "fup":
        # fup <css-selector> <abs-path> — set files on a file input
        call(s, "DOM.enable")
        doc = call(s, "DOM.getDocument", {"depth": 1})
        node = call(s, "DOM.querySelector", {"nodeId": doc["root"]["nodeId"], "selector": rest[0]})
        r = call(s, "DOM.setFileInputFiles", {"files": rest[1:], "nodeId": node["nodeId"]})
        print(json.dumps(r)[:2000])
    elif cmd == "call":
        # raw passthrough: call <Method> <jsonParams>
        r = call(s, rest[0], json.loads(rest[1]) if len(rest) > 1 else {})
        print(json.dumps(r)[:4000])
    elif cmd == "eval":
        r = call(s, "Runtime.evaluate", {"expression": rest[0], "awaitPromise": True, "returnByValue": True})
        print(json.dumps(r.get("result", r))[:4000])
    elif cmd == "url":
        print(pick(sel))
    s.close()

main()
