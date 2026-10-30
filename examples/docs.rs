//! Source-aware bilingual documentation generation, verification and packaging.
//! Runtime commands are never invoked by generation or link checks.
use anyhow::{bail, Context, Result};
use regex::Regex;
use rustsible_protocol::registry::Builtin;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
const LANGS: [&str; 2] = ["en", "zh-CN"];
fn files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            bail!(
                "Documentation resources cannot be symlinks: {}",
                entry.path().display()
            )
        }
        if kind.is_dir() {
            files(&entry.path(), out)?
        } else if kind.is_file() {
            out.push(entry.path())
        }
    }
    out.sort();
    Ok(())
}
fn parameters(module: Builtin) -> Vec<String> {
    if module == Builtin::SetFact {
        return vec!["cacheable".into(), "<variable>".into()];
    }
    module
        .parameters()
        .iter()
        .filter(|p| !p.starts_with('_') && !(module == Builtin::Shell && **p == "argv"))
        .map(|p| p.to_string())
        .collect()
}
fn header(path: &str, lang: &str, title: &str) -> String {
    let depth = path.matches('/').count();
    let up = "../".repeat(depth);
    let other = if lang == "en" { "zh-CN" } else { "en" };
    format!(
        "[English / 简体中文]({up}../{other}/{path}) · [Index / 目录]({up}index.md)\n\n# {title}\n"
    )
}
fn translation(text: &str, lang: &str, map: &BTreeMap<String, String>) -> Result<String> {
    if lang == "en" || text.is_empty() {
        return Ok(text.to_owned());
    }
    map.get(text)
        .cloned()
        .with_context(|| format!("Missing Chinese CLI translation: {text}"))
}
fn supplemental(path: &str, id: &str) -> &'static str {
    match id {
        "input" if path == "rustsible collection pack" => "Collection source directory",
        "input" if path.starts_with("rustsible collection") => {
            "Collection archive to verify or install"
        }
        "input" => "Vault input file, or - for standard input",
        "output" => "New collection archive path; existing files are not overwritten",
        "path" => "Collection installation root; custom roots also need search-path configuration",
        "encrypt-vault-id" => "Choose the encryption label and matching password source",
        "new-vault-password-file" => "New password file for rekey; repeatable",
        "new-vault-id" => "New rekey password source: ID@FILE or ID@prompt; repeatable",
        _ => "",
    }
}
fn cli(
    command: &clap::Command,
    path: &str,
    lang: &str,
    map: &BTreeMap<String, String>,
    out: &mut String,
) -> Result<()> {
    let id = path.replace(' ', "-");
    out.push_str(&format!("\n<a id=\"{id}\"></a>\n\n## `{path}`\n\n"));
    out.push_str(&translation(
        &command
            .get_about()
            .map(ToString::to_string)
            .unwrap_or_default(),
        lang,
        map,
    )?);
    let mut copy = command.clone();
    out.push_str(&format!("\n\n```text\n{}\n```\n\n", copy.render_usage()));
    out.push_str(if lang == "en" {
        "| Argument | Required | Default | Meaning |\n|---|---|---|---|\n"
    } else {
        "| 参数 | 必填 | 默认值 | 含义 |\n|---|---|---|---|\n"
    });
    for arg in command.get_arguments().filter(|arg| !arg.is_hide_set()) {
        let name = match (arg.get_short(), arg.get_long()) {
            (Some(s), Some(l)) => format!("-{s}, --{l}"),
            (Some(s), None) => format!("-{s}"),
            (None, Some(l)) => format!("--{l}"),
            _ => arg.get_id().to_string(),
        };
        let desc = translation(
            &arg.get_help()
                .map(ToString::to_string)
                .unwrap_or_else(|| supplemental(path, arg.get_id().as_str()).into()),
            lang,
            map,
        )?
        .replace('|', "&#124;");
        let default = arg
            .get_default_values()
            .iter()
            .map(|v| v.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "| `{name}` | {} | {} | {desc} |\n",
            arg.is_required_set(),
            if default.is_empty() { "—" } else { &default }
        ));
    }
    for sub in command.get_subcommands().filter(|s| s.get_name() != "help") {
        cli(sub, &format!("{path} {}", sub.get_name()), lang, map, out)?
    }
    Ok(())
}
fn generate(root: &Path, write: bool) -> Result<()> {
    let map: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("docs/cli-translations.json"))?)?;
    let mut command = rustsible::cli::build_cli()
        .disable_colored_help(true)
        .term_width(100);
    command.build();
    for lang in LANGS {
        let mut body = header(
            "reference/cli.md",
            lang,
            if lang == "en" {
                "Command line reference"
            } else {
                "命令行参考"
            },
        );
        body.push_str(if lang=="en"{"\nGenerated from Clap. Defaults shown here are parser defaults; configuration and connection variables can override applicable defaults. Use the configuration and variable references for effective precedence. Commands/identifiers remain untranslated. Dry-run permits temporary worker staging but forbids business mutation; syntax/list are controller-only.\n"}else{"\n由 Clap 生成。表中默认值为参数解析默认值，适用的配置和连接变量可以覆盖它们；有效优先级见配置与变量参考。命令和标识符不翻译。预演允许临时 worker 引导，禁止业务修改；语法和列表检查仅在控制端进行。\n"});
        cli(&command, "rustsible", lang, &map, &mut body)?;
        generated(
            &root.join("docs").join(lang).join("reference/cli.md"),
            &body,
            write,
        )?;
        let mut body = header(
            "reference/modules.md",
            lang,
            if lang == "en" {
                "Builtin module catalog"
            } else {
                "内置模块目录"
            },
        );
        body.push_str(if lang=="en"{"\nThe effective validator takes precedence over a shared parameter superset. Check may predict, query or skip; see each page.\n\n| Module | Public parameters | Check | Diff | Async |\n|---|---|---|---|---|\n"}else{"\n以实际校验器为准，不直接把共同参数超集视为可用接口。check 可以预测、查询或跳过，详见各页。\n\n| 模块 | 公开参数 | Check | Diff | Async |\n|---|---|---|---|---|\n"});
        for module in Builtin::ALL {
            let check = if matches!(module, Builtin::Uri | Builtin::WaitFor) {
                "skip"
            } else if matches!(module, Builtin::Command | Builtin::Shell) {
                "guarded"
            } else {
                match module.check_support() {
                    rustsible_protocol::registry::CheckSupport::ReadOnly => "readonly",
                    _ => "prediction",
                }
            };
            body.push_str(&format!(
                "| [{}](modules/{}.md) | {} | {check} | {} | {} |\n",
                module.name(),
                module.name(),
                parameters(module)
                    .iter()
                    .map(|parameter| format!("`{parameter}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                module.supports_diff(),
                module.supports_async()
            ));
        }
        generated(
            &root.join("docs").join(lang).join("reference/modules.md"),
            &body,
            write,
        )?;
        let mut pages = Vec::new();
        files(&root.join("docs").join(lang), &mut pages)?;
        let mut body = header(
            "index.md",
            lang,
            if lang == "en" {
                "Rustsible handbook"
            } else {
                "Rustsible 使用与开发手册"
            },
        );
        body.push_str(if lang=="en"{"\nStart with installation and the local tutorial. Guides explain tasks; references define the supported interface; development covers extension authors and contributors. Module pages contain complete task examples and parameter constraints.\n"}else{"\n从安装和本地教程开始。操作指南说明任务用法，参考手册定义支持的接口，开发文档面向扩展作者和贡献者；模块页含完整示例和参数约束。\n"});
        for group in ["tutorials", "guides", "reference", "development"] {
            body.push_str(&format!("\n<a id=\"{group}\"></a>\n\n## {group}\n\n"));
            for p in pages
                .iter()
                .filter(|p| p.extension().is_some_and(|s| s == "md"))
            {
                let relative = p.strip_prefix(root.join("docs").join(lang))?;
                if relative
                    .components()
                    .next()
                    .is_some_and(|c| c.as_os_str() == group)
                    && relative.components().count() == 2
                {
                    let text = fs::read_to_string(p)?;
                    let title = text
                        .lines()
                        .find_map(|l| l.strip_prefix("# "))
                        .context("Page needs title")?;
                    body.push_str(&format!("- [{title}]({})\n", relative.display()));
                }
            }
        }
        generated(&root.join("docs").join(lang).join("index.md"), &body, write)?;
    }
    Ok(())
}
fn generated(path: &Path, body: &str, write: bool) -> Result<()> {
    if write {
        fs::write(path, body)?
    } else if fs::read_to_string(path).ok().as_deref() != Some(body) {
        bail!("Stale generated page: {}; run docs --write", path.display())
    }
    Ok(())
}
fn without_fences(text: &str) -> String {
    let mut fenced = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            lines.push(line)
        }
    }
    lines.join("\n")
}
fn blocks(text: &str) -> Result<Vec<String>> {
    let mut result = Vec::new();
    let mut current = None;
    for line in text.lines() {
        if line.starts_with("```") {
            if let Some(block) = current.take() {
                result.push(block)
            } else {
                current = Some(line.to_owned())
            }
        } else if let Some(block) = current.as_mut() {
            block.push('\n');
            block.push_str(line)
        }
    }
    if current.is_some() {
        bail!("Unclosed Markdown code fence")
    }
    Ok(result)
}
fn peer(root: &Path, path: &Path) -> Result<PathBuf> {
    let rel = path.strip_prefix(root)?;
    let value = rel.to_string_lossy();
    if let Some(rest) = value.strip_prefix("docs/en/") {
        return Ok(root.join("docs/zh-CN").join(rest));
    }
    if let Some(rest) = value.strip_prefix("docs/zh-CN/") {
        return Ok(root.join("docs/en").join(rest));
    }
    let name = path
        .file_name()
        .context("Missing Markdown filename")?
        .to_string_lossy();
    Ok(path.with_file_name(if name.ends_with(".zh-CN.md") {
        name.replace(".zh-CN.md", ".md")
    } else {
        name.replace(".md", ".zh-CN.md")
    }))
}
fn check(root: &Path, offline: bool) -> Result<()> {
    let mut all = Vec::new();
    for dir in ["docs", "examples"] {
        files(&root.join(dir), &mut all)?
    }
    for name in ["README.md", "README.zh-CN.md"] {
        all.push(root.join(name))
    }
    if !offline {
        files(&root.join("tests/fixtures/tls"), &mut all)?
    }
    let links = Regex::new(r"\[[^\]\n]*\]\(([^\s)]+)\)")?;
    let anchors = Regex::new(r#"<a id="([^"]+)""#)?;
    for path in all
        .iter()
        .filter(|p| p.extension().is_some_and(|v| v == "md"))
    {
        let text = fs::read_to_string(path)?;
        let other = peer(root, path)?;
        let other_text = fs::read_to_string(&other)
            .with_context(|| format!("Missing language peer: {}", other.display()))?;
        if blocks(&text)? != blocks(&other_text)? {
            bail!("Bilingual executable blocks diverge: {}", path.display())
        }
        let ids = |s: &str| {
            anchors
                .captures_iter(s)
                .map(|c| c[1].to_string())
                .collect::<Vec<_>>()
        };
        if ids(&text) != ids(&other_text) {
            bail!("Bilingual section IDs diverge: {}", path.display())
        }
        if ids(&text).len() != ids(&text).into_iter().collect::<BTreeSet<_>>().len() {
            bail!("Duplicate section ID: {}", path.display())
        }
        let plain = without_fences(&text);
        if !links.captures_iter(&plain).any(|link| {
            path.parent()
                .unwrap()
                .join(link[1].split('#').next().unwrap())
                .canonicalize()
                .ok()
                == other.canonicalize().ok()
        }) {
            bail!("Missing language switch in {}", path.display())
        }
        for link in links.captures_iter(&plain) {
            let target = &link[1];
            if target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            let (file, anchor) = target.split_once('#').unwrap_or((target, ""));
            let target = if file.is_empty() {
                path.to_owned()
            } else {
                path.parent().context("Missing parent")?.join(file)
            };
            if !target.exists() {
                bail!("Broken link in {}: {}", path.display(), target.display())
            }
            if !target.canonicalize()?.starts_with(root.canonicalize()?) {
                bail!("Link escapes documentation root: {}", path.display())
            }
            if !anchor.is_empty()
                && target.extension().is_some_and(|p| p == "md")
                && !ids(&fs::read_to_string(&target)?)
                    .iter()
                    .any(|id| id == anchor)
            {
                bail!("Unknown anchor {anchor} in {}", path.display())
            }
        }
    }
    if !offline {
        let example =
            fs::read_to_string(root.join("crates/native-example/examples/minimal_module.rs"))?;
        let actual = example
            .split_once("use anyhow")
            .context("Minimal SDK source needs its import")?;
        let snippet = format!("use anyhow{}", actual.1);
        let guide = fs::read_to_string(root.join("docs/en/development/native-extensions.md"))?;
        if !guide.contains(snippet.trim()) {
            bail!("SDK snippet drifted from its compiled source")
        }
    }
    let parameter_rows = Regex::new(r"(?m)^\| `([^`]+)` \|")?;
    for lang in LANGS {
        for module in Builtin::ALL {
            let path = root
                .join("docs")
                .join(lang)
                .join("reference/modules")
                .join(format!("{}.md", module.name()));
            let text = fs::read_to_string(&path)?;
            let expected = format!("<!-- parameters: {} -->", parameters(module).join(","));
            let rows = parameter_rows
                .captures_iter(&text)
                .map(|c| c[1].to_string())
                .collect::<Vec<_>>();
            if rows != parameters(module) {
                bail!(
                    "Module table does not match public parameters: {}",
                    path.display()
                )
            }
            if !text.contains(&expected) {
                bail!("Module parameter coverage is stale: {}", path.display())
            }
            for id in [
                "purpose",
                "parameters",
                "behavior",
                "returns",
                "examples",
                "verification",
            ] {
                if !text.contains(&format!("id=\"{id}\"")) {
                    bail!("Incomplete module page: {}", path.display())
                }
            }
        }
    }
    println!(
        "Documentation verified: language peers, section IDs, code blocks, links and 25 modules"
    );
    Ok(())
}
fn package(root: &Path, output: &Path) -> Result<()> {
    let source = root.canonicalize()?;
    let destination = output.canonicalize()?;
    if destination == source
        || destination.starts_with(source.join("docs"))
        || destination.starts_with(source.join("examples"))
    {
        bail!("Package output must not replace documentation sources")
    }
    let release: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("RELEASE.json"))?)?;
    let commit = release["source_commit"]
        .as_str()
        .context("Missing release source identity")?;
    let mut paths = Vec::new();
    for dir in ["docs", "examples"] {
        files(&root.join(dir), &mut paths)?
    }
    for name in ["README.md", "README.zh-CN.md", "LICENSE"] {
        paths.push(root.join(name))
    }
    for source in paths {
        if source.extension().is_some_and(|e| e == "rs") {
            continue;
        }
        let relative = source.strip_prefix(root)?;
        let dest = output.join(relative);
        fs::create_dir_all(dest.parent().context("Missing destination parent")?)?;
        if source.extension().is_some_and(|e| e == "md") {
            let text = fs::read_to_string(source)?
                .replace(
                    "https://github.com/LeeXN/rustsible/blob/main/",
                    &format!("https://github.com/LeeXN/rustsible/blob/{commit}/"),
                )
                .replace(
                    "https://github.com/LeeXN/rustsible/tree/main/",
                    &format!("https://github.com/LeeXN/rustsible/tree/{commit}/"),
                );
            fs::write(dest, text)?;
        } else {
            fs::copy(source, dest)?;
        }
    }
    check(output, true)
}
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [mode] if mode == "--write" => generate(&root, true),
        [mode] if mode == "--check" => {
            generate(&root, false)?;
            check(&root, false)
        }
        [mode, path] if mode == "--package" => {
            generate(&root, false)?;
            package(&root, Path::new(path))
        }
        _ => bail!("Usage: docs --write | --check | --package DIRECTORY"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_handbook_is_closed_and_detects_corruption() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        fs::write(
            temporary.path().join("RELEASE.json"),
            r#"{"source_commit":"documentation-test"}"#,
        )?;
        package(root, temporary.path())?;
        assert!(!temporary.path().join("examples/prompt_fixture.rs").exists());
        assert!(fs::read_to_string(
            temporary
                .path()
                .join("docs/en/development/native-extensions.md")
        )?
        .contains("/tree/documentation-test/"));
        let page = temporary.path().join("docs/en/tutorials/local.md");
        let original = fs::read_to_string(&page)?;
        fs::write(&page, format!("{original}\n[missing](nonexistent.md)\n"))?;
        assert!(check(temporary.path(), true).is_err());
        fs::write(&page, &original)?;
        fs::remove_file(temporary.path().join("docs/zh-CN/tutorials/local.md"))?;
        assert!(check(temporary.path(), true).is_err());
        Ok(())
    }
}
