use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

#[test]
fn compares_with_zig_ast_check_when_configured() {
    let std_dir = std::env::var_os("ZIG_SRC_DIR").map(|src_dir| {
        let std_dir = PathBuf::from(src_dir).join("lib/std");
        assert!(
            std_dir.join("std.zig").is_file(),
            "ZIG_SRC_DIR must contain lib/std/std.zig, missing under {}",
            std_dir.display()
        );
        std_dir
    });
    let zig = std::env::var_os("ZIG_BIN");
    if let Some(zig) = &zig {
        let version = Command::new(zig)
            .arg("version")
            .output()
            .unwrap_or_else(|err| panic!("could not run ZIG_BIN {zig:?}: {err}"));
        assert!(
            version.status.success(),
            "ZIG_BIN {zig:?} failed to report its version"
        );
    }
    let (Some(std_dir), Some(zig)) = (std_dir, zig) else {
        let _ = writeln!(
            io::stderr(),
            "Skipping Zig AST comparison, set ZIG_SRC_DIR and ZIG_BIN to enable it"
        );
        return;
    };

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

            let ours = zigsyn::parse_file(&path);
            let zig_output = Command::new(&zig)
                .arg("ast-check")
                .arg(&path)
                .output()
                .unwrap_or_else(|err| {
                    panic!("could not run zig ast-check on {}: {err}", path.display())
                });
            assert_eq!(
                ours.is_ok(),
                zig_output.status.success(),
                "ast-check mismatch for {}\nzigsyn error: {:?}\nZig output: {}",
                path.display(),
                ours.err(),
                String::from_utf8_lossy(&zig_output.stderr),
            );
        }
    }
}
