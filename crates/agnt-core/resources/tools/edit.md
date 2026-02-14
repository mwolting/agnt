Apply a sequence of anchored edits to a file relative to the working directory.

Use hashline anchors from `read` output (`line:hash`). Operations are applied in order and can target single lines, anchored ranges, or file-level changes.

Supported operation kinds:
- `replace`
- `insert_before`
- `insert_after`
- `delete`
- `replace_range`
- `delete_range`
- `rewrite_file`
- `move_file`
- `delete_file`
