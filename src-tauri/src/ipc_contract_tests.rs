use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

fn generated_commands() -> BTreeSet<String> {
    include_str!(concat!(env!("OUT_DIR"), "/ipc-command-names.txt"))
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn handler_commands() -> BTreeSet<String> {
    let source = include_str!("lib.rs");
    let marker = "tauri::generate_handler![";
    let marker_start = source
        .find(marker)
        .expect("generate_handler! registry exists");
    let open = marker_start + marker.len() - 1;
    let bytes = source.as_bytes();
    let mut depth = 0_usize;
    let mut close = None;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    source[open + 1..close.expect("generate_handler! list is balanced")]
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.rsplit("::").next().unwrap().to_string())
        .collect()
}

fn permission_commands(manifest_dir: &Path) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    let permissions = manifest_dir.join("permissions");
    for entry in fs::read_dir(permissions).expect("permissions directory exists") {
        let path = entry.expect("permission entry is readable").path();
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("permission TOML is readable");
        let mut remainder = source.as_str();
        while let Some(start) = remainder.find("commands.allow") {
            remainder = &remainder[start..];
            let open = remainder.find('[').expect("commands.allow has an array");
            let close = remainder[open + 1..]
                .find(']')
                .map(|index| open + 1 + index)
                .expect("commands.allow array is closed");
            let body = &remainder[open + 1..close];
            result.extend(quoted_values(body));
            remainder = &remainder[close + 1..];
        }
    }
    result
}

fn frontend_invocations(project_root: &Path) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for entry in WalkDir::new(project_root.join("src"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if !matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("js" | "ts" | "vue")
        ) {
            continue;
        }
        let source = fs::read_to_string(path).expect("frontend source is readable");
        for quote in ['\'', '"'] {
            let needle = format!("invoke({quote}");
            let mut remainder = source.as_str();
            while let Some(start) = remainder.find(&needle) {
                let value = &remainder[start + needle.len()..];
                if let Some(end) = value.find(quote) {
                    result.insert(value[..end].to_string());
                    remainder = &value[end + 1..];
                } else {
                    break;
                }
            }
        }
    }
    result
}

fn quoted_values(source: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut remainder = source;
    while let Some(open) = remainder.find('"') {
        let value = &remainder[open + 1..];
        let Some(close) = value.find('"') else {
            break;
        };
        values.push(value[..close].to_string());
        remainder = &value[close + 1..];
    }
    values
}

fn difference(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.difference(right).cloned().collect()
}

#[test]
fn ipc_registries_and_frontend_calls_are_consistent() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest_dir.parent().expect("Tauri directory has a parent");
    let generated = generated_commands();
    let handler = handler_commands();
    let permissions = permission_commands(&manifest_dir);
    let invoked = frontend_invocations(project_root);

    assert_eq!(
        handler,
        generated,
        "generate_handler! drifted from Rust commands; missing from handler: {:?}; stale in handler: {:?}",
        difference(&generated, &handler),
        difference(&handler, &generated),
    );
    assert_eq!(
        permissions,
        generated,
        "permission TOML drifted from Rust commands; missing permissions: {:?}; stale permissions: {:?}",
        difference(&generated, &permissions),
        difference(&permissions, &generated),
    );
    assert!(
        invoked.is_subset(&generated),
        "frontend invokes unknown commands: {:?}",
        difference(&invoked, &generated),
    );

    let generated_client =
        fs::read_to_string(project_root.join("src/generated/ipc.ts")).expect("IPC client exists");
    for command in &generated {
        assert!(
            generated_client.contains(&format!("  '{command}',")),
            "generated TypeScript client is missing {command}"
        );
    }
}
