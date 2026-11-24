pub const PERMITTED_COMMENT: &str = "// WHY:";

pub fn comment_lines(source: &str) -> Vec<usize> {
    let text = source.as_bytes();
    let mut lines = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        at += if rest.starts_with(b"//") {
            if !rest.starts_with(PERMITTED_COMMENT.as_bytes()) {
                lines.push(line_of(text, at));
            }
            length_to_line_end(rest)
        } else if rest.starts_with(b"/*") {
            lines.push(line_of(text, at));
            length_of_block_comment(rest)
        } else if let Some(length) = length_of_raw_string(rest) {
            length
        } else if rest[0] == b'"' {
            length_of_string(rest)
        } else if rest[0] == b'\'' {
            length_of_char_or_lifetime(rest)
        } else {
            1
        };
    }
    lines
}

fn line_of(text: &[u8], at: usize) -> usize {
    1 + text[..at].iter().filter(|&&b| b == b'\n').count()
}

fn length_to_line_end(rest: &[u8]) -> usize {
    rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len())
}

fn length_of_block_comment(rest: &[u8]) -> usize {
    rest.windows(2)
        .skip(2)
        .position(|pair| pair == b"*/")
        .map_or(rest.len(), |end| end + 4)
}

fn length_of_raw_string(rest: &[u8]) -> Option<usize> {
    let after_r = rest.strip_prefix(b"r")?;
    let hashes = after_r.iter().take_while(|&&b| b == b'#').count();
    if after_r.get(hashes) != Some(&b'"') {
        return None;
    }
    let body = &after_r[hashes + 1..];
    let mut closing = vec![b'"'];
    closing.extend(std::iter::repeat_n(b'#', hashes));
    let end = body
        .windows(closing.len())
        .position(|window| window == closing.as_slice())
        .map_or(body.len(), |end| end + closing.len());
    Some(1 + hashes + 1 + end)
}

fn length_of_string(rest: &[u8]) -> usize {
    let mut at = 1;
    while at < rest.len() {
        match rest[at] {
            b'\\' => at += 2,
            b'"' => return at + 1,
            _ => at += 1,
        }
    }
    rest.len()
}

fn length_of_char_or_lifetime(rest: &[u8]) -> usize {
    match rest.get(1..) {
        Some([b'\\', ..]) => rest[2..]
            .iter()
            .position(|&b| b == b'\'')
            .map_or(rest.len(), |end| end + 3),
        Some([_, b'\'', ..]) => 3,
        _ => 1,
    }
}
