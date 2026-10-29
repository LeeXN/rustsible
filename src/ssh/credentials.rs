//! Prompted secrets stay out of variable contexts, argv and configuration files.
//! They are only overlaid onto an ephemeral connection snapshot in the runtime.
use anyhow::{bail, Context, Result};
use std::{fs::OpenOptions, os::fd::AsRawFd};
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    ssh: Option<String>,
    escalation: Option<String>,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("ssh_prompted", &self.ssh.is_some())
            .field("become_prompted", &self.escalation.is_some())
            .finish()
    }
}
impl Credentials {
    pub fn read(ssh: bool, escalation: bool) -> Result<Self> {
        Ok(Self {
            ssh: ssh.then(|| prompt("SSH password: ")).transpose()?,
            escalation: escalation
                .then(|| prompt("BECOME password: "))
                .transpose()?,
        })
    }
    pub(crate) fn apply(&self, config: &mut super::config::ConnectionConfig) {
        if let Some(password) = self
            .ssh
            .as_ref()
            .filter(|_| config.get_ssh_password().is_none())
        {
            config.set_private("ansible_password", password.clone());
        }
        if let Some(password) = self
            .escalation
            .as_ref()
            .filter(|_| config.get_ssh_sudo_password().is_none())
        {
            config.set_private("ansible_become_password", password.clone());
        }
    }
}
pub(crate) fn prompt(label: &str) -> Result<String> {
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context(
        "Password prompt requires a controlling terminal; non-interactive input is not accepted",
    )?;
    let fd = tty.as_raw_fd();
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
        bail!("Cannot read terminal settings for password prompt");
    }
    let mut previous = unsafe { std::mem::zeroed::<libc::sigaction>() };
    let mut ignored = unsafe { std::mem::zeroed::<libc::sigaction>() };
    ignored.sa_sigaction = libc::SIG_IGN;
    if unsafe { libc::sigaction(libc::SIGINT, &ignored, &mut previous) } != 0 {
        bail!("Cannot protect terminal restoration during password prompt");
    }
    struct Restore {
        fd: i32,
        terminal: libc::termios,
        signal: libc::sigaction,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &self.terminal);
                libc::sigaction(libc::SIGINT, &self.signal, std::ptr::null_mut());
            }
        }
    }
    // rpassword raises SIGINT before returning its Ctrl-C error. Delay that
    // signal's default termination so both library and outer guards restore
    // echo first; the interrupted prompt then aborts command setup.
    let _restore = Restore {
        fd,
        terminal: original,
        signal: previous,
    };
    let password =
        rpassword::prompt_password(label).context("Password prompt failed or was interrupted")?;
    if password.is_empty() {
        bail!("Password prompt returned an empty password");
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompted_credentials_are_redacted_and_declared_variables_keep_precedence() {
        let credentials = Credentials {
            ssh: Some("prompt-ssh-canary".into()),
            escalation: Some("prompt-sudo-canary".into()),
        };
        assert!(!format!("{credentials:?}").contains("canary"));
        let mut config = super::super::config::ConnectionConfig::from_context(
            "target",
            std::collections::HashMap::new(),
        )
        .unwrap();
        credentials.apply(&mut config);
        assert_eq!(config.get_ssh_password(), Some("prompt-ssh-canary"));
        assert_eq!(config.get_ssh_sudo_password(), Some("prompt-sudo-canary"));
        let mut declared = super::super::config::ConnectionConfig::from_context(
            "target",
            std::collections::HashMap::from([
                (
                    "ansible_password".into(),
                    serde_yaml::Value::String("declared-ssh".into()),
                ),
                (
                    "ansible_become_password".into(),
                    serde_yaml::Value::String("declared-sudo".into()),
                ),
            ]),
        )
        .unwrap();
        credentials.apply(&mut declared);
        assert_eq!(declared.get_ssh_password(), Some("declared-ssh"));
        assert_eq!(declared.get_ssh_sudo_password(), Some("declared-sudo"));
    }
}
