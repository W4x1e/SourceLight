"""Shared bits for the Florence-2 and SigLIP 2 spikes: the library, images, sessions, VRAM."""
import json, os, subprocess, threading, time
import numpy as np
from PIL import Image
import onnxruntime as ort

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.join(os.environ["LOCALAPPDATA"], "app.sourcelight")
PHOTOS = json.load(open(os.path.join(HERE, "photos.json"), encoding="utf-8"))


def image(p):
    path = p["preview"] if p["preview"] and os.path.exists(p["preview"]) else p["thumb"]
    return Image.open(path).convert("RGB")


def session(path, provider="dml", basic=False):
    so = ort.SessionOptions()
    so.log_severity_level = 3
    if basic:
        so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_BASIC
    provs = ["DmlExecutionProvider", "CPUExecutionProvider"] if provider == "dml" else ["CPUExecutionProvider"]
    return ort.InferenceSession(path, so, providers=provs)


def vram_used_mb():
    try:
        out = subprocess.check_output(
            ["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"], text=True)
        return int(out.strip().splitlines()[0])
    except Exception:
        return -1


class VramPeak:
    """Polls nvidia-smi while a block runs; peak minus the idle baseline."""

    def __enter__(self):
        self.base = vram_used_mb()
        self.peak = self.base
        self._stop = False
        self._t = threading.Thread(target=self._poll, daemon=True)
        self._t.start()
        return self

    def _poll(self):
        while not self._stop:
            self.peak = max(self.peak, vram_used_mb())
            time.sleep(0.2)

    def __exit__(self, *a):
        self._stop = True
        self._t.join()

    @property
    def delta(self):
        return self.peak - self.base


def dump(name, obj):
    with open(os.path.join(HERE, "out", name), "w", encoding="utf-8") as f:
        json.dump(obj, f, indent=1, ensure_ascii=False)


os.makedirs(os.path.join(HERE, "out"), exist_ok=True)
