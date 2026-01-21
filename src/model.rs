use std::collections::VecDeque;
use std::fs::File;

// Single step in the installation process
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending, // Not yet started
    Running, // Currently in progress
    Done,    // Completed successfully
    Skipped, // Was skipped
    Failed,  // Failed with an error
}

// Single installation step
pub struct Step {
    pub name: String,        // The name of the step
    pub status: StepStatus,  // The current status of the step
    pub err: Option<String>, // An error message if the step failed
}

// Events sent from the installer thread to the main UI
pub enum InstallerEvent {
    // A log message to be displayed in the UI
    Log(String),
    // The overall installation progress, as a value between 0.0 and 1.0
    Progress(f64),
    // An update on the status of a specific step
    Step {
        index: usize,
        status: StepStatus,
        err: Option<String>,
    },
    // Done
    Done(Option<String>),
}

// The main application state
pub struct App {
    // The list of all installation steps
    pub steps: Vec<Step>,
    // The overall progress of the installation
    pub progress: f64,
    // A queue of log messages to be displayed
    pub logs: VecDeque<String>,
    // The current frame of the loading spinner animation
    pub spinner_idx: usize,
    // A flag indicating whether the installation is finished
    pub done: bool,
    // A final error message if the installation failed
    pub err: Option<String>,
    // An optional handle to the log file for writing logs to disk
    pub log_file: Option<File>,
}
