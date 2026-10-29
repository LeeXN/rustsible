use anyhow::Result;

pub(crate) fn quote_posix_shell_arg(value: &str) -> Result<String> {
    if value.contains('\0') {
        return Err(anyhow::anyhow!("Shell arguments cannot contain NUL bytes"));
    }

    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}
