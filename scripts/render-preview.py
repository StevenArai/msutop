"""Generate documentation from actual Rust frames; Pillow is needed only for these artifacts."""
import argparse
from pathlib import Path
import struct
import subprocess

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
SCENES = ("balanced", "cpu", "io", "memory", "idle", "offline")
MEMORY_WIDTH = 39  # Include the divider at x=38 in the detail crop.


def rgb565_image(data: bytes) -> Image.Image:
    assert len(data) == 160 * 80 * 2
    rgb = bytearray(160 * 80 * 3)
    for index, (pixel,) in enumerate(struct.iter_unpack(">H", data)):
        r, g, b = (pixel >> 11) & 31, (pixel >> 5) & 63, pixel & 31
        rgb[index * 3:index * 3 + 3] = bytes(((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)))
    return Image.frombytes("RGB", (160, 80), bytes(rgb))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, default=ROOT / "target/gnu/release/msutop.exe")
    parser.add_argument("--animate", action="store_true")
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--seconds", type=float, default=8.0)
    args = parser.parse_args()
    exe = args.exe.resolve()
    output, docs = ROOT / "output", ROOT / "docs"
    output.mkdir(exist_ok=True)
    docs.mkdir(exist_ok=True)
    if not 1 <= args.fps <= 60 or args.seconds <= 0:
        parser.error("--fps must be 1..60 and --seconds must be positive")
    subprocess.run([str(exe), "--export", str(output), "--scale", "4", "--fps", str(args.fps)], check=True)
    Image.open(output / "balanced-default-160x80.bmp").save(docs / "screen-160x80.png")
    Image.open(output / "balanced-default-4x.bmp").save(docs / "preview.png")
    Image.open(output / "balanced-vivid-4x.bmp").save(docs / "vivid.png")
    memory = Image.open(output / "balanced-default-160x80.bmp").crop((0, 22, MEMORY_WIDTH, 54))
    memory.resize((MEMORY_WIDTH * 8, 256), Image.Resampling.NEAREST).save(docs / "memory.png")
    network_sheet = Image.new("RGB", (64 * 6 * 3 + 16, 32 * 6 + 28), "#101010")
    network_labels = ImageDraw.Draw(network_sheet)
    for index, (scene, label) in enumerate((("balanced", "GBE"), ("idle", "FE"), ("offline", "offline"))):
        x = index * (64 * 6 + 8)
        network_labels.text((x + 4, 5), label, fill="#cccccc", font=ImageFont.load_default())
        crop = Image.open(output / f"{scene}-default-160x80.bmp").crop((96, 22, 160, 54))
        network_sheet.paste(crop.resize((64 * 6, 32 * 6), Image.Resampling.NEAREST), (x, 24))
    network_sheet.save(docs / "network.png")
    network_cases = (("down", "down"), ("nocarrier", "nocarrier"),
                     ("active", "active / idle"), ("transfer", "active / traffic"))
    network_states = Image.new("RGB", (64 * 6 * 2 + 8, (32 * 6 + 24) * 2), "#101010")
    network_state_labels = ImageDraw.Draw(network_states)
    for index, (state, label) in enumerate(network_cases):
        directory = output / f"network-{state}"
        subprocess.run([str(exe), "--export", str(directory), "--scale", "4", "--fps", str(args.fps),
                        "--network", state], check=True)
        x, y = index % 2 * (64 * 6 + 8), index // 2 * (32 * 6 + 24)
        network_state_labels.text((x + 4, y + 5), label, fill="#cccccc", font=ImageFont.load_default())
        crop = Image.open(directory / "balanced-default-160x80.bmp").crop((96, 22, 160, 54))
        network_states.paste(crop.resize((64 * 6, 32 * 6), Image.Resampling.NEAREST), (x, y + 24))
    network_states.save(docs / "network-states.png")
    pixel_output = output / "no-aa"
    subprocess.run([str(exe), "--export", str(pixel_output), "--scale", "4", "--fps", str(args.fps), "--no-aa"], check=True)
    Image.open(pixel_output / "balanced-default-4x.bmp").save(docs / "pixel.png")
    comparison = Image.new("RGB", (640, 688), "#101010")
    labels = ImageDraw.Draw(comparison)
    labels.text((5, 6), "AA / CPU + disk + network + process graphs, columns and rings / RGB565", fill="#cccccc", font=ImageFont.load_default())
    comparison.paste(Image.open(output / "balanced-default-4x.bmp"), (0, 24))
    labels.text((5, 350), "noAA / same values, padding and colors / RGB565", fill="#cccccc", font=ImageFont.load_default())
    comparison.paste(Image.open(pixel_output / "balanced-default-4x.bmp"), (0, 368))
    comparison.save(docs / "aa-comparison.png")
    memory_comparison = Image.new("RGB", (MEMORY_WIDTH * 8 + 8, 556), "#101010")
    memory_labels = ImageDraw.Draw(memory_comparison)
    for index, directory in enumerate((output, pixel_output)):
        top = index * 278
        memory_labels.text((6, top + 4), "AA" if index == 0 else "noAA", fill="#cccccc", font=ImageFont.load_default())
        crop = Image.open(directory / "balanced-default-160x80.bmp").crop((0, 22, MEMORY_WIDTH, 54))
        memory_comparison.paste(crop.resize((MEMORY_WIDTH * 8, 256), Image.Resampling.NEAREST), (4, top + 20))
    memory_comparison.save(docs / "memory-aa-comparison.png")
    memory_pressure = Image.new("RGB", (MEMORY_WIDTH * 8 * 2 + 8, 280), "#101010")
    pressure_labels = ImageDraw.Draw(memory_pressure)
    for index, (scene, label) in enumerate((("balanced", "used / cached / free"), ("memory", "low memory + low swap"))):
        x = index * (MEMORY_WIDTH * 8 + 8)
        pressure_labels.text((x + 4, 5), label, fill="#cccccc", font=ImageFont.load_default())
        crop = Image.open(output / f"{scene}-default-160x80.bmp").crop((0, 22, MEMORY_WIDTH, 54))
        memory_pressure.paste(crop.resize((MEMORY_WIDTH * 8, 256), Image.Resampling.NEAREST), (x, 24))
    memory_pressure.save(docs / "memory-pressure.png")
    sheet = Image.new("RGB", (640 * 2 + 16, (320 + 28) * 3 + 16), "#101010")
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default()
    for index, scene in enumerate(SCENES):
        x, y = (index % 2) * (640 + 8) + 4, (index // 2) * (320 + 28) + 4
        draw.text((x + 4, y + 5), scene, fill="#cccccc", font=font)
        sheet.paste(Image.open(output / f"{scene}-default-4x.bmp"), (x, y + 24))
    sheet.save(docs / "states.png")
    cpu_cases = (4, 8, 16, 32, 64)
    cpu_sheet = Image.new("RGB", (640, len(cpu_cases) * 116), "#101010")
    cpu_labels = ImageDraw.Draw(cpu_sheet)
    for index, threads in enumerate(cpu_cases):
        directory = output / f"cpu-{threads}-threads"
        subprocess.run([str(exe), "--export", str(directory), "--scale", "4", "--fps", str(args.fps),
                        "--mock-threads", str(threads)], check=True)
        top = index * 116
        cpu_labels.text((6, top + 5), f"{threads} threads / one bar per thread / right aligned",
                        fill="#cccccc", font=font)
        crop = Image.open(directory / "balanced-default-160x80.bmp").crop((0, 0, 160, 22))
        cpu_sheet.paste(crop.resize((640, 88), Image.Resampling.NEAREST), (0, top + 24))
    cpu_sheet.save(docs / "cpu-responsive.png")
    if args.animate:
        raw = output / "animation.rgb565"
        frame_count = round(args.fps * args.seconds)
        subprocess.run([str(exe), "--headless", "--rgb565", str(raw), "--frames", str(frame_count), "--fps", str(args.fps), "--scene", "balanced"], check=True)
        data = raw.read_bytes()
        stride = 25_600
        assert len(data) == frame_count * stride
        frames = [rgb565_image(data[index:index + stride]).resize((640, 320), Image.Resampling.NEAREST)
                  for index in range(0, len(data), stride)]
        # GIF stores centiseconds; alternating 30/40 ms delays preserve the 30 fps average.
        durations = [10 * (round((i + 1) * 100 / args.fps) - round(i * 100 / args.fps))
                     for i in range(frame_count)]
        frames[0].save(docs / "preview.gif", save_all=True, append_images=frames[1:], duration=durations,
                       loop=0, optimize=True, disposal=1)
        network_frames = [frame.crop((96 * 4, 22 * 4, 160 * 4, 54 * 4))
                          .resize((512, 256), Image.Resampling.NEAREST) for frame in frames]
        network_frames[0].save(docs / "network-activity.gif", save_all=True, append_images=network_frames[1:],
                               duration=durations, loop=0, optimize=True, disposal=1)
    print(f"Preview artifacts saved in {docs}")


if __name__ == "__main__":
    main()
