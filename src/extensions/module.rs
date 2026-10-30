use super::*;
use rustsible_protocol::registry::{Builtin, CheckSupport};
#[derive(Debug, Clone)]
pub enum Module {
    Builtin(Builtin),
    Native(Arc<Native>),
    Action(Arc<super::compiled::Action>),
}
impl PartialEq<Builtin> for Module {
    fn eq(&self, other: &Builtin) -> bool {
        matches!(self,Self::Builtin(value) if value==other)
    }
}
impl Module {
    pub fn definition(&self) -> Option<&Definition> {
        match self {
            Self::Native(value) => Some(&value.definition),
            Self::Action(value) => Some(&value.definition),
            _ => None,
        }
    }
    pub fn builtin(&self) -> Option<Builtin> {
        match self {
            Self::Builtin(value) => Some(*value),
            _ => None,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Builtin(value) => value.name(),
            Self::Native(value) => &value.definition.name,
            Self::Action(value) => &value.definition.name,
        }
    }
    pub fn parameters(&self) -> Vec<&str> {
        match self {
            Self::Builtin(value) => value.parameters().to_vec(),
            _ => self
                .definition()
                .unwrap()
                .parameters
                .keys()
                .map(String::as_str)
                .collect(),
        }
    }
    pub fn check_support(&self) -> CheckSupport {
        match self {
            Self::Builtin(value) => value.check_support(),
            _ => {
                if self.definition().unwrap().capabilities.mutates {
                    CheckSupport::Predict
                } else {
                    CheckSupport::ReadOnly
                }
            }
        }
    }
    pub fn validate_parameters(&self, args: &serde_yaml::Value) -> Result<()> {
        match self {
            Self::Builtin(value) => value.validate_parameters(args),
            _ => self.definition().unwrap().validate_call(&Call::new(
                self.definition().unwrap(),
                args.clone(),
                false,
                false,
                1000,
            )),
        }
    }
    pub fn validate_capabilities(&self, check: bool, diff: bool, asynchronous: bool) -> Result<()> {
        match self {
            Self::Builtin(value) => value.validate_capabilities(check, diff, asynchronous),
            _ => {
                let cap = self.definition().unwrap().capabilities;
                if check && !cap.check || diff && !cap.diff || asynchronous && !cap.asynchronous {
                    bail!("Native module does not support the requested check/diff/async mode");
                }
                Ok(())
            }
        }
    }
    pub fn may_modify(&self, args: &serde_yaml::Value, check: bool) -> bool {
        match self {
            Self::Builtin(value) => value.may_modify(args, check),
            _ => !check && self.definition().unwrap().capabilities.mutates,
        }
    }
}
