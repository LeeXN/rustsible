mod parser;
mod task;
mod play;
mod handlers;
mod templar;
pub mod filters;

use anyhow::Result;
use log::{info, error, debug};
use crate::inventory::Inventory;

pub use task::{Task, TaskResult};
pub use play::Play;
pub use handlers::Handler;

pub fn execute(playbook_file: &str, inventory: &Inventory) -> Result<()> {
    info!("Loading playbook from file: {}", playbook_file);
    
    let playbook = parser::parse_playbook(playbook_file)?;
    info!("Playbook contains {} plays", playbook.plays.len());
    
    for (index, play) in playbook.plays.iter().enumerate() {
        info!("PLAY [{}] ({}/{})", play.name, index + 1, playbook.plays.len());
        
        let hosts = inventory.filter_hosts(&play.hosts);
        if hosts.is_empty() {
            error!("No hosts matched for play '{}' with pattern: {}", play.name, play.hosts);
            continue;
        }
        
        debug!("Play '{}' matched {} hosts", play.name, hosts.len());
        let play_result = play.execute(&hosts);
        
        if let Err(e) = play_result {
            error!("Play '{}' failed: {}", play.name, e);
            // Continue with next play unless fail_fast is enabled
            if playbook.fail_fast {
                return Err(e);
            }
        }
    }
    
    info!("Playbook execution completed");
    Ok(())
} 