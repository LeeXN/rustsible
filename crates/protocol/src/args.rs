use anyhow::Result;
use serde_yaml::Value;

pub fn tokenize_args(input: &str) -> Result<Vec<String>> {
    #[derive(Clone, Copy)]
    enum Quote {
        Single,
        Double,
    }

    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quote = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some(Quote::Single) => {
                if ch == '\'' {
                    quote = None;
                } else {
                    token.push(ch);
                }
            }
            Some(Quote::Double) => match ch {
                '"' => quote = None,
                '\\' => {
                    let escaped = chars
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Trailing escape in quoted argument"))?;
                    match escaped {
                        '$' | '`' | '"' | '\\' => token.push(escaped),
                        '\n' => {}
                        other => {
                            // POSIX double quotes preserve a backslash unless
                            // it precedes a character that can be escaped.
                            token.push('\\');
                            token.push(other);
                        }
                    }
                }
                _ => token.push(ch),
            },
            None => match ch {
                ch if ch.is_whitespace() => {
                    if token_started {
                        tokens.push(std::mem::take(&mut token));
                        token_started = false;
                    }
                }
                '\'' => {
                    token_started = true;
                    quote = Some(Quote::Single);
                }
                '"' => {
                    token_started = true;
                    quote = Some(Quote::Double);
                }
                '\\' => {
                    token_started = true;
                    let escaped = chars
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Trailing escape in argument list"))?;
                    if escaped != '\n' {
                        token.push(escaped);
                    }
                }
                _ => {
                    token_started = true;
                    token.push(ch);
                }
            },
        }
    }

    if quote.is_some() {
        return Err(anyhow::anyhow!("Unclosed quote in argument list"));
    }
    if token_started {
        tokens.push(token);
    }

    Ok(tokens)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandArgs {
    pub cmd: Option<String>,
    pub argv: Option<Vec<String>>,
    pub chdir: Option<String>,
    pub creates: Option<String>,
    pub removes: Option<String>,
    pub stdin: Option<String>,
    #[serde(default = "yes")]
    pub stdin_add_newline: bool,
    #[serde(default = "yes")]
    pub strip_empty_ends: bool,
    #[serde(default = "yes")]
    pub expand_argument_vars: bool,
}
fn yes() -> bool {
    true
}
impl CommandArgs {
    pub fn parse(args: &Value, shell: bool) -> Result<Self> {
        let args = if let Value::String(command) = args {
            serde_yaml::to_value(std::collections::BTreeMap::from([("cmd", command)]))?
        } else {
            args.clone()
        };
        let parsed: Self = serde_yaml::from_value(args)?;
        if parsed.cmd.is_some() == parsed.argv.is_some() {
            anyhow::bail!("Command requires exactly one of cmd or argv");
        }
        if shell && parsed.argv.is_some() {
            anyhow::bail!("Shell does not accept argv");
        }
        if parsed
            .cmd
            .as_ref()
            .is_some_and(|cmd| cmd.trim().is_empty() || cmd.contains('\0'))
        {
            anyhow::bail!("Command must be nonempty without NUL bytes");
        }
        if !shell {
            let argv = parsed.arguments()?;
            if argv.first().is_none_or(String::is_empty)
                || argv.iter().any(|arg| arg.contains('\0'))
            {
                anyhow::bail!(
                    "Command requires a nonempty executable and arguments without NUL bytes"
                );
            }
        }
        for path in [&parsed.chdir, &parsed.creates, &parsed.removes]
            .into_iter()
            .flatten()
        {
            if path.is_empty() || path.contains('\0') {
                anyhow::bail!("Command paths must be nonempty without NUL bytes");
            }
        }
        Ok(parsed)
    }
    pub fn arguments(&self) -> Result<Vec<String>> {
        if let Some(argv) = &self.argv {
            Ok(argv.clone())
        } else {
            tokenize_args(self.cmd.as_deref().unwrap_or_default())
        }
    }
    pub fn has_prediction(&self) -> bool {
        self.creates.is_some() || self.removes.is_some()
    }
}
