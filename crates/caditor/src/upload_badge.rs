pub const TEXT: &str = "Loading to the graphics card";
pub const STOPPED: &str = "Loading to the graphics card stopped";
pub const CONTINUE: &str = "Continue loading";
pub const CANCEL_HOVER: &str = "Stop loading bodies to the graphics card; the ones not loaded yet stay hidden until Continue loading";
pub const STOPPED_HOVER: &str = "Loading to the graphics card was stopped; the bodies not loaded yet stay hidden until Continue loading";
pub const CONTINUE_HOVER: &str = "Load the hidden bodies to the graphics card and show them";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadRequest {
    Stop,
    Resume,
}
