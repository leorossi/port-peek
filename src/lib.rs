//! Find which process is listening on a local port.
//!
//! The lookup is implemented with native kernel interfaces:
//!
//! - macOS: `libproc` for process file descriptors and sockets, plus `sysctl`
//!   (`KERN_PROCARGS2`) for the argument vector.
//! - Linux: `NETLINK_SOCK_DIAG` for the socket table, then `/proc` for the
//!   inode-to-PID mapping and process metadata.
//!
//! Docker-forwarded ports are annotated with container metadata from the local
//! Docker Engine API. No `lsof`, `netstat`, `ss`, or `docker` output is parsed.

pub mod cli;
#[cfg(unix)]
mod docker;
mod error;
mod model;
pub mod output;
mod platform;

pub use error::{Error, Result};
pub use model::{ContainerInfo, Listener, Protocol, Report, shell_join};

/// Look up listeners for `port` using the requested protocol.
///
/// A socket that is visible but whose owner cannot be read is returned with
/// `pid: None`; a missing listener is an empty report, not an error.
pub fn find_listeners(port: u16, protocol: Protocol) -> Result<Report> {
    if port == 0 {
        return Err(Error::Backend(
            "port must be between 1 and 65535".to_owned(),
        ));
    }

    let mut listeners = platform::find_listeners(port, protocol)?;
    #[cfg(unix)]
    docker::annotate(&mut listeners);
    Ok(Report::new(port, protocol, listeners).normalized())
}
