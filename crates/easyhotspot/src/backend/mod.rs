//! The platform backend: NetworkManager on Linux, the Mobile Hotspot on
//! Windows. Both expose the same `Live` API to the model.

pub mod common;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{Event, Live, helper_install, runtime};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Event, Live, helper_install};

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("Easy Hotspot runs on Linux and Windows.");
