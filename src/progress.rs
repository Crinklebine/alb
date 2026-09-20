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
                    eprint!(
                        "\r\x1b[2K{} {operation}: {amount}",
                        ["|", "/", "-", "\\"][frame % 4]
                    );
                    let _ = io::stderr().flush();
                    frame += 1;
                }
                if receiver.recv_timeout(Duration::from_millis(150))
                    != Err(mpsc::RecvTimeoutError::Timeout)
                {
                    break;
                }
            }
            if terminal {
                eprint!("\r\x1b[2K");
                let _ = io::stderr().flush();
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
