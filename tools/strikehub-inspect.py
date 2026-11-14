#!/usr/bin/env python3
"""strikehub-inspect: drive a running StrikeHub desktop app (macOS) that was
launched with STRIKEHUB_INSPECTOR=1.

Talks newline-delimited JSON over the app's unix-domain socket
($TMPDIR/strikehub-inspect-<pid>.sock). Stdlib only.

Usage:
  strikehub-inspect.py targets
  strikehub-inspect.py screenshot out.png
  strikehub-inspect.py eval '<js>'
  strikehub-inspect.py click '<css selector>'
  strikehub-inspect.py type '<css selector>' '<text>'
  strikehub-inspect.py wait '<css selector>' [timeout_ms]
  strikehub-inspect.py text '<css selector>'

Socket discovery: --socket PATH, or --pid PID, or the newest
$TMPDIR/strikehub-inspect-*.path file whose process is still alive.
"""

import argparse
import typing
import base64
import glob
import json
import os
import socket
import sys
import time


def live_pid(path_file: str) -> typing.Optional[int]:
    try:
        pid = int(os.path.basename(path_file).split("-")[2].split(".")[0])
    except (IndexError, ValueError):
        return None
    try:
        os.kill(pid, 0)
    except (ProcessLookupError, PermissionError, OSError):
        return None
    return pid


def find_socket(args) -> str:
    if args.socket:
        return args.socket
    tmp = os.environ.get("TMPDIR") or "/tmp"
    if args.pid:
        return os.path.join(tmp, f"strikehub-inspect-{args.pid}.sock")
    cands = []
    for f in glob.glob(os.path.join(tmp, "strikehub-inspect-*.path")):
        pid = live_pid(f)
        if pid:
            cands.append((os.path.getmtime(f), pid, f))
    if not cands:
        sys.exit("error: no live strikehub-inspect socket found (is the app running with STRIKEHUB_INSPECTOR=1?)")
    cands.sort(reverse=True)
    pid, path_file = cands[0][1], cands[0][2]
    with open(path_file) as fh:
        sock = fh.read().strip()
    print(f"# using pid={pid} socket={sock}", file=sys.stderr)
    return sock


def call(sock_path: str, request: dict, retries: int = 10) -> dict:
    """Send one JSON request, read one JSON response line.

    Retries while the app reports the webview is not up yet.
    """
    payload = (json.dumps(request) + "\n").encode()
    last_err = None
    for attempt in range(retries):
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.settimeout(90)
            s.connect(sock_path)
            s.sendall(payload)
            buf = b""
            while not buf.endswith(b"\n"):
                chunk = s.recv(1 << 20)
                if not chunk:
                    break
                buf += chunk
            s.close()
            if not buf:
                last_err = "empty response"
                continue
            resp = json.loads(buf.decode().strip().splitlines()[-1])
            if resp.get("ok") is False and "no WKWebView found" in resp.get("error", ""):
                last_err = resp["error"]
                time.sleep(1.0)
                continue
            return resp
        except (ConnectionRefusedError, FileNotFoundError, socket.timeout) as e:
            last_err = f"{type(e).__name__}: {e}"
            time.sleep(1.0)
    sys.exit(f"error: {last_err}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="explicit unix socket path")
    ap.add_argument("--pid", type=int, help="strikehub pid (socket derived from it)")
    ap.add_argument("command", choices=["screenshot", "eval", "click", "type", "wait", "text", "targets"])
    ap.add_argument("arg1", nargs="?", help="output file (screenshot) / js (eval) / css (click,type,text,wait)")
    ap.add_argument("arg2", nargs="?", help="text to type")
    ap.add_argument("timeout_ms", nargs="?", type=int, help="wait timeout (default 15000)")
    args = ap.parse_args()

    sock = find_socket(args)

    if args.command == "targets":
        resp = call(sock, {"cmd": "targets"})
    elif args.command == "screenshot":
        if not args.arg1:
            sys.exit("error: screenshot needs an output path")
        resp = call(sock, {"cmd": "screenshot"})
    elif args.command == "eval":
        if args.arg1 is None:
            sys.exit("error: eval needs a JS expression")
        resp = call(sock, {"cmd": "eval", "js": args.arg1})
    elif args.command == "click":
        if not args.arg1:
            sys.exit("error: click needs a css selector")
        resp = call(sock, {"cmd": "click", "css": args.arg1})
    elif args.command == "type":
        if not args.arg1 or args.arg2 is None:
            sys.exit("error: type needs a css selector and text")
        resp = call(sock, {"cmd": "type", "css": args.arg1, "text": args.arg2})
    elif args.command == "wait":
        if not args.arg1:
            sys.exit("error: wait needs a css selector")
        timeout = args.timeout_ms or (int(args.arg2) if args.arg2 else 15000)
        resp = call(sock, {"cmd": "wait", "css": args.arg1, "timeout_ms": timeout})
    elif args.command == "text":
        if not args.arg1:
            sys.exit("error: text needs a css selector")
        resp = call(sock, {"cmd": "text", "css": args.arg1})
    else:  # pragma: no cover
        sys.exit("error: bad command")

    if resp.get("ok") is False:
        sys.exit(f"error: {resp.get('error')}")
    if args.command == "screenshot":
        with open(args.arg1, "wb") as fh:
            fh.write(base64.b64decode(resp["png_b64"]))
        print(f"wrote {args.arg1}")
    else:
        print(json.dumps(resp, indent=2))


if __name__ == "__main__":
    main()
