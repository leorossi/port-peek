use crate::{
    Result,
    model::{Listener, Protocol},
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub(crate) fn find_listeners(port: u16, protocol: Protocol) -> Result<Vec<Listener>> {
    macos::find_listeners(port, protocol)
}

#[cfg(target_os = "linux")]
pub(crate) fn find_listeners(port: u16, protocol: Protocol) -> Result<Vec<Listener>> {
    linux::find_listeners(port, protocol)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn find_listeners(_port: u16, _protocol: Protocol) -> Result<Vec<Listener>> {
    Err(crate::Error::UnsupportedPlatform)
}
