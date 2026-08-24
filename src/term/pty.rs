//! A child process on a pseudo terminal.
//!
//! One thread per process pumps its output to the UI thread and reports the
//! exit after the last byte, so a session that ends still shows what it said.
//! The process keeps running — and its output keeps being consumed — while lg
//! shows something else, which is what lets several sessions live at once.

use anyhow::{Context, Result};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;

const READ_BUFFER: usize = 8 * 1024;

/// What the process sends back to the UI thread.
#[derive(Debug)]
pub enum PtyMsg {
    Output(Vec<u8>),
    /// The process ended; the string is what to show the user.
    Exited(String),
}

/// What to run on a pseudo terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawn {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Variables to set for the child.
    pub env: Vec<(String, String)>,
    /// Variables to drop from the inherited environment.
    pub env_remove: Vec<String>,
}

pub struct PtyProcess {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    rx: Receiver<PtyMsg>,
    pump: Option<JoinHandle<()>>,
    size: (u16, u16),
}

impl PtyProcess {
    /// Start `spawn` on a pty of `rows` x `cols`.
    pub fn start(spawn: &Spawn, (rows, cols): (u16, u16)) -> Result<Self> {
        let size = PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .context("open pseudo terminal")?;

        let mut command = CommandBuilder::new(&spawn.program);
        command.args(&spawn.args);
        command.cwd(&spawn.cwd);
        for name in &spawn.env_remove {
            command.env_remove(name);
        }
        for (name, value) in &spawn.env {
            command.env(name, value);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("failed to start {}", spawn.program))?;
        // Drop our copy of the slave side, otherwise the master never reaches
        // end of file and the exit is never noticed.
        drop(pair.slave);

        let killer = child.clone_killer();
        let reader = pair.master.try_clone_reader().context("read pty")?;
        let writer = pair.master.take_writer().context("write pty")?;
        let (tx, rx) = channel();
        let pump = std::thread::spawn(move || pump_output(reader, child, tx));

        Ok(Self {
            master: pair.master,
            writer,
            killer,
            rx,
            pump: Some(pump),
            size: (size.rows, size.cols),
        })
    }

    /// Send input to the process. A closed pty means the process is gone, which
    /// the pump reports separately, so nothing is raised here.
    pub fn write(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    /// Tell the process its window changed, which also makes full-screen
    /// programs repaint — how a session that was in the background comes back
    /// looking right.
    pub fn resize(&mut self, (rows, cols): (u16, u16)) {
        let size = (rows.max(1), cols.max(1));
        if size == self.size {
            return;
        }
        let _ = self.master.resize(PtySize {
            rows: size.0,
            cols: size.1,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.size = size;
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn try_recv(&self) -> std::result::Result<PtyMsg, TryRecvError> {
        self.rx.try_recv()
    }

    /// Stop the process. Its pump thread then sees end of file and finishes.
    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        self.kill();
        // The pump thread ends on its own once the pty closes; waiting for it
        // here would block quitting on a process that ignores being killed.
        drop(self.pump.take());
    }
}

/// Read until the pty closes, then report how the process ended. Reading to the
/// end first keeps the last output ahead of the exit notice.
fn pump_output(
    mut reader: Box<dyn std::io::Read + Send>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    tx: Sender<PtyMsg>,
) {
    let mut buffer = vec![0u8; READ_BUFFER];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if tx.send(PtyMsg::Output(buffer[..read].to_vec())).is_err() {
                    return;
                }
            }
            // A closed pty reads as an error on some platforms rather than as
            // end of file; either way there is nothing more to read.
            Err(_) => break,
        }
    }
    let notice = match child.wait() {
        Ok(status) if status.success() => "exited".to_string(),
        Ok(status) => match status.signal() {
            Some(signal) => format!("killed by {signal}"),
            None => format!("exited with status {}", status.exit_code()),
        },
        Err(err) => format!("lost track of the process: {err}"),
    };
    let _ = tx.send(PtyMsg::Exited(notice));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn sh(script: &str) -> Spawn {
        Spawn {
            program: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            cwd: std::env::temp_dir(),
            env: vec![("TERM".to_string(), "xterm-256color".to_string())],
            env_remove: Vec::new(),
        }
    }

    /// Collect messages until the process reports its exit, or the wait is up.
    fn drain_until_exit(process: &PtyProcess) -> (String, Option<String>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = Vec::new();
        while Instant::now() < deadline {
            match process.try_recv() {
                Ok(PtyMsg::Output(bytes)) => output.extend_from_slice(&bytes),
                Ok(PtyMsg::Exited(notice)) => {
                    return (String::from_utf8_lossy(&output).into_owned(), Some(notice));
                }
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(10)),
                Err(TryRecvError::Disconnected) => break,
            }
        }
        (String::from_utf8_lossy(&output).into_owned(), None)
    }

    #[test]
    fn output_arrives_before_the_exit_notice() {
        let process = PtyProcess::start(&sh("printf 'hello pty'"), (24, 80)).expect("start");
        let (output, exit) = drain_until_exit(&process);
        assert!(output.contains("hello pty"), "got {output:?}");
        assert_eq!(exit.as_deref(), Some("exited"));
    }

    #[test]
    fn a_failing_command_reports_its_status() {
        let process = PtyProcess::start(&sh("exit 3"), (24, 80)).expect("start");
        let (_, exit) = drain_until_exit(&process);
        assert_eq!(exit.as_deref(), Some("exited with status 3"));
    }

    #[test]
    fn input_reaches_the_process() {
        let mut process = PtyProcess::start(&sh("read line; printf 'got:%s' \"$line\""), (24, 80))
            .expect("start");
        process.write(b"ping\r");
        let (output, exit) = drain_until_exit(&process);
        assert!(output.contains("got:ping"), "got {output:?}");
        assert_eq!(exit.as_deref(), Some("exited"));
    }

    #[test]
    fn the_process_sees_the_size_it_was_given_and_a_later_resize() {
        let mut process =
            PtyProcess::start(&sh("stty size; read line; stty size"), (24, 80)).expect("start");
        // Wait for the first report before resizing, so the two are distinct.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = String::new();
        while !output.contains("24 80") && Instant::now() < deadline {
            if let Ok(PtyMsg::Output(bytes)) = process.try_recv() {
                output.push_str(&String::from_utf8_lossy(&bytes));
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert!(output.contains("24 80"), "first size: {output:?}");

        process.resize((30, 100));
        assert_eq!(process.size(), (30, 100));
        process.write(b"\r");
        let (rest, _) = drain_until_exit(&process);
        assert!(rest.contains("30 100"), "resized: {rest:?}");
    }

    #[test]
    fn killing_a_process_ends_the_session() {
        let mut process = PtyProcess::start(&sh("sleep 60"), (24, 80)).expect("start");
        process.kill();
        let (_, exit) = drain_until_exit(&process);
        let exit = exit.expect("the kill is reported");
        assert!(
            exit.contains("killed") || exit.contains("status"),
            "unexpected notice: {exit}"
        );
    }

    #[test]
    fn a_program_that_does_not_exist_fails_to_start() {
        let mut spawn = sh("true");
        spawn.program = "/definitely/not/a/program".to_string();
        assert!(PtyProcess::start(&spawn, (24, 80)).is_err());
    }
}
