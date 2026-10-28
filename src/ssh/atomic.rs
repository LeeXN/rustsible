use anyhow::{bail, Context, Result};
use std::path::Path;

use super::connection::{shell_quote, SshConnection};

#[derive(Clone, Copy, Default)]
pub(crate) struct FileMetadata<'a> {
    pub mode: Option<&'a str>,
    pub owner: Option<&'a str>,
    pub group: Option<&'a str>,
}

/// Keep all fallible preparation off the destination inode. GNU cp preserves
/// ACLs and extended attributes explicitly: unsupported metadata is an error,
/// rather than a successful deployment that silently removes a protection.
/// The private staging directory is on the destination filesystem. Resolving
/// its parent once also prevents ancestor symlink changes redirecting writes.
pub(crate) fn install_command(path: &str, metadata: FileMetadata<'_>) -> Result<String> {
    if path.is_empty() || path.contains('\0') {
        bail!("File destination must be a non-empty path without NUL bytes");
    }
    let path = Path::new(path);
    let name = path
        .file_name()
        .context("File destination needs a filename")?
        .to_str()
        .context("File destination must be UTF-8")?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_str()
        .context("File destination must be UTF-8")?;
    if let Some(mode) = metadata.mode {
        if mode.is_empty()
            || !mode.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
            || u32::from_str_radix(mode, 8).map_or(true, |mode| mode > 0o7777)
        {
            bail!("File mode must be an octal string between 0000 and 7777");
        }
    }
    for (kind, value) in [("owner", metadata.owner), ("group", metadata.group)] {
        if value.is_some_and(|value| {
            value.is_empty() || value.contains(':') || value.chars().any(char::is_control)
        }) {
            bail!("File {kind} must be a non-empty name or numeric ID without colons or control characters");
        }
    }
    let mut command = format!(
        r#"set -eu
cd -- {parent}
target={target}
snapshot() {{
    if [ -L "$target" ]; then echo 'Refusing to replace a symbolic link' >&2; return 1; fi
    if [ -e "$target" ]; then
        [ -f "$target" ] || {{ echo 'Destination is not a regular file' >&2; return 1; }}
        stat -c '%d:%i:%f:%u:%g:%s:%y:%z' -- "$target"
    else
        printf absent
    fi
}}
original=$(snapshot)
"#,
        parent = shell_quote(parent),
        target = shell_quote(&format!("./{name}"))
    );
    for (variable, value, lookup) in [
        ("uid", metadata.owner, "id -u --"),
        ("gid", metadata.group, "getent group"),
    ] {
        if let Some(value) = value {
            if value.bytes().all(|byte| byte.is_ascii_digit()) {
                command.push_str(&format!("{variable}={}\n", shell_quote(value)));
            } else if variable == "uid" {
                command.push_str(&format!("uid=$({lookup} {})\n", shell_quote(value)));
            } else {
                command.push_str(&format!("gid=$({lookup} {} | cut -d: -f3)\n[ -n \"$gid\" ] || {{ echo 'Unknown file group' >&2; exit 1; }}\n", shell_quote(value)));
            }
        }
    }
    command.push_str(
        r#"umask 077
staging=$(mktemp -d ./.rustsible-write.XXXXXXXXXX)
trap 'rm -rf -- "$staging"' EXIT
trap 'exit 1' HUP INT TERM
temporary=$staging/content
cat > "$temporary"
if [ "$original" != absent ]; then
    cp --attributes-only --preserve=mode,ownership,xattr --dereference -- "$target" "$temporary"
else
    chmod 0644 -- "$temporary"
fi
"#,
    );
    match (metadata.owner, metadata.group) {
        (Some(_), Some(_)) => command.push_str("chown -- \"$uid:$gid\" \"$temporary\"\n"),
        (Some(_), None) => command.push_str("chown -- \"$uid\" \"$temporary\"\n"),
        (None, Some(_)) => command.push_str("chown -- \":$gid\" \"$temporary\"\n"),
        (None, None) => {}
    }
    if let Some(mode) = metadata.mode {
        command.push_str(&format!("chmod -- {} \"$temporary\"\n", shell_quote(mode)));
    }
    command.push_str(
        r#"sync -d -- "$temporary"
[ "$(snapshot)" = "$original" ] || { echo 'Destination changed during preparation' >&2; exit 1; }
mv -T -- "$temporary" "$target"
"#,
    );
    Ok(command)
}

pub(crate) fn install(
    connection: &dyn SshConnection,
    path: &str,
    content: &[u8],
    become_user: Option<&str>,
    metadata: FileMetadata<'_>,
) -> Result<()> {
    let command = install_command(path, metadata)?;
    let (code, _, stderr) = if let Some(user) = become_user {
        connection.execute_sudo_command_with_input(&command, user, content)?
    } else {
        connection.execute_command_with_input(&command, content)?
    };
    if code != 0 {
        bail!(
            "Atomic file installation failed (exit {code}): {}",
            stderr.trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Host;
    use crate::ssh::connection::{LocalConnection, MockSshConnection};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    #[test]
    fn replacement_preserves_protections_and_cleans_staging() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        std::fs::write(&path, b"original").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let before = std::fs::metadata(&path).unwrap();
        crate::testing::test_xattr(&path, "user.rustsible-protection", Some(b"keep-protection"));
        // Linux POSIX ACL: an additional named user with read access. The
        // numeric identity need not exist in passwd for an ACL to protect it.
        let mut acl = 2u32.to_le_bytes().to_vec();
        for (tag, permission, id) in [
            (1u16, 7u16, u32::MAX),
            (2, 4, 1001),
            (4, 5, u32::MAX),
            (16, 5, u32::MAX),
            (32, 5, u32::MAX),
        ] {
            acl.extend(tag.to_le_bytes());
            acl.extend(permission.to_le_bytes());
            acl.extend(id.to_le_bytes());
        }
        crate::testing::test_xattr(&path, "system.posix_acl_access", Some(&acl));
        let connection = LocalConnection::new(&Host::new("localhost")).unwrap();
        install(
            &connection,
            path.to_str().unwrap(),
            &[0, 255, 42],
            None,
            FileMetadata::default(),
        )
        .unwrap();
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), [0, 255, 42]);
        assert_eq!(after.mode(), before.mode());
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
        assert_ne!(after.ino(), before.ino());
        assert_eq!(
            crate::testing::test_xattr(&path, "user.rustsible-protection", None),
            b"keep-protection"
        );
        assert_eq!(
            crate::testing::test_xattr(&path, "system.posix_acl_access", None),
            acl
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn interrupted_write_keeps_original_bytes_and_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        std::fs::write(&path, b"original").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
        let before = std::fs::metadata(&path).unwrap();
        let command = install_command(path.to_str().unwrap(), FileMetadata::default()).unwrap();
        let connection = LocalConnection::new(&Host::new("localhost")).unwrap();
        // The OS enforces the write limit on the child; no test hook can
        // accidentally bypass the real streaming/write failure path.
        let (code, _, _) = connection
            .execute_command_with_input(
                &format!("ulimit -f 1; sh -c {}", shell_quote(&command)),
                &[42; 128 * 1024],
            )
            .unwrap();
        assert_ne!(code, 0);
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(
            (after.ino(), after.mode(), after.uid(), after.gid()),
            (before.ino(), before.mode(), before.uid(), before.gid())
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn invalid_metadata_and_symlinks_do_not_change_targets() {
        let connection = LocalConnection::new(&Host::new("localhost")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        std::fs::write(&path, b"original").unwrap();
        for metadata in [
            FileMetadata {
                mode: Some("888"),
                ..FileMetadata::default()
            },
            FileMetadata {
                owner: Some("missing-rustsible-account-99999"),
                ..FileMetadata::default()
            },
            FileMetadata {
                group: Some("missing-rustsible-group-99999"),
                ..FileMetadata::default()
            },
            FileMetadata {
                owner: Some("root:root"),
                ..FileMetadata::default()
            },
        ] {
            assert!(install(
                &connection,
                path.to_str().unwrap(),
                b"replacement",
                None,
                metadata
            )
            .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
        }
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(install(
            &connection,
            link.to_str().unwrap(),
            b"replacement",
            None,
            FileMetadata::default()
        )
        .is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"original");
        assert!(link.is_symlink());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn target_swapped_during_preparation_is_rejected_without_following_link() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let protected = directory.path().join("protected");
        std::fs::write(&target, b"original").unwrap();
        std::fs::write(&protected, b"protected").unwrap();
        let tools = directory.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        let cp = tools.join("cp");
        // Replace the pathname after the real attribute copy, mimicking a
        // concurrent writer between inspection and commit.
        std::fs::write(
            &cp,
            format!(
                "#!/bin/sh\n/usr/bin/cp \"$@\" || exit\nrm -- {}\nln -s -- {} {}\n",
                shell_quote(target.to_str().unwrap()),
                shell_quote(protected.to_str().unwrap()),
                shell_quote(target.to_str().unwrap())
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cp, std::fs::Permissions::from_mode(0o755)).unwrap();
        let connection = LocalConnection::new(&Host::new("localhost")).unwrap();
        let command = install_command(target.to_str().unwrap(), FileMetadata::default()).unwrap();
        let (code, _, _) = connection
            .execute_command_with_input(
                &format!(
                    "PATH={}:$PATH sh -c {}",
                    shell_quote(tools.to_str().unwrap()),
                    shell_quote(&command)
                ),
                b"replacement",
            )
            .unwrap();
        assert_ne!(code, 0);
        assert!(target.is_symlink());
        assert_eq!(std::fs::read(protected).unwrap(), b"protected");
        assert!(!std::fs::read_dir(directory.path())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rustsible-")));
    }

    #[test]
    fn privileged_install_passes_binary_data_as_stdin_to_selected_identity() {
        let mut connection = MockSshConnection::new();
        connection
            .expect_execute_sudo_command_with_input()
            .once()
            .withf(|command, user, input| {
                user == "deploy" && input == [0, 255] && command.contains("mktemp -d")
            })
            .returning(|_, _, _| Ok((0, String::new(), String::new())));
        install(
            &connection,
            "/srv/file",
            &[0, 255],
            Some("deploy"),
            FileMetadata::default(),
        )
        .unwrap();
    }
}
