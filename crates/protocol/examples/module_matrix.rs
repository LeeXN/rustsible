//! Render the public parameter/capability table from the shared registry.
use rustsible_protocol::registry::{Builtin, CheckSupport};
fn main() {
    println!("| 模块 | 参数 | check | diff | async |\n|---|---|---|---|---|");
    for module in Builtin::ALL {
        let parameters = module
            .parameters()
            .iter()
            .filter(|name| !name.starts_with('_'))
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        let check = match module.check_support() {
            CheckSupport::ReadOnly => "只读",
            CheckSupport::Predict => "预测",
        };
        println!(
            "| {} | {} | {} | {} | {} |",
            module.name(),
            parameters,
            check,
            if module.supports_diff() {
                "支持"
            } else {
                "—"
            },
            if module.supports_async() {
                "支持"
            } else {
                "—"
            }
        );
    }
}
