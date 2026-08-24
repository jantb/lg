//! Running another terminal program inside lg.

mod keys;
mod mouse;
mod pty;
mod screen;

pub use keys::{encode_key, encode_paste};
pub use mouse::encode_wheel;
pub use pty::{PtyMsg, PtyProcess, Spawn};
pub use screen::render_screen;
