//! Request-local Linux supervision shared by target modules and explicitly
//! installed controller extensions. It has no inventory, SSH or template code.
pub mod archive;
pub mod protection;
pub mod rpc;
pub mod staging;
pub mod supervisor;
