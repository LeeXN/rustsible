use anyhow::{bail, Context, Result};
use regex::Regex;

/// Match Ansible 2.21's last-match/default insertion behavior. Python text IO
/// normalizes CRLF/CR only when a change is written; a no-op keeps original
/// bytes. Keep line endings in the vector so inserting at BOF does not also
/// manufacture a newline at the unrelated EOF.
pub fn process_line_modifications(
    content: &mut String,
    line: Option<String>,
    regexp: Option<String>,
    state: &str,
    after: Option<String>,
    before: Option<String>,
) -> Result<bool> {
    if line
        .as_ref()
        .is_some_and(|line| line.contains(['\r', '\n']))
    {
        bail!("line must be a single line");
    }
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = normalized
        .split_inclusive('\n')
        .map(str::to_owned)
        .collect();
    let regexp = regexp.map(|pattern| Regex::new(&pattern)).transpose()?;
    let changed = match state {
        "present" => {
            let desired = line.as_ref().context("state=present requires line")?;
            let found = regexp
                .as_ref()
                .and_then(|regex| lines.iter().rposition(|line| matches(regex, line)))
                .or_else(|| {
                    lines
                        .iter()
                        .rposition(|line| line.trim_end_matches('\n') == desired)
                });
            let new_line = format!("{desired}\n");
            if let Some(index) = found {
                if lines[index] == new_line {
                    false
                } else {
                    lines[index] = new_line;
                    true
                }
            } else {
                let position =
                    if after.as_deref() == Some("BOF") || before.as_deref() == Some("BOF") {
                        0
                    } else if let Some(pattern) = after.filter(|pattern| pattern != "EOF") {
                        let regex = Regex::new(&pattern)?;
                        lines
                            .iter()
                            .rposition(|line| matches(&regex, line))
                            .map_or(lines.len(), |index| index + 1)
                    } else if let Some(pattern) = before {
                        let regex = Regex::new(&pattern)?;
                        lines
                            .iter()
                            .rposition(|line| matches(&regex, line))
                            .unwrap_or(lines.len())
                    } else {
                        lines.len()
                    };
                if position == lines.len() {
                    if let Some(last) = lines.last_mut() {
                        if !last.ends_with('\n') {
                            last.push('\n');
                        }
                    }
                }
                lines.insert(position, new_line);
                true
            }
        }
        "absent" => {
            if regexp.is_none() && line.is_none() {
                bail!("state=absent requires line or regexp");
            }
            let old = lines.len();
            lines.retain(|entry| {
                !regexp.as_ref().map_or_else(
                    || line.as_deref() == Some(entry.trim_end_matches('\n')),
                    |regex| matches(regex, entry),
                )
            });
            old != lines.len()
        }
        _ => bail!("Invalid lineinfile state"),
    };
    if changed {
        *content = lines.concat();
    }
    Ok(changed)
}
fn matches(regex: &Regex, line: &str) -> bool {
    // Python $ can match before the final newline; Rust's $ is strict. The
    // raw alternative also retains support for explicit newline expressions.
    regex.is_match(line) || regex.is_match(line.trim_end_matches('\n'))
}
