//! Running a command under a macOS Seatbelt sandbox.
//!
//! [`runner`] orchestrates a run; the modules around it each own one decision it
//! makes: [`policy`] renders the SBPL text, [`launch`] picks the command,
//! [`proxy_env`] handles outbound network, [`mcp_session`] provides the tools the
//! sandboxed session calls back into, and [`registry`] records what is running.

mod banner;
mod launch;
mod mcp_session;
mod policy;
mod proxy_env;
pub mod registry;
pub mod runner;
pub mod seatbelt;
