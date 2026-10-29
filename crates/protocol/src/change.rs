thread_local! { static CHANGE_STATE: std::cell::Cell<Option<(bool, bool)>> = const { std::cell::Cell::new(None) }; }

/// Invocation-local accounting avoids mixing hosts on concurrent/reused
/// threads. Only explicitly mutating, successful operations record a change.
pub struct ChangeScope(Option<(bool, bool)>);
impl ChangeScope {
    pub fn enter(check: bool) -> Self {
        let prior = CHANGE_STATE.get();
        let check = check || prior.is_some_and(|(check, _)| check);
        CHANGE_STATE.set(Some((check, false)));
        Self(prior)
    }
}
impl Drop for ChangeScope {
    fn drop(&mut self) {
        let changed = known_changed();
        CHANGE_STATE.set(self.0.map(|(check, prior)| (check, prior || changed)));
    }
}
pub fn is_check_active() -> bool {
    CHANGE_STATE.get().is_some_and(|(check, _)| check)
}
pub fn known_changed() -> bool {
    CHANGE_STATE.get().is_some_and(|(_, changed)| changed)
}
pub fn ensure_mutation_allowed() -> anyhow::Result<()> {
    if CHANGE_STATE.get().is_some_and(|(check, _)| check) {
        anyhow::bail!("Check mode forbids target mutations");
    }
    Ok(())
}
pub fn record_change() {
    CHANGE_STATE.set(CHANGE_STATE.get().map(|(check, _)| (check, true)));
}
