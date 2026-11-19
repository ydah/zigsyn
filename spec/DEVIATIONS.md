# Deviations

- A UTF-8 BOM at byte zero is skipped to match Zig. Source offsets still
  include its bytes.
- Tabs and carriage returns between tokens are accepted by default to match
  Zig. Set `ScanOptions::strict_whitespace` to require the PEG whitespace rule.
