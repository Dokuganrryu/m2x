const CONDITIONS: [&str; 16] = [
    "eq", "ne", "cs", "hs", "cc", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le",
];

pub fn without_qualifier(mnemonic: &str) -> &str {
    mnemonic.split('.').next().unwrap_or(mnemonic)
}

pub fn is_op(mnemonic: &str, base: &str) -> bool {
    without_qualifier(mnemonic)
        .strip_prefix(base)
        .is_some_and(|condition| condition.is_empty() || CONDITIONS.contains(&condition))
}

pub fn is_op_setting_flags_or_not(mnemonic: &str, base: &str) -> bool {
    let plain = without_qualifier(mnemonic);
    is_op(plain, base)
        || plain
            .strip_prefix(base)
            .and_then(|rest| rest.strip_prefix('s'))
            .is_some_and(|condition| condition.is_empty() || CONDITIONS.contains(&condition))
}

// WHY: Armv8-M ARM, every floating-point and coprocessor-10/11 mnemonic starts with `v`, and no Armv8-M Mainline integer mnemonic does
pub fn is_floating_point(mnemonic: &str) -> bool {
    mnemonic.starts_with('v')
}

pub fn operand_list(operands: &str) -> Vec<&str> {
    let mut list = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, c) in operands.char_indices() {
        match c {
            '[' | '{' => depth += 1,
            ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                list.push(operands[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    let last = operands[start..].trim();
    if !last.is_empty() {
        list.push(last);
    }
    list
}

pub fn general_register(operand: &str) -> Option<usize> {
    let number: usize = operand
        .trim()
        .trim_end_matches('!')
        .strip_prefix('r')?
        .parse()
        .ok()?;
    (number <= 12).then_some(number)
}

pub fn register_list(operand: &str) -> Vec<usize> {
    let Some(inner) = operand
        .trim()
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return Vec::new();
    };
    inner
        .split(',')
        .flat_map(|item| match item.split_once('-') {
            Some((first, last)) => match (general_register(first), general_register(last)) {
                (Some(first), Some(last)) => (first..=last).collect(),
                _ => Vec::new(),
            },
            None => general_register(item).into_iter().collect(),
        })
        .collect()
}

pub fn branch_target(operands: &str) -> Option<&str> {
    let start = operands.find('<')?;
    let end = operands.rfind('>')?;
    let target = operands.get(start + 1..end)?;
    Some(match target.rfind("+0x") {
        Some(offset) => &target[..offset],
        None => target,
    })
}
