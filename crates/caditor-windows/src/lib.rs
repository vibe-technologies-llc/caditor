#![cfg(windows)]

#[cfg(test)]
mod tests;

mod cloning;
mod console;
mod files;
mod maximize;
mod process;
mod registry;
mod window;

pub use cloning::{cluster_size, duplicate_extents};
pub use console::{attach_parent_console, on_console_close};
pub use files::{
    FileId, HIDDEN_ATTRIBUTE, final_path, move_file_durably, open_for_identity, replace_file,
};
pub use maximize::{ButtonRect, ButtonState, on_maximize_button, set_maximize_button};
pub use process::process_running;
pub use registry::{boot_id, machine_guid};
pub use window::{DialogParent, on_session_end, show_error};
