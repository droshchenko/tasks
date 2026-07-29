mod render;

pub use render::*;

/// Re-exported so the view asks the socket to follow a board without knowing where the socket lives.
pub use crate::web::watch_project;
