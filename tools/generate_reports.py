#!/usr/bin/env python3
"""Generate exact reports from the user's pinned Minecraft server JAR."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-jar", type=Path, required=True)
    parser.add_argument("--java", default="java", help="Java 25 executable")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    pin = json.loads((Path(__file__).resolve().parent.parent / "config/minecraft-26.3.json").read_text())
    server = args.server_jar.resolve()
    if hashlib.sha1(server.read_bytes()).hexdigest() != pin["server_sha1"]:
        parser.error("server JAR does not match the pinned Minecraft 26.3 SHA-1")
    output = args.output.resolve()
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        parser.error("output must be absent or empty")
    with tempfile.TemporaryDirectory(prefix="orbital-datagen-") as temp:
        subprocess.run([args.java, "-DbundlerMainClass=net.minecraft.data.Main", "-jar", str(server),
                        "--reports", "--output", str(output)], cwd=temp, check=True)
    for name, expected in pin["reports_sha1"].items():
        if hashlib.sha1((output / "reports" / name).read_bytes()).hexdigest() != expected:
            raise SystemExit(f"Generated {name} does not match the pinned report hash")
    print(f"Verified {pin['version']} reports: {output / 'reports'}")


if __name__ == "__main__":
    main()
