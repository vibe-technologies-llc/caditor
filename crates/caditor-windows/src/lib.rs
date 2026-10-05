#![cfg(windows)]

#[cfg(test)]
mod tests;

mod console;
mod files;
mod process;
mod registry;
mod window;

pub use console::{attach_parent_console, on_console_close};
pub use files::{FileId, HIDDEN_ATTRIBUTE, move_file_durably, open_for_identity, replace_file};
pub use process::process_running;
pub use registry::{boot_id, machine_guid};
pub use window::{DialogParent, on_session_end, show_error};
