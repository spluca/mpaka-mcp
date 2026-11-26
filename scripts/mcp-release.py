#!/usr/bin/env python3
"""Package and verify versioned mpaka-mcp release assets."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
import subprocess
import tarfile
import zipfile
from pathlib import Path


TARGETS = {
    "x86_64-unknown-linux-gnu": ("tar.gz", "mpaka-mcp"),
    "aarch64-unknown-linux-gnu": ("tar.gz", "mpaka-mcp"),
    "x86_64-apple-darwin": ("tar.gz", "mpaka-mcp"),
    "aarch64-apple-darwin": ("tar.gz", "mpaka-mcp"),
    "x86_64-pc-windows-msvc": ("zip", "mpaka-mcp.exe"),
}
VERSION_RE = re.compile(r"\d+\.\d+\.\d+\Z")


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def asset_name(version: str, target: str) -> str:
    suffix, _ = TARGETS[target]
    return f"mpaka-mcp-v{version}-{target}.{suffix}"


def package(version: str, target: str, binary: Path, output_dir: Path) -> Path:
    if not VERSION_RE.fullmatch(version):
        raise ValueError("version must use MAJOR.MINOR.PATCH")
    if target not in TARGETS:
        raise ValueError(f"unsupported target: {target}")
    if not binary.is_file() or binary.is_symlink():
        raise ValueError("binary must be a regular, non-symlink file")
    output_dir.mkdir(parents=True, exist_ok=True)
    archive = output_dir / asset_name(version, target)
    member_name = TARGETS[target][1]
    if archive.exists():
        archive.unlink()
    if archive.suffix == ".zip":
        info = zipfile.ZipInfo(member_name, date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = 0o100755 << 16
        with zipfile.ZipFile(archive, "w") as output:
            output.writestr(info, binary.read_bytes())
    else:
        with archive.open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w") as output:
                    info = tarfile.TarInfo(member_name)
                    info.size = binary.stat().st_size
                    info.mode = 0o755
                    info.uid = 0
                    info.gid = 0
                    info.uname = ""
                    info.gname = ""
                    info.mtime = 0
                    with binary.open("rb") as source:
                        output.addfile(info, source)
    return archive


def manifest(version: str, release_dir: Path, signing_key: Path) -> None:
    if not VERSION_RE.fullmatch(version):
        raise ValueError("version must use MAJOR.MINOR.PATCH")
    artifacts = []
    for target in sorted(TARGETS):
        path = release_dir / asset_name(version, target)
        if not path.is_file():
            raise ValueError(f"missing release asset: {path.name}")
        artifacts.append(
            {
                "filename": path.name,
                "target": target,
                "size": path.stat().st_size,
                "sha256": digest(path),
            }
        )
    data = {"version": f"v{version}", "artifacts": artifacts}
    manifest_path = release_dir / "manifest.json"
    manifest_path.write_text(
        json.dumps(data, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    signature_path = release_dir / "manifest.json.sig"
    subprocess.run(
        ["openssl", "dgst", "-sha256", "-sign", str(signing_key), "-out", str(signature_path), str(manifest_path)],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    paths = sorted([release_dir / item["filename"] for item in artifacts] + [manifest_path, signature_path])
    (release_dir / "SHA256SUMS").write_text(
        "".join(f"{digest(path)}  {path.name}\n" for path in paths), encoding="ascii"
    )


def verify(version: str, release_dir: Path, public_key: Path) -> None:
    manifest_path = release_dir / "manifest.json"
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    if data.get("version") != f"v{version}":
        raise ValueError("release manifest version does not match")
    artifacts = data.get("artifacts")
    if (
        not isinstance(artifacts, list)
        or len(artifacts) != len(TARGETS)
        or {item.get("target") for item in artifacts} != set(TARGETS)
    ):
        raise ValueError("release manifest does not contain exactly the supported targets")
    for item in artifacts:
        path = release_dir / item["filename"]
        if path.name != asset_name(version, item["target"]) or not path.is_file():
            raise ValueError("unexpected or missing release asset")
        if item.get("sha256") != digest(path) or item.get("size") != path.stat().st_size:
            raise ValueError(f"asset integrity check failed: {path.name}")
        member_name = TARGETS[item["target"]][1]
        if path.suffix == ".zip":
            with zipfile.ZipFile(path) as archive:
                if archive.namelist() != [member_name]:
                    raise ValueError(f"unexpected archive contents: {path.name}")
        else:
            with tarfile.open(path, "r:gz") as archive:
                members = archive.getmembers()
                if len(members) != 1 or members[0].name != member_name or not members[0].isreg():
                    raise ValueError(f"unexpected archive contents: {path.name}")
    signature = release_dir / "manifest.json.sig"
    result = subprocess.run(
        ["openssl", "dgst", "-sha256", "-verify", str(public_key), "-signature", str(signature), str(manifest_path)],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise ValueError("release manifest signature verification failed")
    sums = (release_dir / "SHA256SUMS").read_text(encoding="ascii").splitlines()
    expected_names = {item["filename"] for item in artifacts} | {"manifest.json", "manifest.json.sig"}
    parsed = {}
    for line in sums:
        checksum, name = line.split("  ", 1)
        parsed[name] = checksum
    if set(parsed) != expected_names:
        raise ValueError("SHA256SUMS has missing or unexpected files")
    for name, checksum in parsed.items():
        if digest(release_dir / name) != checksum:
            raise ValueError(f"checksum verification failed: {name}")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    package_parser = commands.add_parser("package")
    package_parser.add_argument("--version", required=True)
    package_parser.add_argument("--target", required=True)
    package_parser.add_argument("--binary", type=Path, required=True)
    package_parser.add_argument("--output-dir", type=Path, required=True)
    manifest_parser = commands.add_parser("manifest")
    manifest_parser.add_argument("--version", required=True)
    manifest_parser.add_argument("--release-dir", type=Path, required=True)
    manifest_parser.add_argument("--signing-key", type=Path, required=True)
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--version", required=True)
    verify_parser.add_argument("--release-dir", type=Path, required=True)
    verify_parser.add_argument("--public-key", type=Path, required=True)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        if args.command == "package":
            print(package(args.version, args.target, args.binary, args.output_dir))
        elif args.command == "manifest":
            manifest(args.version, args.release_dir, args.signing_key)
        else:
            verify(args.version, args.release_dir, args.public_key)
    except (OSError, ValueError, KeyError, json.JSONDecodeError, zipfile.BadZipFile, tarfile.TarError) as error:
        parser().error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
