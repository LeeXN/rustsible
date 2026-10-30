[English / 简体中文](../../zh-CN/development/embedding.md) · [Index / 目录](../index.md)

# Compile-time Rust interfaces

<a id="registration"></a>

## Register before execution

Use Registry::new(...).with_compiled(Components{...}) before sharing the Arc; pass it through ExecutionOptions.extensions or AdHocOptions.extensions. Real integration is demonstrated by [compiled_extensions](https://github.com/LeeXN/rustsible/blob/main/tests/compiled_extensions.rs). This is a Rust source interface, not a stable dynamic-library ABI.

<a id="interfaces"></a>

## Available boundaries

Action: validated controller function, no async/task timeout. Callback: payload-free host/module/outcome/changed/check completion events. Connection: factory plus bounded worker Session. Become: bootstrap command plus a separate Zeroizing stdin prefix; UID is still verified. Strategy: a valid selected-host permutation around linear/free, retaining serial/throttle/failure rules. These trusted functions must honor their own execution contract.

<a id="runner"></a>

## Executable providers in an embedded controller

At the entrypoint dispatch rustsible_native::rpc::handle_runner_mode() before normal argument/configuration work, matching the stock controller. This exec-based runner avoids allocating in a forked multithreaded controller. External inventory/lookup/filter share the same explicit installation and protocol checks.
