#!/usr/bin/env python3
"""Generate exact reports from the user's pinned Minecraft server JAR."""
import argparse
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-jar", type=Path, required=True)
    parser.add_argument("--java", default="java", help="Java 25 executable")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--javac", help="JDK 25 compiler; defaults beside --java")
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
        java = Path(shutil.which(args.java) or args.java).resolve()
        javac = args.javac or str(java.with_name("javac.exe" if os.name == "nt" else "javac"))
        classpath = os.pathsep.join(str(p) for p in sorted(Path(temp).rglob("*.jar")))
        classes = Path(temp) / "oracle-classes"
        classes.mkdir()
        subprocess.run([javac, "-cp", classpath, "-d", str(classes),
                        str(Path(__file__).resolve().with_name("VanillaOracle.java"))], check=True)
        physics = output / "reports/physics.json"
        subprocess.run([str(java), "-cp", str(classes) + os.pathsep + classpath,
                        "VanillaOracle", str(physics)], cwd=temp, check=True)
        if hashlib.sha1(physics.read_bytes()).hexdigest() != pin["physics_sha1"]:
            raise SystemExit("Generated physics does not match the pinned report hash")
    for name, expected in pin["reports_sha1"].items():
        if hashlib.sha1((output / "reports" / name).read_bytes()).hexdigest() != expected:
            raise SystemExit(f"Generated {name} does not match the pinned report hash")
    registry = json.loads((output / "reports/registries.json").read_text())
    digest = hashlib.sha1()
    for name in sorted(registry["minecraft:item"]["entries"]):
        namespace, item = name.split(":", 1)
        data = (output / "reports" / namespace / "components/item" / f"{item}.json").read_bytes()
        digest.update(name.encode())
        digest.update(b"\0")
        digest.update(len(data).to_bytes(8, "little"))
        digest.update(data)
    if digest.hexdigest() != pin["item_components_sha1"]:
        raise SystemExit("Generated item components do not match the pinned aggregate hash")
    print(f"Verified {pin['version']} reports: {output / 'reports'}")


if __name__ == "__main__":
    main()
