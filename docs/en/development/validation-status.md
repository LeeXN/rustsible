[English / 简体中文](../../zh-CN/development/validation-status.md) · [Index / 目录](../index.md)

# Delivery and validation records

<a id="baseline"></a>

## Functional baseline

The 22-step Linux/native-execution plan was delivered through independent PRs. Native collections/extensions landed in PR23 (CI40978538184,8 successful jobs). Final package acceptance landed in PR24 (source48c3f05a044f769691b7fc731ad6464a545b72d6, CI40985150671,9 successful jobs). The documentation reorganization adds its own checks without redefining that historical source identity.

<a id="evidence"></a>

## Read the evidence

[PR23](https://github.com/LeeXN/rustsible/pull/23), [PR24](https://github.com/LeeXN/rustsible/pull/24), [final CI](https://github.com/LeeXN/rustsible/actions/runs/40985150671). The artifacts include both controller packages, each with both workers, plus a native collection. Four native glibc/musl cases retain12 target receipts; cross-controller cases retain separate records. A corrected backup disables stdout newline trimming and validates the complete configuration digest after rollback.

<a id="limits"></a>

## Evidence boundaries

Use the current [compatibility matrix](../reference/compatibility.md), not a broad claim of complete Ansible equivalence. DNF/Yum/Pacman/Zypper and Upstart have x86-only real validation. Existing unsafe-libyaml maintenance advisory is recorded by cargo audit; do not add blanket ignores to make gates green. New runtime behavior must add its own representative target-state assertions.
