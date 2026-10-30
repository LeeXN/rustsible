//! Trusted components linked into a controller. This is a Rust source API,
//! never a dynamic-library ABI or permission to execute repository plugins.
use super::*;
use crate::{
    inventory::Host,
    modules::ModuleResult,
    playbook::Strategy,
    ssh::{config::ConnectionConfig, connection::Connection},
};

pub type ActionHandler = fn(&Call, &HashMap<String, serde_yaml::Value>) -> Result<ModuleResult>;
#[derive(Debug)]
pub struct Action {
    pub definition: Definition,
    pub execute: ActionHandler,
}
pub type ConnectionFactory = fn(&ConnectionConfig) -> Result<Connection>;
pub type BecomeHandler = fn(&ConnectionConfig, &str) -> Result<Bootstrap>;
/// Authentication travels through stdin before the worker image, never argv.
/// A provider must consume exactly these bytes before running the bootstrap.
pub struct Bootstrap {
    pub command: String,
    pub input: zeroize::Zeroizing<Vec<u8>>,
}
#[derive(Debug)]
pub struct Scheduling {
    pub engine: Strategy,
    pub order: fn(&[Host]) -> Result<Vec<usize>>,
}
/// Payload-free events deliberately exclude parameters, output and task names:
/// a callback cannot accidentally bypass no_log or Vault result censorship.
#[derive(Debug, Clone)]
pub struct Completion {
    pub host: String,
    pub module: String,
    pub outcome: rustsible_protocol::Outcome,
    pub changed: bool,
    pub check: bool,
}
#[derive(Debug, Default)]
pub struct Components {
    pub actions: BTreeMap<&'static str, Arc<Action>>,
    pub connections: BTreeMap<&'static str, ConnectionFactory>,
    pub become_providers: BTreeMap<&'static str, BecomeHandler>,
    pub strategies: BTreeMap<&'static str, Scheduling>,
    pub callbacks: Vec<fn(&Completion)>,
}
impl Components {
    pub(crate) fn validate(&self) -> Result<()> {
        for name in self
            .actions
            .keys()
            .chain(self.connections.keys())
            .chain(self.become_providers.keys())
            .chain(self.strategies.keys())
        {
            rustsible_protocol::extension::validate_name(name)?;
        }
        for (name, action) in &self.actions {
            action.definition.validate()?;
            if action.definition.name != *name
                || action.definition.kind != Kind::Module
                || action.definition.capabilities.asynchronous
            {
                bail!("Compiled actions require matching module metadata and cannot run asynchronously");
            }
        }
        if self
            .strategies
            .values()
            .any(|strategy| !matches!(strategy.engine, Strategy::Linear | Strategy::Free))
        {
            bail!("Compiled scheduling must select a builtin barrier engine");
        }
        Ok(())
    }
}
impl Registry {
    /// Configure before sharing the registry with execution threads. Components
    /// are immutable for a run, so changing a connection cannot reuse old leases.
    pub fn with_compiled(self: Arc<Self>, components: Components) -> Result<Arc<Self>> {
        components.validate()?;
        let mut registry = Arc::try_unwrap(self).map_err(|_| {
            anyhow::anyhow!("Configure compiled components before sharing the registry")
        })?;
        registry.compiled = components;
        Ok(Arc::new(registry))
    }
    pub fn components(&self) -> &Components {
        &self.compiled
    }
    pub(crate) fn notify(&self, event: &Completion) {
        for callback in &self.compiled.callbacks {
            // Reporting must never turn completed business work into a retry.
            if std::panic::catch_unwind(|| callback(event)).is_err() {
                log::error!("Compiled callback panicked after module completion");
            }
        }
    }
}
