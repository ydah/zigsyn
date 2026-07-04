use zigsyn::ast::ContainerMember;

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
