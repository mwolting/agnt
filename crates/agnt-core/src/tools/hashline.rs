pub(crate) const MAX_READ_LIMIT: usize = 20_000;
const HASH_PREFIX_LEN: usize = 4;

pub(crate) struct FileLines {
    pub(crate) lines: Vec<String>,
    pub(crate) line_ending: String,
    pub(crate) trailing_newline: bool,
}

impl FileLines {
    pub(crate) fn parse(content: &str) -> Self {
        let line_ending = if content.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let normalized = content.replace("\r\n", "\n");
        let trailing_newline = normalized.ends_with('\n');

        let mut lines = if normalized.is_empty() {
            Vec::new()
        } else {
            normalized
                .split('\n')
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
        };
        if trailing_newline && !lines.is_empty() {
            lines.pop();
        }

        Self {
            lines,
            line_ending: line_ending.to_string(),
            trailing_newline,
        }
    }

    pub(crate) fn render(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }

        let mut rendered = self.lines.join(&self.line_ending);
        if self.trailing_newline {
            rendered.push_str(&self.line_ending);
        }
        rendered
    }
}

pub(crate) fn hashline(line_no: usize, line: &str) -> String {
    format!("{line_no}:{}|{line}", line_hash_prefix(line))
}

pub(crate) fn resolve_anchor(anchor: &str, lines: &[String]) -> Result<usize, String> {
    if lines.is_empty() {
        return Err("cannot resolve anchor in an empty file".to_string());
    }

    let (line_no, hash_prefix) = parse_anchor(anchor)?;
    let expected_idx = line_no.saturating_sub(1);

    if expected_idx < lines.len()
        && line_hash_prefix(&lines[expected_idx]).starts_with(&hash_prefix)
    {
        return Ok(expected_idx);
    }

    let mut matches: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            line_hash_prefix(line)
                .starts_with(&hash_prefix)
                .then_some(idx)
        })
        .collect();

    if matches.is_empty() {
        return Err(format!("anchor `{anchor}` not found"));
    }
    if matches.len() == 1 {
        return Ok(matches[0]);
    }

    matches.sort_by_key(|idx| idx.abs_diff(expected_idx));
    let best = matches[0];
    let best_distance = best.abs_diff(expected_idx);
    let second_distance = matches[1].abs_diff(expected_idx);

    if best_distance == second_distance {
        let candidates = matches
            .iter()
            .take(4)
            .map(|idx| (idx + 1).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "anchor `{anchor}` is ambiguous (candidate line numbers: {candidates})"
        ));
    }

    Ok(best)
}

pub(crate) fn replacement_lines(content: &str) -> Vec<String> {
    let normalized = content.replace("\r\n", "\n");
    let mut lines = if normalized.is_empty() {
        vec![String::new()]
    } else {
        normalized
            .split('\n')
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
    };

    if normalized.ends_with('\n') && lines.len() > 1 {
        lines.pop();
    }

    lines
}

fn parse_anchor(anchor: &str) -> Result<(usize, String), String> {
    let trimmed = anchor.trim();
    let (line_no_raw, hash_raw) = trimmed
        .split_once(':')
        .ok_or_else(|| format!("invalid anchor `{anchor}` (expected `line:hash`)"))?;

    let line_no = line_no_raw
        .parse::<usize>()
        .map_err(|_| format!("invalid line number in anchor `{anchor}`"))?;
    if line_no == 0 {
        return Err(format!("invalid line number in anchor `{anchor}`"));
    }

    let hash_prefix = hash_raw.trim().to_lowercase();
    if hash_prefix.len() < 2 {
        return Err(format!(
            "invalid hash prefix in anchor `{anchor}` (minimum 2 characters)"
        ));
    }
    if hash_prefix.len() > HASH_PREFIX_LEN {
        return Err(format!(
            "invalid hash prefix in anchor `{anchor}` (maximum {HASH_PREFIX_LEN} characters)"
        ));
    }
    if !hash_prefix.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!(
            "invalid hash prefix in anchor `{anchor}` (must be hex)"
        ));
    }

    Ok((line_no, hash_prefix))
}

fn line_hash_prefix(line: &str) -> String {
    // FNV-1a 64-bit.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in line.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    let hex = format!("{hash:016x}");
    hex[..HASH_PREFIX_LEN].to_string()
}
