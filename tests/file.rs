use zigsyn::ast::ContainerMember;
use zigsyn::scanner::{ScanOptions, Scanner};
use zigsyn::Parser;

#[test]
fn parses_helloworld_source() {
    let file = zigsyn::parse_source(
        r#"
const std = @import("std");

pub fn main() void {
    std.debug.print("hello\n", .{});
}
"#,
    )
    .unwrap();

    assert_eq!(file.members.len(), 2);
    assert!(matches!(file.members[0], ContainerMember::Var(_)));
    assert!(matches!(file.members[1], ContainerMember::Fn(_)));
}

#[test]
fn parse_dir_reads_direct_zig_files() {
    let files = zigsyn::parse_dir("tests/testdata").unwrap();
    assert!(files.contains_key("helloworld.zig"));
}

#[test]
fn parser_respects_scanner_whitespace_options() {
    let source = "const value =\r1;";
    let path = "tests/testdata/tabbed.zig";
    assert!(zigsyn::parse_source(source).is_ok());
    assert!(zigsyn::parse_file(path).is_ok());

    let options = ScanOptions {
        strict_whitespace: true,
    };
    for scanner in [
        Scanner::with_options(source, options),
        Scanner::from_file_with_options(path, options).unwrap(),
    ] {
        let error = Parser::from_scanner(scanner).parse_file().unwrap_err();
        assert!(error
            .to_string()
            .contains("invalid whitespace outside PEG skip rule"));
    }
}

#[test]
fn parse_errors_report_byte_offsets() {
    let lexical_input = "const x = \"é\" #;";
    let error: zigsyn::Error = zigsyn::parse_source(lexical_input).unwrap_err();
    assert!(matches!(
        error,
        zigsyn::Error::Syntax { offset, .. } if offset == lexical_input.find('#').unwrap()
    ));

    let syntax_input = "const x = \"é\"; var a u8;";
    let error = zigsyn::parse_source(syntax_input).unwrap_err();
    assert!(matches!(
        error,
        zigsyn::Error::UnexpectedToken { offset, .. } if offset == syntax_input.find("u8").unwrap()
    ));
}

#[test]
fn parsed_ast_can_be_shared_across_threads() {
    fn require_thread_safe(_: impl Send + Sync) {}

    let file = zigsyn::parse_source("/// Main entry point.\npub fn main() void {}").unwrap();
    require_thread_safe(file);
}

#[test]
fn file_io_errors_include_path_and_preserve_source() {
    let path = "tests/testdata/missing-source.zig";
    let error = zigsyn::parse_file(path).unwrap_err();
    assert!(error.to_string().contains(path));
    let source = std::error::Error::source(&error).unwrap();
    let io_error = source.downcast_ref::<std::io::Error>().unwrap();
    assert_eq!(io_error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn directory_results_keep_failures_and_skip_directories() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("zigsyn-parse-dir-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    std::fs::create_dir(directory.join("ignored.zig")).unwrap();
    std::fs::write(directory.join("good.zig"), "const good = 1;").unwrap();
    std::fs::write(directory.join("bad.zig"), "const bad = ;").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("good.zig", directory.join("link.zig")).unwrap();

    let entries = zigsyn::parse_dir_entries(&directory);
    let fail_fast = zigsyn::parse_dir(&directory);
    std::fs::remove_dir_all(&directory).unwrap();

    let entries = entries.unwrap();
    assert!(entries["good.zig"].is_ok());
    assert!(entries["bad.zig"].is_err());
    assert!(!entries.contains_key("ignored.zig"));
    #[cfg(unix)]
    assert!(entries["link.zig"].is_ok());
    assert!(matches!(
        fail_fast,
        Err(zigsyn::Error::UnexpectedToken { path, .. }) if path == directory.join("bad.zig")
    ));
}
