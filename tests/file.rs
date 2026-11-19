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
