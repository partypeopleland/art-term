//! Native operator credentials, transactional Host profiles and HTTPS admin API.
//! No frontend DTO contains a credential; embedding platforms supply the cipher.
#[cfg(all(feature = "openab-fixture", not(debug_assertions)))]
compile_error!("openab-fixture is forbidden in release builds");
pub mod api;
pub mod history;
pub mod profiles;
pub mod screenshot;
pub mod secret;
pub mod session;
#[cfg(test)]
mod tests;
pub mod transfer;
