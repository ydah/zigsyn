use std::io::{self, Write};
use std::path::PathBuf;

#[test]
fn parses_zig_stdlib_when_configured() {
    let Some(src_dir) = std::env::var_os("ZIG_SRC_DIR") else {
        let _ = writeln!(
            io::stderr(),
            "Skipping Zig standard library walk, set ZIG_SRC_DIR to enable it"
        );
        return;
    };

    let std_dir = PathBuf::from(src_dir).join("lib/std");
    assert!(
        std_dir.join("std.zig").is_file(),
        "ZIG_SRC_DIR must contain lib/std/std.zig, missing under {}",
        std_dir.display()
    );

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
