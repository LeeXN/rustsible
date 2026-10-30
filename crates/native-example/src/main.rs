fn main() -> anyhow::Result<()> {
    rustsible_protocol::extension::serve(
        &rustsible_native_example::catalog(),
        rustsible_native_example::handle,
    )
}
