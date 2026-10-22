//! The epoch ticker: one thread per engine that advances the engine epoch every [`TICK`] while any guest call is
//! running, and sleeps otherwise. Each store checks its wall-clock deadline whenever the epoch moves, so deadlines
//! are honoured with a granularity of about one tick.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::thread::{self, Thread};
use std::time::Duration;

use wasmtime::Engine;

/// How often the epoch advances during a guest call.
pub const TICK: Duration = Duration::from_millis(1);

/// Shared by the runtime and every instance; the thread ends when the last owner is dropped.
pub struct Ticker {
    active: AtomicUsize,
    thread: OnceLock<Thread>,
}

impl Ticker {
    /// Starts the ticker thread for `engine`.
    pub fn start(engine: Engine) -> Arc<Ticker> {
        let t = Arc::new(Ticker { active: AtomicUsize::new(0), thread: OnceLock::new() });
        let weak = Arc::downgrade(&t);
        let handle = thread::Builder::new().name("bhmod-wasm-epoch".into()).spawn(move || run(&weak, &engine));
        let thread = handle.expect("cannot start the epoch thread").thread().clone();
        t.thread.get_or_init(|| thread);
        t
    }

    /// Marks a guest call as running until the guard is dropped.
    pub fn busy(&self) -> Busy<'_> {
        self.active.fetch_add(1, Ordering::SeqCst);
        self.wake();
        Busy(self)
    }

    fn wake(&self) {
        if let Some(t) = self.thread.get() {
            t.unpark();
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.wake();
    }
}

/// A running guest call.
pub struct Busy<'a>(&'a Ticker);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

fn run(weak: &Weak<Ticker>, engine: &Engine) {
    while let Some(t) = weak.upgrade() {
        let busy = t.active.load(Ordering::SeqCst) > 0;
        drop(t);
        if busy {
            engine.increment_epoch();
            thread::sleep(TICK);
        } else {
            thread::park();
        }
    }
}
