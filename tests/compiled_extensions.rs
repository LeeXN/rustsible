mod support;
use anyhow::Result;
use rustsible::{
    extensions::{compiled::*, Registry},
    inventory::{parse as parse_inventory, Host},
    modules::ModuleResult,
    playbook::{ExecutionOptions, Strategy},
    ssh::{
        config::ConnectionConfig,
        connection::{Connection, LocalConnection, SshConnection, WorkerConnection},
    },
};
use rustsible_protocol::extension::{Call, Capabilities, Definition, Kind};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
static EVENTS: Mutex<Vec<Completion>> = Mutex::new(Vec::new());
static CONNECTED: AtomicUsize = AtomicUsize::new(0);
static ELEVATED: AtomicUsize = AtomicUsize::new(0);
struct Local(LocalConnection);
impl SshConnection for Local {
    fn is_local(&self) -> bool {
        true
    }
    fn execute_command(&self, command: &str) -> Result<(i32, String, String)> {
        self.0.execute_command(command)
    }
    fn execute_command_with_input(
        &self,
        command: &str,
        input: &[u8],
    ) -> Result<(i32, String, String)> {
        self.0.execute_command_with_input(command, input)
    }
}
impl WorkerConnection for Local {
    fn start_worker(&self, command: &str) -> Result<rustsible::worker::transport::Session> {
        rustsible::worker::transport::Session::local(command)
    }
}
fn connect(_: &ConnectionConfig) -> Result<Connection> {
    CONNECTED.fetch_add(1, Ordering::SeqCst);
    let mut host = Host::new("localhost");
    host.set_variable("ansible_connection", "local");
    Ok(Connection::Compiled(Box::new(Local(LocalConnection::new(
        &host,
    )?))))
}
fn elevate(_: &ConnectionConfig, bootstrap: &str) -> Result<Bootstrap> {
    // The fixture selects the current UID; production providers must enforce
    // their requested identity, which is checked by the worker handshake.
    ELEVATED.fetch_add(1, Ordering::SeqCst);
    Ok(Bootstrap {
        command: bootstrap.into(),
        input: Default::default(),
    })
}
fn action(_: &Call, vars: &HashMap<String, serde_yaml::Value>) -> Result<ModuleResult> {
    assert!(vars.contains_key("inventory_hostname"));
    Ok(ModuleResult {
        msg: "private-action-output".into(),
        ..Default::default()
    })
}
fn reverse(hosts: &[Host]) -> Result<Vec<usize>> {
    Ok((0..hosts.len()).rev().collect())
}
fn bad_order(_: &[Host]) -> Result<Vec<usize>> {
    Ok(vec![0, 0])
}
fn record(event: &Completion) {
    EVENTS.lock().unwrap().push(event.clone());
}
fn components(order: fn(&[Host]) -> Result<Vec<usize>>) -> Components {
    Components {
        actions: BTreeMap::from([(
            "linked.example.action",
            Arc::new(Action {
                definition: Definition {
                    name: "linked.example.action".into(),
                    kind: Kind::Module,
                    capabilities: Capabilities {
                        check: true,
                        diff: false,
                        asynchronous: false,
                        mutates: false,
                    },
                    parameters: Default::default(),
                },
                execute: action,
            }),
        )]),
        connections: BTreeMap::from([("linked.example.local", connect as ConnectionFactory)]),
        become_providers: BTreeMap::from([("linked.example.identity", elevate as BecomeHandler)]),
        strategies: BTreeMap::from([(
            "linked.example.reverse",
            Scheduling {
                engine: Strategy::Linear,
                order,
            },
        )]),
        callbacks: vec![record],
    }
}
#[test]
fn linked_components_run_through_the_real_playbook_worker_and_reporting_paths() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let inventory = temporary.path().join("inventory.yml");
    let playbook = temporary.path().join("play.yml");
    let output = temporary.path().join("result");
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
    fs::write(&inventory, format!("all:\n  hosts:\n    first: {{}}\n    second: {{}}\n  vars:\n    ansible_connection: linked.example.local\n    ansible_become: true\n    ansible_become_method: linked.example.identity\n    ansible_become_user: {user}\n"))?;
    fs::write(&playbook, format!("- hosts: all\n  gather_facts: false\n  strategy: linked.example.reverse\n  serial: 1\n  tasks:\n    - linked.example.action: {{}}\n      no_log: true\n    - copy:\n        content: '{{{{ inventory_hostname }}}}'\n        dest: {}\n", output.display()))?;
    let registry = Registry::new(&[], temporary.path())?.with_compiled(components(reverse))?;
    let scope = registry.enter();
    let inventory = parse_inventory(inventory.to_str().unwrap())?;
    drop(scope);
    let mut options = ExecutionOptions {
        extensions: registry,
        worker_dir: Some(support::worker_bundle()),
        forks: 1,
        check_mode: true,
        ..Default::default()
    };
    rustsible::playbook::execute_with_options(playbook.to_str().unwrap(), &inventory, &options)?;
    assert!(!output.exists());
    options.check_mode = false;
    rustsible::playbook::execute_with_options(playbook.to_str().unwrap(), &inventory, &options)?;
    assert_eq!(fs::read_to_string(&output)?, "first");
    assert_eq!(CONNECTED.load(Ordering::SeqCst), 4);
    assert_eq!(ELEVATED.load(Ordering::SeqCst), 4);
    let events = EVENTS.lock().unwrap().clone();
    assert_eq!(events.len(), 8);
    assert_eq!(events[0].host, "second");
    assert_eq!(events[2].host, "first");
    assert!(events[..4].iter().all(|event| event.check));
    assert!(!format!("{events:?}").contains("private-action-output"));
    options.extensions =
        Registry::new(&[], temporary.path())?.with_compiled(components(bad_order))?;
    assert!(rustsible::playbook::execute_with_options(
        playbook.to_str().unwrap(),
        &inventory,
        &options
    )
    .is_err());
    assert_eq!(fs::read_to_string(output)?, "first");
    Ok(())
}
