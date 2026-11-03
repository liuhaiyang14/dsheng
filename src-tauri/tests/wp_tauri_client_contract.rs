//! S5 联调冒烟（静态）：TauriClient 的 command 名/参数形状与壳层注册表一致。
//!
//! 不开真窗口：解析 src-ui/src/api/tauriClient.ts 的全部
//! `this.invoke('<cmd>', { ... })` 调用（命令名 + 参数对象顶层键集），
//! 与 `dsheng_app::contracts::COMMANDS` 逐一比对（双向），并核对
//! lib.rs `generate_handler![...]` 注册了全部 command。
//! （工作包说明：文件读取比对可接受。）

use dsheng_app::contracts::COMMANDS;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn repo_path(relative: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/src-tauri
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// 从 tauriClient.ts 提取 invoke 清单：cmd → 参数键集（按出现顺序）。
/// 兼容 `this.invoke('cmd')` 与 `this.invoke<{...}>('cmd')`（返回类型泛型）。
fn extract_invokes(source: &str) -> BTreeMap<String, Vec<String>> {
    let mut found = BTreeMap::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while let Some(pos) = source[i..].find("this.invoke") {
        let mut start = i + pos + "this.invoke".len();
        i = start;
        // 可选泛型参数 <{...}>：跨过到 '>' 结束（不含嵌套泛型的简单形式用括号深度）
        if bytes.get(start) == Some(&b'<') {
            let mut depth = 0usize;
            while start < bytes.len() {
                match bytes[start] {
                    b'<' => depth += 1,
                    b'>' => {
                        depth -= 1;
                        if depth == 0 {
                            start += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                start += 1;
            }
            i = start;
        }
        // 跳过 '('
        if bytes.get(start) != Some(&b'(') {
            continue;
        }
        start += 1;
        i = start;
        // 命令名（单引号字符串）
        let Some(name) = bytes.get(start).filter(|b| **b == b'\'').map(|_| {
            let end = source[start + 1..].find('\'').expect("命令名闭合") + start + 1;
            let name = source[start + 1..end].to_string();
            i = end + 1;
            name
        }) else {
            continue;
        };
        // 第二参数对象（可选）：跳过空白与逗号，遇 `{` 收集顶层键
        let mut keys = Vec::new();
        let mut j = i;
        while j < bytes.len() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b',' {
            j += 1;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'{' {
                keys = top_level_keys(&source[j..]);
                i = j;
            }
        }
        found.entry(name).or_insert(keys);
    }
    found
}

/// 对象字面量顶层键（shorthand `{ a }` 与 `a: expr` 都取 `a`）。
fn top_level_keys(src: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let mut expect_key = true; // depth==1 时收集标识符
    let mut chars = src.chars().peekable();
    let mut in_string: Option<char> = None;
    let mut consumed_first_brace = false;
    while let Some(c) = chars.next() {
        if let Some(q) = in_string {
            if c == '\\' {
                chars.next();
            } else if c == q {
                in_string = None;
            }
            if depth >= 1 {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                in_string = Some(c);
                if depth >= 1 {
                    current.push(c);
                }
            }
            '{' | '(' | '[' => {
                depth += 1;
                if c == '{' && depth == 1 {
                    consumed_first_brace = true;
                    expect_key = true;
                    current.clear();
                    continue;
                }
                if depth >= 1 {
                    current.push(c);
                }
            }
            '}' | ')' | ']' => {
                if c == '}' && depth == 1 {
                    if !current.trim().is_empty() {
                        keys.push(key_of(&current));
                    }
                    return keys; // 顶层对象结束
                }
                depth = depth.saturating_sub(1);
                if depth >= 1 {
                    current.push(c);
                }
            }
            ',' if depth == 1 => {
                if !current.trim().is_empty() {
                    keys.push(key_of(&current));
                }
                current.clear();
                expect_key = true;
            }
            _ => {
                if depth >= 1 {
                    current.push(c);
                }
                let _ = expect_key;
            }
        }
        let _ = consumed_first_brace;
    }
    keys
}

/// `key: value` / `key` / `key ?? null` → key（首个标识符）。
fn key_of(fragment: &str) -> String {
    let trimmed = fragment.trim();
    trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect()
}

#[test]
fn s5_tauri_client_invokes_match_registry_exactly() {
    let path = repo_path("../src-ui/src/api/tauriClient.ts");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取 tauriClient.ts 失败（{e}）；UI 文件不可移动"));
    let invokes = extract_invokes(&source);

    let registry: BTreeMap<&str, Vec<&str>> =
        COMMANDS.iter().map(|c| (c.name, c.args.to_vec())).collect();

    // 双向：UI 有 ↔ 注册表有
    let ui_names: Vec<&String> = invokes.keys().collect();
    let reg_names: Vec<&str> = registry.keys().copied().collect();
    assert_eq!(
        ui_names, reg_names,
        "UI invoke 命令集与壳层 COMMANDS 注册表不一致"
    );
    assert_eq!(ui_names.len(), 31, "UI DshengClient 应有 31 条命令");

    // 参数键集逐一比对（顺序无关、内容一致；含可选参数恒定发送的键）
    for (name, ui_keys) in &invokes {
        let reg_keys = registry[name.as_str()].clone();
        let mut ui_sorted = ui_keys.clone();
        ui_sorted.sort();
        let mut reg_sorted: Vec<&str> = reg_keys.clone();
        reg_sorted.sort();
        assert_eq!(
            ui_sorted, reg_sorted,
            "command {name} 参数键集不一致：UI={ui_keys:?} 注册表={reg_keys:?}"
        );
    }
}

#[test]
fn s5_generate_handler_registers_all_commands() {
    let lib = std::fs::read_to_string(repo_path("src/lib.rs")).expect("读取 lib.rs");
    let handler = lib
        .split("generate_handler![")
        .nth(1)
        .and_then(|rest| rest.split("]").next())
        .expect("lib.rs 含 generate_handler![...]");
    for spec in COMMANDS {
        let path = format!("contracts::commands::{}", spec.name);
        assert!(handler.contains(&path), "generate_handler 未注册 {path}");
    }
    assert!(handler.contains("contracts::ping"), "ping 仍应注册");
}

#[test]
fn s5_command_param_names_align_with_ui_keys() {
    // Rust 参数为 snake_case、UI 键为 camelCase（tauri 默认 lower_camel_case）。
    // 逐 command 比对 commands.rs 源内 fn 签名与注册表键。
    let src =
        std::fs::read_to_string(repo_path("src/contracts/commands.rs")).expect("读取 commands.rs");
    for spec in COMMANDS {
        let sig = src
            .split(&format!("pub fn {}(", spec.name))
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .unwrap_or_else(|| panic!("commands.rs 缺 fn {}", spec.name));
        for key in spec.args {
            let snake = camel_to_snake(key);
            assert!(
                sig.contains(&format!("{}:", snake)),
                "fn {} 参数应含 {}（UI 键 {key}）",
                spec.name,
                snake
            );
        }
    }
}

fn camel_to_snake(key: &str) -> String {
    let mut out = String::new();
    for c in key.chars() {
        if c.is_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
