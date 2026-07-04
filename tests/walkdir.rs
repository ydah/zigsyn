use std::path::Path;

#[test]
fn parses_zig_stdlib_when_configured() {
    let Ok(src_dir) = std::env::var("ZIG_SRC_DIR") else {
        return;
    };

    let std_dir = Path::new(&src_dir).join("lib/std");
    if !std_dir.exists() {
        return;
    }

    let mut stack = vec![std_dir];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|name| name.to_str());
                if matches!(name, Some("zig-cache" | "zig-out")) {
                    continue;
                }
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("zig") {
                zigsyn::parse_file(&path).unwrap_or_else(|err| {
                    panic!("failed to parse {}: {err}", path.display());
                });
            }
        }
    }
}
