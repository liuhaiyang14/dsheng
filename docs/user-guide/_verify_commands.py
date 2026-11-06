#!/usr/bin/env python3
"""_verify_commands.py —— 手册命令一致性检查。

扫描 docs/user-guide/*.md（不含本文件与 checklist 的非命令文本）中出现的每一条
`dsheng <cmd> …` 命令，逐条与 contracts/v1/cli-surface.json 的 55 条命令比对：

1. 命令路径（子命令序列）必须能在合同 commands 里找到对应条目；
2. 命令中的 `--flag` 必须出现在合同 syntax 的 flag 集合或文档化全局/别名集合中。

用法：python3 docs/user-guide/_verify_commands.py [仓库根目录]
退出码：0 全部一致；1 存在不一致（未知命令/未知 flag）或文件缺失。
"""

import json
import re
import sys
from pathlib import Path

# 文档化全局 flag（合同之外、CLI 实现/文档明确声明）
GLOBAL_FLAGS = {"--json", "--db"}
# cli-surface aliases 注记的 task update CLI sugar flag
ALIAS_FLAGS = {
    ("task", "update"): {"--progress", "--status", "--note"},
}

FENCE_RE = re.compile(r"```[^\n]*\n(.*?)```", re.S)
INLINE_RE = re.compile(r"`([^`\n]+)`")
# 「dsheng」作为独立词且后随空白：排除 dsheng-agent、路径 /dsheng、~/.local/bin/dsheng
DSHENG_RE = re.compile(r"(?<![A-Za-z0-9_/.-])dsheng(?=\s)")


def load_contract(root: Path):
    surface = json.loads((root / "contracts/v1/cli-surface.json").read_text(encoding="utf-8"))
    paths = {}  # verb_path tuple -> set[command name]
    flags = {}  # command name -> set of flags from syntax
    for c in surface["commands"]:
        toks = c["syntax"].split()
        assert toks[0] == "dsheng", c["syntax"]
        verb, fl = [], set()
        verb_open = True  # 动词段在第一个 flag 处结束
        for t in toks[1:]:
            t2 = t.strip("[]()")  # 合同语法中的可选/互斥段 [--flag <v>] (--a | --b)
            if t2.startswith("--"):
                # 去掉内联值，如 --framework <5why|5w|5w2h> → --framework
                fl.add(t2.split("<")[0].rstrip("]"))
                verb_open = False
            elif verb_open and not t2.startswith("-") and not t2.startswith("<"):
                verb.append(t2)
        # 特例：version/help 的动词是 flag 本身
        if "--version" in fl and not verb:
            verb = ["--version"]
        if "--help" in fl and not verb:
            verb = ["--help"]
        key = tuple(verb)
        paths.setdefault(key, set()).add(c["command"])
        flags[c["command"]] = fl
    return paths, flags


def verb_done(verb):
    """语法串里动词段在第一个 flag 处结束；export/task 等组的动词不超过 3 段。"""
    return len(verb) >= 3


def doc_commands(md_text: str):
    """产出 (line_no, tokens_after_dsheng) 列表；处理围栏块的反斜杠续行。"""
    out = []
    lines = md_text.splitlines()

    def scan(text, base_line):
        for m in DSHENG_RE.finditer(text):
            toks = text[m.end():].split()
            toks = [t.strip("()\"'，。；）") for t in toks]
            toks = [t for t in toks if t]
            out.append((base_line, toks))

    # 1) 围栏代码块（续行合并）
    pos, line_of_pos = 0, 1
    for fm in FENCE_RE.finditer(md_text):
        before = md_text[pos:fm.start()]
        line_of_pos += before.count("\n")
        # 块内
        block_lines = fm.group(1).splitlines()
        buf, buf_start = "", None
        for ln in block_lines:
            if buf_start is None:
                buf_start = line_of_pos
            if ln.rstrip().endswith("\\"):
                buf += ln.rstrip()[:-1] + " "
                line_of_pos += 1
                continue
            buf += ln
            scan(buf, buf_start)
            buf, buf_start = "", None
            line_of_pos += 1
        if buf:
            scan(buf, buf_start)
        pos = fm.end()
        line_of_pos = fm.start(1) and md_text[:fm.end()].count("\n") + 1
    # 2) 行内代码（排除已在围栏内的——行内正则只在块外文本上再跑一遍）
    stripped = FENCE_RE.sub(lambda _: "", md_text)
    for im in INLINE_RE.finditer(stripped):
        scan(im.group(1), stripped[:im.start()].count("\n") + 1)
    return out


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    guide = root / "docs/user-guide"
    paths, flags_by_cmd = load_contract(root)

    md_files = sorted(p for p in guide.glob("*.md"))
    if not md_files:
        print(f"错误：未找到手册文件（{guide}）")
        return 1

    total, bad = 0, []
    per_file = {}
    for md in md_files:
        text = md.read_text(encoding="utf-8")
        cmds = doc_commands(text)
        per_file[md.name] = len(cmds)
        for line_no, toks in cmds:
            total += 1
            verb = []
            for t in toks:
                if t.startswith("--") or t.startswith("<") or t in {"\\", "…", "..."}:
                    break
                verb.append(t)
                if len(verb) >= 3:
                    break
            if not verb and toks and toks[0] in ("--version", "--help"):
                verb = [toks[0]]
            key = tuple(verb)
            matched = paths.get(key)
            if not matched:
                bad.append((md.name, line_no, "未知命令路径: dsheng " + " ".join(verb)))
                continue
            allowed = set(GLOBAL_FLAGS)
            for name in matched:
                allowed |= flags_by_cmd[name]
                for alias_key, extra in ALIAS_FLAGS.items():
                    if name.startswith(" ".join(alias_key)):
                        allowed |= extra
            for t in toks:
                if t.startswith("--"):
                    flag = t.split("<")[0].rstrip(",;)]")
                    if flag not in allowed:
                        bad.append((md.name, line_no, f"未知 flag {flag}（命令 dsheng {' '.join(verb)}）"))

    print(f"手册文件 {len(md_files)} 个；扫描到 dsheng 命令 {total} 条：")
    for name, n in per_file.items():
        print(f"  {name}: {n}")
    if bad:
        print(f"\n不一致 {len(bad)} 处：")
        for f, ln, msg in bad:
            print(f"  {f}:{ln}  {msg}")
        return 1
    print("\n全部命令与 contracts/v1/cli-surface.json 一致（55 条命令集合内；flag 均有出处）。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
