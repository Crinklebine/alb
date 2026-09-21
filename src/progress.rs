//! One stderr line on terminals; bounded stage messages when redirected.
use std::{
    io::{self, IsTerminal, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
pub struct Progress {
    done: Arc<AtomicUsize>,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Progress {
    pub fn new(operation: &'static str, total: Option<usize>) -> Self {
        let done = Arc::new(AtomicUsize::new(0));
        let count = done.clone();
        let (stop, receiver) = mpsc::channel();
        let terminal = io::stderr().is_terminal() && crate::platform::terminal_progress_supported();
        if !terminal {
            eprintln!(
                "{operation}: starting{}",
                total.map(|n| format!(" ({n} files)")).unwrap_or_default()
            );
        }
        let worker = thread::spawn(move || {
            let mut frame = 0;
            loop {
                if terminal {
                    let n = count.load(Ordering::Relaxed);
                    let amount = total
                        .map(|t| format!("{n}/{t} files"))
                        .unwrap_or_else(|| format!("{n} entries"));
                    // Overwrite before clearing the remaining tail so terminals
                    // never display a deliberately blank line between frames.
                    let line = format!(
                        "\r{} {operation}: {amount}\x1b[K",
                        ["|", "/", "-", "\\"][frame % 4]
                    );
                    // Format first, then hold stderr only for this complete frame.
                    let mut stderr = io::stderr().lock();
                    let _ = stderr.write_all(line.as_bytes());
                    let _ = stderr.flush();
                    frame += 1;
                }
                if receiver.recv_timeout(Duration::from_millis(250))
                    != Err(mpsc::RecvTimeoutError::Timeout)
                {
                    break;
                }
            }
            if terminal {
                let mut stderr = io::stderr().lock();
                let _ = stderr.write_all(b"\r\x1b[2K");
                let _ = stderr.flush();
            } else {
                eprintln!(
                    "{operation}: stage ended ({} processed)",
                    count.load(Ordering::Relaxed)
                );
            }
        });
        Self {
            done,
            stop,
            worker: Some(worker),
        }
    }
    pub fn set(&self, count: usize) {
        self.done.store(count, Ordering::Relaxed);
    }
}
impl Drop for Progress {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
