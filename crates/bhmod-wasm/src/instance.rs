//! One instantiated component: the WIT `guest` export behind [`Instance`], for components of API 0.2 or 0.1.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wasmtime::component::{Component, Linker, Resource};
use wasmtime::{Store, Trap, UpdateDeadline};

use bhmod_core::channels::ChannelId;
use bhmod_core::runtime::{CallError, CommandId, HostApi, Instance, InstanceLimits, SubscriptionId, TaskId};
use bhmod_core::types as core;

use crate::bindings::{v01, v02, PlayerRes};
use crate::state::State;
use crate::ticker::Ticker;

/// How long instantiation (core module start functions) may run.
pub const INSTANTIATE_BUDGET: Duration = Duration::from_millis(500);

/// The guest export of one API version.
enum Bindings {
    V02(v02::ServerMod),
    V01(v01::ServerMod),
}

/// The `blockheads:api` version a component was built for (from its `guest` export).
pub fn api_version_of(component: &Component) -> &'static str {
    let ty = component.component_type();
    let old = ty.exports(component.engine()).any(|(name, _)| name == "blockheads:api/guest@0.1.0");
    if old {
        "0.1.0"
    } else {
        "0.2.0"
    }
}

/// One WASM mod.
pub struct WasmInstance {
    store: Store<State>,
    bindings: Bindings,
    ticker: Arc<Ticker>,
    /// Set after a trap: the component model forbids entering the instance again.
    poisoned: bool,
}

fn trap_text(e: wasmtime::Error) -> String {
    match e.downcast_ref::<Trap>() {
        Some(t) => t.to_string(),
        None => format!("{e:#}"),
    }
}

/// A lender of player handles over the store's resource table; the handles are returned after the call.
fn lender(store: &mut Store<State>) -> impl FnMut(&str) -> Resource<PlayerRes> + '_ {
    move |id| {
        let st = store.data_mut();
        // The table only fails past u32::MAX live entries.
        let r = st.table.push(PlayerRes(id.to_string())).expect("resource table full");
        st.lent.push(r.rep());
        Resource::new_borrow(r.rep())
    }
}

type Guest = wasmtime::Result<()>;
type Done = Result<(), CallError>;

impl WasmInstance {
    /// Instantiates `component`. No host API is available yet, so a start function that calls an import traps.
    pub fn new(
        linker: &Linker<State>,
        component: &Component,
        ticker: Arc<Ticker>,
        limits: InstanceLimits,
    ) -> Result<WasmInstance, String> {
        let mut store = Store::new(component.engine(), State::new(limits.memory_bytes));
        store.limiter(|s| &mut s.limits);
        store.epoch_deadline_callback(|mut ctx| {
            if ctx.data().deadline.is_some_and(|d| Instant::now() >= d) {
                ctx.data_mut().timed_out = true;
                return Ok(UpdateDeadline::Interrupt);
            }
            Ok(UpdateDeadline::Continue(1))
        });
        store.data_mut().deadline = Some(Instant::now() + INSTANTIATE_BUDGET);
        store.set_epoch_deadline(1);
        let busy = ticker.busy();
        let bindings = match api_version_of(component) {
            "0.1.0" => v01::ServerMod::instantiate(&mut store, component, linker).map(Bindings::V01),
            _ => v02::ServerMod::instantiate(&mut store, component, linker).map(Bindings::V02),
        };
        drop(busy);
        store.data_mut().deadline = None;
        Ok(WasmInstance { store, bindings: bindings.map_err(trap_text)?, ticker, poisoned: false })
    }

    /// Runs one guest call under the host's deadline and maps traps and timeouts.
    fn call(&mut self, host: &mut dyn HostApi, f: &mut dyn FnMut(&Bindings, &mut Store<State>) -> Guest) -> Done {
        if self.poisoned {
            self.return_lent();
            return Err(CallError::Trap("the instance trapped earlier and cannot run again".into()));
        }
        self.store.data_mut().enter(host);
        self.store.set_epoch_deadline(1);
        let busy = self.ticker.busy();
        let r = f(&self.bindings, &mut self.store);
        drop(busy);
        let timed_out = self.store.data().timed_out;
        self.store.data_mut().leave(host);
        self.return_lent();
        let Err(e) = r else { return Ok(()) };
        self.poisoned = true;
        Err(if timed_out { CallError::Timeout } else { CallError::Trap(trap_text(e)) })
    }

    fn return_lent(&mut self) {
        for rep in std::mem::take(&mut self.store.data_mut().lent) {
            // A handle the guest still held is gone with the call; deleting it can only fail if it was never there.
            let _ = self.store.data_mut().table.delete(Resource::<PlayerRes>::new_own(rep));
        }
    }
}

impl Instance for WasmInstance {
    fn on_enable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        let mut out = Ok(());
        let r = self.call(host, &mut |b, s| match b {
            Bindings::V02(b) => v02::on_enable(b, s).map(|r| out = r),
            Bindings::V01(b) => v01::on_enable(b, s).map(|r| out = r),
        });
        r.and(out.map_err(CallError::Failed))
    }

    fn on_disable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        self.call(host, &mut |b, s| match b {
            Bindings::V02(b) => v02::on_disable(b, s),
            Bindings::V01(b) => v01::on_disable(b, s),
        })
    }

    fn handle_event(
        &mut self,
        host: &mut dyn HostApi,
        sub: SubscriptionId,
        ev: &core::Event,
    ) -> Result<core::Outcome, CallError> {
        let mut out = None;
        self.call(host, &mut |b, s| match b {
            Bindings::V02(b) => {
                let w = v02::event_to(ev, &mut lender(s));
                w.map_or(Ok(()), |w| v02::handle_event(b, s, sub, &w).map(|o| out = Some(o)))
            }
            Bindings::V01(b) => {
                let w = v01::event_to(ev, &mut lender(s));
                w.map_or(Ok(()), |w| v01::handle_event(b, s, sub, &w).map(|o| out = Some(o)))
            }
        })?;
        Ok(out.unwrap_or(core::Outcome { cancelled: ev.cancelled(), mutation: None }))
    }

    fn run_command(
        &mut self,
        host: &mut dyn HostApi,
        id: CommandId,
        sender: &core::CommandSender,
        label: &str,
        args: &[String],
    ) -> Result<core::CommandOutcome, CallError> {
        let mut out = None;
        self.call(host, &mut |b, s| match b {
            Bindings::V02(b) => {
                let w = v02::sender_to(sender, &mut lender(s));
                v02::run_command(b, s, id, w, label, args).map(|o| out = Some(o))
            }
            Bindings::V01(b) => {
                let w = v01::sender_to(sender, &mut lender(s));
                v01::run_command(b, s, id, w, label, args).map(|o| out = Some(o))
            }
        })?;
        Ok(out.expect("set by a successful call"))
    }

    fn run_task(&mut self, host: &mut dyn HostApi, id: TaskId) -> Result<(), CallError> {
        self.call(host, &mut |b, s| match b {
            Bindings::V02(b) => v02::run_task(b, s, id),
            Bindings::V01(b) => v01::run_task(b, s, id),
        })
    }

    fn call_service(
        &mut self,
        host: &mut dyn HostApi,
        name: &str,
        method: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, CallError> {
        let mut out = Ok(Vec::new());
        let r = self.call(host, &mut |b, s| match b {
            Bindings::V02(b) => v02::call_service(b, s, name, method, args).map(|r| out = r),
            Bindings::V01(b) => v01::call_service(b, s, name, method, args).map(|r| out = r),
        });
        r.and(out.map_err(CallError::Failed))
    }

    fn on_channel_message(
        &mut self,
        host: &mut dyn HostApi,
        channel: ChannelId,
        source: &core::ChannelSource,
        data: &[u8],
    ) -> Result<(), CallError> {
        self.call(host, &mut |b, s| match b {
            Bindings::V02(b) => {
                let w = v02::source_to(source, &mut lender(s));
                v02::on_channel_message(b, s, channel, w, data)
            }
            Bindings::V01(b) => {
                let w = v01::source_to(source, &mut lender(s));
                v01::on_channel_message(b, s, channel, w, data)
            }
        })
    }
}
