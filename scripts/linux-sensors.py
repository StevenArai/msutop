"""Read-only RAPL sensor helper; SMART temperature is polled every five seconds."""
import argparse
import glob
import json
import resource
import select
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path


def number(path):
    try:
        return int(Path(path).read_text().strip())
    except (OSError, ValueError):
        return None


def smart_temperature(document):
    value = document.get("temperature", {}).get("current")
    if value is None:
        value = document.get("nvme_smart_health_information_log", {}).get("temperature")
    if isinstance(value, (int, float)) and 0 <= value <= 150:
        return float(value)
    for attribute in document.get("ata_smart_attributes", {}).get("table", []):
        if attribute.get("id") in (190, 194):
            # Packed ATA raw integers can contain Min/Max in upper bytes.
            try:
                value = float(attribute.get("raw", {}).get("string", "").split()[0])
            except (ValueError, IndexError):
                continue
            if 0 <= value <= 150:
                return value
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--disk", action="append", default=[])
    parser.add_argument("--smart-seconds", type=float, default=5.0)
    args = parser.parse_args()
    domains = []
    for directory in glob.glob("/sys/class/powercap/*"):
        path = Path(directory)
        try:
            if path.joinpath("name").read_text().startswith("package"):
                domains.append((path / "energy_uj", number(path / "max_energy_range_uj")))
        except OSError:
            pass
    previous = {}
    smartctl = shutil.which("smartctl") or ("/usr/sbin/smartctl" if Path("/usr/sbin/smartctl").exists() else None)
    lock = threading.Lock()
    smart = {"temperature": None, "timestamp": 0.0, "polls": 0, "errors": 0, "wall_ms": 0.0}

    def poll_smart():
        values = []
        started = time.monotonic()
        errors = 0
        for disk in args.disk:
            if smartctl is None:
                errors += 1
                continue
            try:
                result = subprocess.run([smartctl, "-B", "/dev/null", "-j", "-A", "-n", "standby,0", disk],
                                        capture_output=True, text=True, timeout=2.0)
                temperature = smart_temperature(json.loads(result.stdout))
                if temperature is not None:
                    values.append(temperature)
                elif result.returncode & 7:
                    errors += 1
            except (OSError, subprocess.TimeoutExpired, ValueError):
                errors += 1
        with lock:
            smart.update(temperature=max(values) if values else None, timestamp=time.monotonic(),
                         polls=smart["polls"] + 1, errors=smart["errors"] + errors,
                         wall_ms=(time.monotonic() - started) * 1000)

    worker = None
    next_smart = 0.0
    started_cpu = resource.getrusage(resource.RUSAGE_SELF)
    started_children = resource.getrusage(resource.RUSAGE_CHILDREN)
    while True:
        now = time.monotonic()
        if now >= next_smart and (worker is None or not worker.is_alive()):
            worker = threading.Thread(target=poll_smart, daemon=True)
            worker.start()
            next_smart = now + max(5.0, args.smart_seconds)
        watts = []
        for path, maximum in domains:
            value = number(path)
            before = previous.get(path)
            if value is not None and before is not None:
                delta = value - before[0]
                if delta < 0 and maximum is not None:
                    delta += maximum
                if delta >= 0 and now > before[1]:
                    watts.append(delta / 1_000_000 / (now - before[1]))
            if value is None:
                previous.pop(path, None)
            else:
                previous[path] = (value, now)
        with lock:
            disk = dict(smart)
        usage = resource.getrusage(resource.RUSAGE_SELF)
        children = resource.getrusage(resource.RUSAGE_CHILDREN)
        cpu_seconds = usage.ru_utime + usage.ru_stime - started_cpu.ru_utime - started_cpu.ru_stime
        children_seconds = children.ru_utime + children.ru_stime - started_children.ru_utime - started_children.ru_stime
        result = {"cpu_watts": sum(watts) if watts else None,
                  "disk_temperature": disk["temperature"] if now - disk["timestamp"] < 15 else None,
                  "helper_cpu_seconds": cpu_seconds, "smartctl_cpu_seconds": children_seconds,
                  "smart_polls": disk["polls"], "smart_errors": disk["errors"], "smart_wall_ms": disk["wall_ms"]}
        try:
            print(json.dumps(result, separators=(",", ":")), flush=True)
        except BrokenPipeError:
            break
        ready, _, _ = select.select([sys.stdin], [], [], 1.0)
        if ready and not sys.stdin.buffer.read(1):
            break


if __name__ == "__main__":
    main()
