# zigsyn

`zigsyn` is a pure Rust scanner and parser for Zig source code. It parses Zig
syntax into a structured AST. Declarations and other nodes retain token lists,
and the scanner records token start positions as byte offsets.

The parser is driven by the bundled [`spec/grammar.peg`](spec/grammar.peg)
snapshot from [`ziglang/zig-spec`](https://github.com/ziglang/zig-spec).

API documentation is available on [docs.rs](https://docs.rs/zigsyn).

## Features

- Pure Rust scanner and parser for Zig syntax.
- Public helpers for parsing source strings, individual files, and directories.
- Structured AST types exposed through `zigsyn::ast`.
- Structured syntax errors instead of unparsed token buckets.
- Optional `serde` support for AST and token data.
- Grammar snapshot and deviation notes kept under [`spec/`](spec/).

## Installation

Add `zigsyn` to your `Cargo.toml`:

```toml
[dependencies]
zigsyn = "0.1"
```

To enable serialization support:

```toml
[dependencies]
zigsyn = { version = "0.1", features = ["serde"] }
```

If you are using this repository directly:

```toml
[dependencies]
zigsyn = { path = "../zigsyn" }
```

## Usage

Parse a source string:

```rust
fn main() {
    let file = zigsyn::parse_source("pub fn main() void {}")
        .expect("valid Zig source");

    assert_eq!(file.members.len(), 1);
}
```

Parse a file:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = zigsyn::parse_file("src/main.zig")?;
    println!("top-level members: {}", file.members.len());
    Ok(())
}
```

For lower-level control, construct a parser directly:

```rust
let mut parser = zigsyn::Parser::from("1 + 2 * 3");
let expression = parser.expression().expect("valid Zig expression");
```

## API Overview

- `zigsyn::parse_source(source)` parses a string into `ast::File`.
- `zigsyn::parse_file(path)` parses a `.zig` file into `ast::File`.
- `zigsyn::parse_dir(path)` parses direct `.zig` children of a directory.
- `zigsyn::parse_dir_entries(path)` returns each direct `.zig` file's parse result.
- `zigsyn::Parser` exposes lower-level parsing entry points.
- `zigsyn::scanner::Scanner` exposes tokenization.

### Migrating from 0.1.0

- Replace `var.ty` with `var.ty()` to borrow the type annotation on the first
  left-hand side entry.
- Replace `init.fields` with `init.fields()` to iterate over initializer values
  in source order.
- Parse functions return `zigsyn::Error`. Match `Error::Syntax` in place of
  `Error::Else`. Syntax and unexpected-token errors expose a zero-based byte
  `offset`. `Error::IO` exposes the underlying error as `source` and an optional
  `path`.
- AST comments use `Arc<Comment>` so parsed ASTs can be shared across threads.
- AST token lists use `zigsyn::token::TokenSpan`, an immutable slice of the
  parser's shared token buffer. Read the tokens with `tokens.as_slice()`, or
  convert a `Vec<Token>` with `.into()` when constructing nodes. With `serde`,
  each token list is serialized and deserialized as an array.

With `serde`, `VarDecl` serializes its type annotations in `lhs` and `AnonInit`
serializes its values in `entries`. Update JSON consumers that read the former
`ty` and `fields` keys. The current AST can read JSON emitted by the 0.1.0 parser.
For formats that encode struct fields by position, regenerate the ASTs from Zig
source or decode them with 0.1.0 and convert them to the current AST.

## Project Layout

- [`src/ast.rs`](src/ast.rs): AST definitions.
- [`src/scanner.rs`](src/scanner.rs): Zig tokenizer.
- [`src/parser.rs`](src/parser.rs): parser implementation.
- [`src/token.rs`](src/token.rs): token and operator definitions.
- [`spec/`](spec/): grammar snapshot and deviation notes.
- [`tests/`](tests/): integration and fixture-based tests.
- [`benches/`](benches/): Criterion benchmarks.
- [`fuzz/`](fuzz/): cargo-fuzz target.

## Development

Run the standard checks:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Run benchmarks:

```sh
cargo bench
```

Run the fuzz target with `cargo-fuzz`:

```sh
cargo fuzz run parse_source
```

## Specification

The active grammar snapshot is documented in [`spec/README.md`](spec/README.md).
Accepted grammar deviations and implementation decisions are tracked in
[`spec/DEVIATIONS.md`](spec/DEVIATIONS.md).

## License

Licensed under the MIT License.
