//! The data of one instance's store: the host API of the call in progress, limits, the resource table and a
//! minimal WASI context (no files, no network, no environment; stdout and stderr go to the mod's log).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use wasmtime::component::ResourceTable;
use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::{OutputStream, Pollable, StreamError, StreamResult};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use bhmod_core::types::LogLevel;
use bhmod_core::HostApi;

/// The most bytes of guest output kept between two flushes; more is dropped.
pub const STDIO_CAP: usize = 64 * 1024;

/// The host API pointer of the call in progress, with the lifetime erased.
#[derive(Clone, Copy)]
pub struct HostPtr(*mut (dyn HostApi + 'static));

// SAFETY: a store (and with it this pointer) is only used on the thread that created the instance; `Send` is
// required by `WasiView` only. The pointer is set for the duration of one guest call (see `State::enter`).
unsafe impl Send for HostPtr {}

/// Guest stdout or stderr: a shared byte buffer drained into the log after every call.
#[derive(Clone, Default)]
pub struct LogPipe(Arc<Mutex<Vec<u8>>>);

impl LogPipe {
    /// Takes everything written so far, split into lines.
    pub fn take_lines(&self) -> Vec<String> {
        let buf = std::mem::take(&mut *self.0.lock().expect("log pipe lock"));
        String::from_utf8_lossy(&buf).lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect()
    }

    fn push(&self, bytes: &[u8]) {
        let mut buf = self.0.lock().expect("log pipe lock");
        let room = STDIO_CAP.saturating_sub(buf.len());
        buf.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }
}

impl IsTerminal for LogPipe {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for LogPipe {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(tokio::io::sink())
    }
}

#[wasmtime_wasi::async_trait]
impl Pollable for LogPipe {
    async fn ready(&mut self) {}
}

impl OutputStream for LogPipe {
    fn write(&mut self, bytes: bytes::Bytes) -> StreamResult<()> {
        self.push(&bytes);
        Ok(())
    }
    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }
    fn check_write(&mut self) -> Result<usize, StreamError> {
        Ok(4096)
    }
}

/// The store data of one instance.
pub struct State {
    host: Option<HostPtr>,
    /// When the call in progress must end.
    pub deadline: Option<Instant>,
    /// Set by the epoch callback when it stopped the guest at the deadline.
    pub timed_out: bool,
    /// Memory and instance limits.
    pub limits: StoreLimits,
    /// Host resources handed to the guest (players).
    pub table: ResourceTable,
    wasi: WasiCtx,
    /// Guest stdout (logged as info).
    pub stdout: LogPipe,
    /// Guest stderr (logged as warnings).
    pub stderr: LogPipe,
}

impl State {
    /// A state with a memory cap of `memory_bytes`.
    pub fn new(memory_bytes: usize) -> State {
        let stdout = LogPipe::default();
        let stderr = LogPipe::default();
        let wasi = WasiCtxBuilder::new().stdout(stdout.clone()).stderr(stderr.clone()).build();
        let limits = StoreLimitsBuilder::new().memory_size(memory_bytes).instances(64).tables(64).memories(16).build();
        State {
            host: None,
            deadline: None,
            timed_out: false,
            limits,
            table: ResourceTable::new(),
            wasi,
            stdout,
            stderr,
        }
    }

    /// The host API of the call in progress.
    pub fn host_api(&mut self) -> Option<&mut dyn HostApi> {
        // SAFETY: the pointer is set by `enter` from a `&mut dyn HostApi` that outlives the guest call and is
        // cleared by `leave` before that borrow ends. Guest code runs on this thread only and host functions never
        // re-enter the guest, so this is the only live reference derived from it.
        self.host.map(|p| unsafe { &mut *p.0 })
    }

    /// Starts a guest call with `host`.
    pub fn enter(&mut self, host: &mut dyn HostApi) {
        self.deadline = Some(host.deadline());
        self.timed_out = false;
        let ptr: *mut (dyn HostApi + '_) = host;
        // SAFETY: only the lifetime is erased; `leave` clears the pointer before the borrow ends.
        let ptr = unsafe { std::mem::transmute::<*mut (dyn HostApi + '_), *mut (dyn HostApi + 'static)>(ptr) };
        self.host = Some(HostPtr(ptr));
    }

    /// Ends the guest call that `enter` started with `host`: forgets the host and forwards guest output to its log.
    pub fn leave(&mut self, host: &mut dyn HostApi) {
        self.host = None;
        self.deadline = None;
        for line in self.stdout.take_lines() {
            host.log(LogLevel::Info, &line);
        }
        for line in self.stderr.take_lines() {
            host.log(LogLevel::Warn, &line);
        }
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_lines_and_cap() {
        let p = LogPipe::default();
        p.push(b"one\n\ntwo\nthr");
        assert_eq!(p.take_lines(), vec!["one", "two", "thr"]);
        assert!(p.take_lines().is_empty());
        p.push(&vec![b'x'; STDIO_CAP + 10]);
        assert_eq!(p.take_lines()[0].len(), STDIO_CAP);
        assert!(!p.is_terminal());
        let _ = p.async_stream();
        let mut s = p.clone();
        assert_eq!(s.check_write().unwrap(), 4096);
        s.flush().unwrap();
    }
}
