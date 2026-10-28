#!/usr/bin/env python3
"""Drives the survey3d window over its agent link (MCP over HTTP) and takes
screenshots of it, for the tutorial.

survey3d must be running with Settings -> General -> Agent link on. On macOS the
terminal running this needs Screen Recording permission (for screencapture).

    from drive import App
    app = App()
    app.call("run_commands", lines=["ze"])
    app.shot("02-imported")                 # whole window
    app.shot("03-cmdline", crop="bottom")    # the command line area
"""

import base64
import json
import os
import subprocess
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
IMAGES = os.path.join(HERE, "..", "images")
WINID = os.path.join(HERE, "winid")


def link_info():
    for path in [
        os.environ.get("SURVEY3D_AGENT_LINK"),
        os.path.expanduser("~/Library/Application Support/survey3d/agent-link.json"),
        os.path.expanduser("~/.config/survey3d/agent-link.json"),
    ]:
        if path and os.path.exists(path):
            with open(path) as f:
                return json.load(f)
    raise SystemExit("No agent-link.json: turn on Settings -> General -> Agent link in survey3d")


class App:
    def __init__(self):
        info = link_info()
        self.url, self.token, self.next_id = info["url"], info["token"], 1
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "tutorial", "version": "1"}})
        if not os.path.exists(WINID):
            subprocess.run(["swiftc", "-O", os.path.join(HERE, "winid.swift"), "-o", WINID], check=True)
        os.makedirs(IMAGES, exist_ok=True)

    def rpc(self, method, params):
        body = {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        self.next_id += 1
        req = urllib.request.Request(
            self.url,
            data=json.dumps(body).encode(),
            headers={
                "Content-Type": "application/json",
                "Accept": "application/json, text/event-stream",
                "Authorization": "Bearer " + self.token,
            },
        )
        reply = json.loads(urllib.request.urlopen(req, timeout=600).read())
        if "error" in reply:
            raise RuntimeError(reply["error"])
        return reply["result"]

    def call(self, tool, **arguments):
        """Calls a tool; returns its text (and saves any image to save_image)."""
        save_image = arguments.pop("save_image", None)
        result = self.rpc("tools/call", {"name": tool, "arguments": arguments})
        texts = []
        for c in result.get("content", []):
            if c.get("type") == "text":
                texts.append(c["text"])
            elif c.get("type") == "image" and save_image:
                with open(os.path.join(IMAGES, save_image + ".png"), "wb") as f:
                    f.write(base64.b64decode(c["data"]))
        text = "\n".join(texts)
        if result.get("isError"):
            raise RuntimeError(f"{tool}: {text}")
        return text

    def run(self, *lines):
        return self.call("run_commands", lines=list(lines))

    def shot(self, name, crop=None, wait=0.6):
        """Screenshot of the window. crop: None, a (x, y, w, h) box in window
        pixels, or a named region (bottom, right, left-top)."""
        time.sleep(wait)  # let the window draw what just changed
        out = subprocess.run([WINID], capture_output=True, text=True, check=True).stdout.split()
        wid, w, h = int(out[0]), int(out[1]), int(out[2])
        path = os.path.join(IMAGES, name + ".png")
        subprocess.run(["screencapture", "-x", "-o", "-l", str(wid), path], check=True)
        # Retina windows capture at twice their size in points.
        pw = int(subprocess.run(["sips", "-g", "pixelWidth", path], capture_output=True, text=True).stdout.split()[-1])
        k = pw / w
        boxes = {
            "bottom": (0, h - 140, w, 140),
            "right": (w - 310, 60, 310, 365),
        }
        box = boxes.get(crop, crop)
        if box:
            x, y, bw, bh = (int(v * k) for v in box)
            subprocess.run(["magick", path, "-crop", f"{bw}x{bh}+{x}+{y}", "+repage", path], check=True)
        return path
