[English / 简体中文](../../zh-CN/guides/files-network.md) · [Index / 目录](../index.md)

# Files, downloads and HTTP

<a id="purpose"></a>

## Before you start

copy/template/get_url/lineinfile stage content then atomically replace regular files. Unspecified mode/owner/group and existing protection metadata are retained; unsafe replacements fail instead of truncating the original. Quote octal modes.

<a id="usage"></a>

## Usage

```bash
rustsible ad-hoc web -i inventory.yml -m stat -a 'path=/etc/passwd checksum_algorithm=sha256'
```

<a id="behavior"></a>

## Behavior and boundaries

get_url downloads at the target and streams large files. force:false keeps existing content unless a supplied checksum differs. uri is for bounded HTTP request/response data, not large downloads; TLS and proxy use target settings. wait_for supports TCP, file presence/absence and text matching. uri/wait_for skip in check. Git needs the target Git tool; unarchive decodes tar/gzip/ZIP natively and rejects escaping paths and links.

<a id="reference"></a>

## Reference

See the [module index](../reference/modules.md), [configuration](../reference/configuration.md), [keywords](../reference/keywords.md) and [results](../reference/results.md).
