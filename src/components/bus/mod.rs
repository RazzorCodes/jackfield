// Bus sub-module index. Re-exports the routing/dispatch surface from child modules.
pub mod bus;
pub mod error;
#[cfg(test)]
pub mod test;
pub use bus::{Bus, BusCmdHandle};
pub use crate::components::registry::RegistrationBuilder;
pub use error::JackfieldError;
