#!/usr/bin/env python3
"""统一版本清单（用例 35/39/70）。

扫描构建产物目录，生成 docs/release/manifest-v<ver>.json：
四类产物（桌面安装/便携、dsheng-agent Skill、独立 CLI、中文手册）
× 平台 × 架构 × 文件 × 大小 × SHA-256 × 版本。

版本一致性（硬校验，不一致报错退出）：
  - 根 Cargo.toml [workspace.package].version
  - src-tauri/tauri.conf.json .version
辅助记录（仅告警，--strict 可升级为报错）：
  - src-tauri/Cargo.toml .package.version（包元数据）
  - src-ui/package.json .version

用法：
  python3 scripts/package/version_manifest.py                     # 扫描 target/release-dist
  python3 scripts/package/version_manifest.py --dist-dir /tmp/out --out docs/release/manifest-v0.1.0.json
  python3 scripts/package/version_manifest.py --print-version     # 供 shell 脚本取版本
  python3 scripts/package/version_manifest.py --verify docs/release/manifest-v0.1.0.json --dist-dir DIR

产物命名约定（分类依据，命名不符即报错，防止清单单边漂移）：
  桌面安装/便携：dsheng_<ver>_macos_<arm64|x64>.dmg
                dsheng_<ver>_macos_<arm64|x64>_portable.zip
                dsheng_<ver>_windows_x64.msi
                dsheng_<ver>_windows_x64_portable.zip
  独立 CLI：     dsheng-cli_<ver>_macos_<arm64|x64>.zip
                dsheng-cli_<ver>_windows_x64.zip
  Skill ZIP：    dsheng-agent-skill_<ver>.zip
  手册：         dsheng-manual_<ver>.zip / dsheng-manual_<ver>.pdf
安装辅助文件与 migration 直接取仓库内文件计入清单（发布包含同一清单）。
清单只记文件名与哈希；大二进制产物不入 git（放 target/ 或 /tmp，见 docs/release/README.md）。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import zipfile
from datetime import datetime, timezone
from pathlib import Path

SCHEMA_VERSION = 1
PRODUCT = "dsheng"

# 期望产物矩阵（docs/specs/provider-config-delivery.md §6）。清单如实记录
# present/missing，不虚构未构建产物。
EXPECTED_ARTIFACTS = [
    ("desktop", "dmg", "macos", "arm64"),
    ("desktop", "portable-zip", "macos", "arm64"),
    ("desktop", "dmg", "macos", "x64"),
    ("desktop", "portable-zip", "macos", "x64"),
    ("desktop", "msi", "windows", "x64"),
    ("desktop", "portable-zip", "windows", "x64"),
    ("cli", "cli-zip", "macos", "arm64"),
    ("cli", "cli-zip", "macos", "x64"),
    ("cli", "cli-zip", "windows", "x64"),
    ("skill", "skill-zip", None, None),
    ("manual", "manual-zip", None, None),
]

# dist 目录里允许存在但不计入产物清单的辅助文件。
IGNORED_DIST_NAMES = re.compile(r"(?i)^(sha256sums\.txt|checksums\.txt|.*\.sha256)$")


def die(msg: str, code: int = 2) -> None:
    print(f"version_manifest: 错误: {msg}", file=sys.stderr)
    sys.exit(code)


def info(msg: str) -> None:
    print(f"version_manifest: {msg}", file=sys.stderr)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def load_versions(repo: Path) -> dict:
    versions = {
        "cargo_workspace": read_cargo_workspace_version(repo / "Cargo.toml"),
        "tauri_conf": read_json_version(repo / "src-tauri" / "tauri.conf.json"),
        "tauri_cargo": read_cargo_package_version(repo / "src-tauri" / "Cargo.toml"),
        "ui_package": read_json_version(repo / "src-ui" / "package.json"),
    }
    if versions["cargo_workspace"] != versions["tauri_conf"]:
        die(
            "版本不一致：根 Cargo.toml workspace.package.version="
            f"{versions['cargo_workspace']!r} != src-tauri/tauri.conf.json version={versions['tauri_conf']!r}"
        )
    return versions


def read_cargo_workspace_version(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    try:
        import tomllib

        data = tomllib.loads(text)
        version = data.get("workspace", {}).get("package", {}).get("version")
        if isinstance(version, str) and version:
            return version
    except ModuleNotFoundError:
        pass
    # 无 tomllib（Python<3.11）时的保守回退：取 [workspace.package] 段内首个 version。
    m = re.search(r"^\[workspace\.package\]\s*$(.*?)^\[", text, re.M | re.S)
    section = m.group(1) if m else text
    m2 = re.search(r'^version\s*=\s*"([^"]+)"', section, re.M)
    if not m2:
        die(f"{path} 内未找到 workspace.package.version")
    return m2.group(1)


def read_cargo_package_version(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    try:
        import tomllib

        data = tomllib.loads(text)
        version = data.get("package", {}).get("version")
        if isinstance(version, str) and version:
            return version
    except ModuleNotFoundError:
        pass
    m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    return m.group(1) if m else ""


def read_json_version(path: Path) -> str:
    data = json.loads(path.read_text(encoding="utf-8"))
    version = data.get("version")
    if not isinstance(version, str) or not version:
        die(f"{path} 内未找到 version 字段")
    return version


def git_info(repo: Path, exclude_manifest: Path | None = None) -> dict:
    def run(*args: str) -> str:
        try:
            return subprocess.run(
                ["git", "-C", str(repo), *args], capture_output=True, text=True, check=True
            ).stdout.strip()
        except subprocess.CalledProcessError:
            return ""

    excluded = set()
    if exclude_manifest is not None:
        excluded = {
            str(exclude_manifest),
            exclude_manifest.name,
            f"docs/release/{exclude_manifest.name}",
        }

    return {
        "commit": run("rev-parse", "HEAD"),
        "branch": run("rev-parse", "--abbrev-ref", "HEAD"),
        # 清单自身属预期写出产物，不计入 dirty
        "dirty": any(
            line[3:].strip() not in excluded
            for line in run("status", "--porcelain").splitlines()
            if line.strip()
        ),
    }


def classify(filename: str, version: str):
    """按命名约定分类产物；返回 (category, kind, platform, arch) 或 None。"""
    # (正则, 分类, 种类, 平台, 固定架构；正则捕获组优先于固定架构)
    patterns = [
        (r"^dsheng_{v}_macos_(arm64|x64)\.dmg$", "desktop", "dmg", "macos", None),
        (r"^dsheng_{v}_macos_(arm64|x64)_portable\.zip$", "desktop", "portable-zip", "macos", None),
        (r"^dsheng_{v}_windows_x64\.msi$", "desktop", "msi", "windows", "x64"),
        (r"^dsheng_{v}_windows_x64_portable\.zip$", "desktop", "portable-zip", "windows", "x64"),
        (r"^dsheng-cli_{v}_macos_(arm64|x64)\.zip$", "cli", "cli-zip", "macos", None),
        (r"^dsheng-cli_{v}_windows_x64\.zip$", "cli", "cli-zip", "windows", "x64"),
        (r"^dsheng-agent-skill_{v}\.zip$", "skill", "skill-zip", None, None),
        (r"^dsheng-manual_{v}\.zip$", "manual", "manual-zip", None, None),
        (r"^dsheng-manual_{v}\.pdf$", "manual", "manual-pdf", None, None),
    ]
    for pat, category, kind, platform, fixed_arch in patterns:
        m = re.match(pat.format(v=re.escape(version)), filename)
        if m:
            arch = m.group(1) if m.groups() else fixed_arch
            return category, kind, platform, arch
    # 版本号不匹配的同族命名：单独识别并报错（陈旧产物混入 dist）。
    loose = [
        (r"^dsheng_.*\.(dmg|msi|zip)$", "desktop"),
        (r"^dsheng-cli_.*\.zip$", "cli"),
        (r"^dsheng-agent-skill_.*\.zip$", "skill"),
        (r"^dsheng-manual_.*\.(zip|pdf)$", "manual"),
    ]
    for pat, category in loose:
        if re.match(pat, filename):
            die(f"产物 {filename} 命名版本与当前版本 {version} 不符（陈旧/异版产物不得入清单）")
    return None


def zip_summary(path: Path) -> dict:
    """Zip 产物内容摘要（校验辅助，不展开全部成员）。"""
    try:
        with zipfile.ZipFile(path) as zf:
            names = zf.namelist()
    except (zipfile.BadZipFile, OSError) as e:  # pragma: no cover - 构建异常路径
        die(f"读取 zip {path} 失败: {e}")
    top = sorted({n.split("/")[0] for n in names})
    return {
        "entries": len(names),
        "top_level": top[:20],
        "includes_migrations": any(
            re.search(r"migrations?/[^/]+\.sql$", n) for n in names
        ),
        "includes_cli_binary": any(
            re.search(r"(^|/)dsheng(-cli)?(\.exe)?$", n) for n in names
        ),
    }


def scan_dist(dist_dirs: list[Path], version: str) -> list[dict]:
    artifacts: list[dict] = []
    unclassified: list[str] = []
    for dist in dist_dirs:
        if not dist.is_dir():
            die(f"产物目录不存在: {dist}")
        for path in sorted(dist.iterdir()):
            if not path.is_file() or IGNORED_DIST_NAMES.match(path.name):
                continue
            got = classify(path.name, version)
            if got is None:
                unclassified.append(str(path))
                continue
            category, kind, platform, arch = got
            entry: dict = {
                "category": category,
                "kind": kind,
                "platform": platform,
                "arch": arch,
                "file": path.name,
                "size_bytes": path.stat().st_size,
                "sha256": sha256_file(path),
            }
            if path.suffix == ".zip":
                entry["zip"] = zip_summary(path)
            artifacts.append(entry)
            info(f"+ {category}/{kind} {platform or '-'} {arch or '-'} {path.name}")
    if unclassified:
        die(
            "产物目录存在无法分类的文件（命名必须符合约定，见脚本头部注释）:\n  "
            + "\n  ".join(unclassified)
        )
    return artifacts


def repo_components(repo: Path) -> dict:
    """随版本发布的仓库内组件：CLI 安装辅助文件 + migration 清单。"""
    helpers = []
    install_dir = repo / "install"
    if install_dir.is_dir():
        for path in sorted(install_dir.iterdir()):
            if not path.is_file():
                continue
            if path.suffix not in {".sh", ".ps1"}:
                continue
            platform = "windows" if path.suffix == ".ps1" else "macos"
            helpers.append(
                {
                    "kind": "install-helper",
                    "platform": platform,
                    "file": str(path.relative_to(repo)),
                    "size_bytes": path.stat().st_size,
                    "sha256": sha256_file(path),
                }
            )
            info(f"+ cli/install-helper {platform} {path.name}")

    migrations = []
    mig_dir = repo / "crates" / "dsheng-core" / "migrations"
    if mig_dir.is_dir():
        for path in sorted(mig_dir.glob("*.sql")):
            migrations.append(
                {
                    "name": path.name,
                    "sha256": sha256_file(path),
                }
            )
    # 与 crates/dsheng-core/src/repository/mod.rs 的 MIGRATIONS include_str 清单核对。
    mod_rs = repo / "crates" / "dsheng-core" / "src" / "repository" / "mod.rs"
    embedded = []
    if mod_rs.is_file():
        embedded = re.findall(r'include_str!\("\.\./\.\./migrations/([^"]+)"\)', mod_rs.read_text(encoding="utf-8"))
    listed = [m["name"] for m in migrations]
    if sorted(embedded) != listed:
        die(
            "migration 清单与 repository MIGRATIONS 不一致:\n"
            f"  MIGRATIONS 嵌入: {sorted(embedded)}\n  目录文件: {listed}"
        )
    return {"cli_install_helpers": helpers, "migrations": migrations, "db_schema_version": len(migrations)}


def build_manifest(repo: Path, dist_dirs: list[Path], versions: dict, version: str) -> dict:
    artifacts = scan_dist(dist_dirs, version)
    components = repo_components(repo)
    tauri_conf = json.loads((repo / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))

    found = {(a["category"], a["kind"], a["platform"], a["arch"]) for a in artifacts}
    coverage = []
    for category, kind, platform, arch in EXPECTED_ARTIFACTS:
        coverage.append(
            {
                "category": category,
                "kind": kind,
                "platform": platform,
                "arch": arch,
                "status": "present" if (category, kind, platform, arch) in found else "missing",
            }
        )

    out_default = repo / "docs" / "release" / f"manifest-v{version}.json"
    return {
        "schema_version": SCHEMA_VERSION,
        "product": PRODUCT,
        "identifier": tauri_conf.get("identifier", ""),
        "version": version,
        "generated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "git": git_info(repo, exclude_manifest=out_default),
        "versions": versions,
        "signing": {"signed": False, "note": "unsigned：不签名、不绕过 Gatekeeper/SmartScreen；见 docs/release/UNSIGNED.md"},
        "artifacts": artifacts,
        "coverage": coverage,
        "components": components,
    }


def verify_manifest(repo: Path, manifest_path: Path, dist_dirs: list[Path]) -> int:
    """包清单核对：按清单重算哈希，与记录逐项比对。"""
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    version = data.get("version", "")
    failures = 0

    def resolve(name: str) -> Path | None:
        for dist in dist_dirs:
            cand = dist / name
            if cand.is_file():
                return cand
        return None

    for art in data.get("artifacts", []):
        path = resolve(art["file"])
        if path is None:
            print(f"VERIFY 缺失: {art['file']}", file=sys.stderr)
            failures += 1
            continue
        actual = sha256_file(path)
        if actual != art["sha256"]:
            print(f"VERIFY 哈希不一致: {art['file']}", file=sys.stderr)
            failures += 1
        elif path.stat().st_size != art.get("size_bytes"):
            print(f"VERIFY 大小不一致: {art['file']}", file=sys.stderr)
            failures += 1
        else:
            print(f"OK {art['file']} {actual[:16]}…")

    for helper in data.get("components", {}).get("cli_install_helpers", []):
        path = repo / helper["file"]
        if not path.is_file():
            print(f"VERIFY 缺失: {helper['file']}", file=sys.stderr)
            failures += 1
        elif sha256_file(path) != helper["sha256"]:
            print(f"VERIFY 哈希不一致: {helper['file']}", file=sys.stderr)
            failures += 1
        else:
            print(f"OK {helper['file']}")

    for mig in data.get("components", {}).get("migrations", []):
        path = repo / "crates" / "dsheng-core" / "migrations" / mig["name"]
        if not path.is_file() or sha256_file(path) != mig["sha256"]:
            print(f"VERIFY migration 不一致: {mig['name']}", file=sys.stderr)
            failures += 1
        else:
            print(f"OK migrations/{mig['name']}")

    live = load_versions(repo)
    if live["cargo_workspace"] != version:
        print(
            f"VERIFY 当前代码版本 {live['cargo_workspace']} 与清单版本 {version} 不一致",
            file=sys.stderr,
        )
        failures += 1

    print(f"核对完成: {'PASS' if failures == 0 else f'FAIL（{failures} 项）'}")
    return 0 if failures == 0 else 1


def main() -> int:
    ap = argparse.ArgumentParser(description="dsheng 统一版本清单")
    ap.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[2])
    ap.add_argument("--dist-dir", type=Path, action="append", default=None, help="产物目录（可多次）")
    ap.add_argument("--out", type=Path, default=None, help="输出 JSON 路径")
    ap.add_argument("--print-version", action="store_true", help="仅打印校验后的版本号")
    ap.add_argument("--verify", type=Path, metavar="MANIFEST", help="核对既有清单")
    ap.add_argument("--strict", action="store_true", help="辅助版本不一致也报错")
    ap.add_argument("--allow-empty", action="store_true", help="无产物时也生成清单（coverage 全 missing）")
    args = ap.parse_args()

    repo = args.repo_root.resolve()
    versions = load_versions(repo)
    version = versions["cargo_workspace"]

    # 辅助版本一致性（tauri Cargo 包版本 / UI package.json）：默认告警。
    for key in ("tauri_cargo", "ui_package"):
        if versions[key] and versions[key] != version:
            msg = f"{key} 版本 {versions[key]!r} 与统一版本 {version!r} 不一致"
            if args.strict:
                die(msg)
            info(f"警告: {msg}")

    if args.print_version:
        print(version)
        return 0

    if args.verify:
        dist_dirs = [d.resolve() for d in (args.dist_dir or [repo / "target" / "release-dist"])]
        return verify_manifest(repo, args.verify.resolve(), dist_dirs)

    dist_dirs = [d.resolve() for d in (args.dist_dir or [repo / "target" / "release-dist"])]
    manifest = build_manifest(repo, dist_dirs, versions, version)

    present = manifest["artifacts"]
    if not present and not args.allow_empty:
        die("未发现任何产物；先运行打包脚本，或用 --allow-empty 显式生成占位清单（coverage 全 missing）")

    out = args.out or (repo / "docs" / "release" / f"manifest-v{version}.json")
    out.parent.mkdir(parents=True, exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)
        f.write("\n")
    missing = [c for c in manifest["coverage"] if c["status"] == "missing"]
    info(f"清单已写入 {out}：产物 {len(present)} 项，缺 {len(missing)} 项（coverage 如实记录）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
