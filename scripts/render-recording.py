"""Render recorded, actual RGB565 frames without changing their telemetry or timing."""
import argparse
import csv
from pathlib import Path

from PIL import Image

from importlib.util import module_from_spec, spec_from_file_location


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("raw", type=Path)
    parser.add_argument("--timings", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--scale", type=int, default=4)
    parser.add_argument("--skip-seconds", type=float, default=4.3)
    parser.add_argument("--seconds", type=float, default=15.0)
    args = parser.parse_args()
    spec = spec_from_file_location("preview", Path(__file__).with_name("render-preview.py"))
    preview = module_from_spec(spec)
    spec.loader.exec_module(preview)
    data = args.raw.read_bytes()
    stride = 160 * 80 * 2
    if len(data) % stride:
        parser.error("recording ends with a partial frame")
    timings = None
    if args.timings:
        with args.timings.open(newline="", encoding="utf-8") as file:
            timings = [float(row["seconds"]) for row in csv.DictReader(file)]
        if len(timings) != len(data) // stride:
            parser.error("frame count and timing count disagree")
    start = max(0, round(args.skip_seconds * args.fps))
    end = min(len(data) // stride, start + round(args.seconds * args.fps))
    if start >= end:
        parser.error("no frames remain after skipping warm-up")
    frames = [preview.rgb565_image(data[i * stride:(i + 1) * stride])
              .resize((160 * args.scale, 80 * args.scale), Image.Resampling.NEAREST)
              for i in range(start, end)]
    times = (timings[start:end] if timings else [i / args.fps for i in range(start, end)])
    durations = [max(10, 10 * (round((times[i + 1] - times[0]) * 100)
                              - round((times[i] - times[0]) * 100)))
                 for i in range(len(times) - 1)] + [round(1000 / args.fps / 10) * 10]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    frames[0].save(args.output, save_all=True, append_images=frames[1:], duration=durations,
                   loop=0, optimize=True, disposal=1)
    frames[len(frames) // 2].save(args.output.with_suffix(".png"))
    print(f"Rendered {len(frames)} measured frames, {sum(durations) / 1000:.2f}s: {args.output}")


if __name__ == "__main__":
    main()
