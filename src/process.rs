//! Deadline-bound subprocesses. Files avoid pipe backpressure and inherited-pipe hangs.
use anyhow::{bail, Context, Result};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Capture {
    path: PathBuf,
    file: File,
}
impl Capture {
    fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "herdr-linear-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => return Ok(Self { path, file }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
    fn bytes(&mut self) -> Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        self.file
            .by_ref()
            .take(32_000_001)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 32_000_000 {
            bail!("command output exceeds 32 MB limit");
        }
        Ok(bytes)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn run(command: &mut Command, timeout: Duration) -> Result<Output> {
    let mut stdout = Capture::new()?;
    let mut stderr = Capture::new()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout.file.try_clone()?)
        .stderr(stderr.file.try_clone()?)
        .spawn()?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
        }
        if Instant::now() >= deadline
            || stdout.file.metadata()?.len() > 32_000_000
            || stderr.file.metadata()?.len() > 32_000_000
        {
            let _ = child.kill();
            child.wait().context("could not reap timed-out command")?;
            bail!("command timed out or exceeded output limit; retry the request");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Ok(Output {
        status,
        stdout: stdout.bytes()?,
        stderr: stderr.bytes()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn times_out_and_reaps_direct_child() {
        let start = Instant::now();
        let error = run(
            Command::new("sh").args(["-c", "exec sleep 10"]),
            Duration::from_millis(30),
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn drains_output_without_pipe_deadlock() {
        let output = run(
            Command::new("sh").args(["-c", "printf hello; printf 'first\\nsecond' >&2"]),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(output.stdout, b"hello");
        assert_eq!(output.stderr, b"first\nsecond");
    }
}
