use anyhow::Result;

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
