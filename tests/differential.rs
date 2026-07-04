use std::path::Path;
use std::process::Command;

#[test]
fn compares_with_zig_ast_check_when_configured() {
    let Ok(src_dir) = std::env::var("ZIG_SRC_DIR") else {
        return;
    };
    let Ok(zig) = std::env::var("ZIG_BIN") else {
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
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("zig") {
                continue;
            }

            let ours = zigsyn::parse_file(&path).is_ok();
            let zig_ok = Command::new(&zig)
                .arg("ast-check")
                .arg(&path)
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            assert_eq!(ours, zig_ok, "ast-check mismatch for {}", path.display());
        }
    }
}
