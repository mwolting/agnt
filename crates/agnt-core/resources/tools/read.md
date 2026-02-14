Read lines from a file relative to the working directory.

Returns hashline-formatted lines in `line:hash|content` format, where `line` is 1-based and `hash` is a short content hash prefix used for anchored edits.

Supports pagination:
- `offset` is a 0-based line offset (default `0`)
- `limit` is max lines to return (optional; when omitted, reads through end of file; when provided, values above `20000` are clamped)
