use std::sync::mpsc::{self, Receiver, TryRecvError};

thread_local! { static IN_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// Install after ratatui::init: only UI-thread panics may restore the terminal.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !IN_WORKER.with(|flag| flag.get()) {
            previous(info);
        }
    }));
}

pub fn run<T>(work: impl FnOnce() -> anyhow::Result<T>) -> Result<T, String> {
    IN_WORKER.with(|flag| flag.set(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
    IN_WORKER.with(|flag| flag.set(false));
    match result {
        Ok(result) => result.map_err(|e| format!("{e:#}")),
        Err(_) => Err("background worker panicked; retry the request".into()),
    }
}

/// One background result. Dropping this handle discards late results.
pub struct Job<T> {
    result: Receiver<Result<T, String>>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(
        work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    ) -> Result<Self, String> {
        let (sender, result) = mpsc::channel();
        std::thread::Builder::new()
            .name("linear-worker".into())
            .spawn(move || {
                let _ = sender.send(run(work));
            })
            .map_err(|e| format!("could not start worker: {e}"))?;
        Ok(Self { result })
    }

    #[cfg(test)]
    pub fn channel() -> (mpsc::Sender<Result<T, String>>, Self) {
        let (sender, result) = mpsc::channel();
        (sender, Self { result })
    }

    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("background worker stopped unexpectedly".into()))
            }
        }
    }
}
