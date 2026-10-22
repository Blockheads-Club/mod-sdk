//! One instantiated component: the WIT `guest` export behind [`Instance`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use wasmtime::component::{Component, Linker, Resource};
use wasmtime::{Store, Trap, UpdateDeadline};

use bhmod_core::runtime::{CallError, CommandId, HostApi, Instance, InstanceLimits, SubscriptionId, TaskId};
use bhmod_core::types as core;

use crate::bindings::{self, events, PlayerRes, ServerMod};
use crate::state::State;
use crate::ticker::Ticker;

/// How long instantiation (core module start functions) may run.
pub const INSTANTIATE_BUDGET: Duration = Duration::from_millis(500);

/// One WASM mod.
pub struct WasmInstance {
    store: Store<State>,
    bindings: ServerMod,
    ticker: Arc<Ticker>,
    /// Player handles lent to the guest for the call in progress.
    lent: Vec<u32>,
    /// Set after a trap: the component model forbids entering the instance again.
    poisoned: bool,
}

fn trap_text(e: wasmtime::Error) -> String {
    match e.downcast_ref::<Trap>() {
        Some(t) => t.to_string(),
        None => format!("{e:#}"),
    }
}

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
        let bindings = ServerMod::instantiate(&mut store, component, linker).map_err(trap_text);
        drop(busy);
        store.data_mut().deadline = None;
        Ok(WasmInstance { store, bindings: bindings?, ticker, lent: Vec::new(), poisoned: false })
    }

    /// Lends a player handle to the guest for the call in progress.
    fn lend(&mut self, id: &str) -> Resource<PlayerRes> {
        // The table only fails past u32::MAX live entries.
        let r = self.store.data_mut().table.push(PlayerRes(id.to_string())).expect("resource table full");
        self.lent.push(r.rep());
        Resource::new_borrow(r.rep())
    }

    fn sender(&mut self, s: &core::CommandSender) -> events::CommandSender {
        match s {
            core::CommandSender::Console => events::CommandSender::Console,
            core::CommandSender::Player(id) => events::CommandSender::Player(self.lend(id)),
        }
    }

    fn event(&mut self, ev: &core::Event) -> events::Event {
        match ev {
            core::Event::PlayerJoin { player } => {
                events::Event::PlayerJoin(events::PlayerJoinEvent { player: self.lend(player) })
            }
            core::Event::PlayerQuit { player, was_kick } => {
                events::Event::PlayerQuit(events::PlayerQuitEvent { player: self.lend(player), was_kick: *was_kick })
            }
            core::Event::Chat { player, text, cancelled } => events::Event::Chat(events::ChatEvent {
                player: self.lend(player),
                text: text.clone(),
                cancelled: *cancelled,
            }),
            core::Event::Command { sender, name, args, raw, cancelled } => {
                events::Event::Command(events::CommandEvent {
                    sender: self.sender(sender),
                    name: name.clone(),
                    args: args.clone(),
                    raw: raw.clone(),
                    cancelled: *cancelled,
                })
            }
            core::Event::Tick { tick, dt } => events::Event::Tick(events::TickEvent { tick: *tick, dt: *dt }),
            core::Event::WorldLoaded { world_name, world_id } => events::Event::WorldLoaded(events::WorldLoadedEvent {
                world_name: world_name.clone(),
                world_id: world_id.clone(),
            }),
            core::Event::WorldSave { phase } => events::Event::WorldSave(events::WorldSaveEvent {
                phase: match phase {
                    core::SavePhase::Pre => events::SavePhase::Pre,
                    core::SavePhase::Post => events::SavePhase::Post,
                },
            }),
        }
    }

    /// Runs one guest call under the host's deadline and maps traps and timeouts.
    fn call(&mut self, host: &mut dyn HostApi, f: &mut dyn FnMut(&ServerMod, &mut Store<State>) -> Guest) -> Done {
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
        for rep in std::mem::take(&mut self.lent) {
            // A handle the guest still held is gone with the call; deleting it can only fail if it was never there.
            let _ = self.store.data_mut().table.delete(Resource::<PlayerRes>::new_own(rep));
        }
    }
}

type Guest = wasmtime::Result<()>;
type Done = Result<(), CallError>;

impl Instance for WasmInstance {
    fn on_enable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        let mut out = Ok(());
        self.call(host, &mut |b, s| b.blockheads_api_guest().call_on_enable(s).map(|r| out = r))?;
        out.map_err(CallError::Failed)
    }

    fn on_disable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        self.call(host, &mut |b, s| b.blockheads_api_guest().call_on_disable(s))
    }

    fn handle_event(
        &mut self,
        host: &mut dyn HostApi,
        sub: SubscriptionId,
        ev: &core::Event,
    ) -> Result<core::Outcome, CallError> {
        let ev = self.event(ev);
        let mut out = None;
        self.call(host, &mut |b, s| b.blockheads_api_guest().call_handle_event(s, sub, &ev).map(|o| out = Some(o)))?;
        Ok(bindings::outcome_from(out.expect("set by a successful call")))
    }

    fn run_command(
        &mut self,
        host: &mut dyn HostApi,
        id: CommandId,
        sender: &core::CommandSender,
        label: &str,
        args: &[String],
    ) -> Result<core::CommandOutcome, CallError> {
        let mut sender = Some(self.sender(sender));
        let mut out = None;
        self.call(host, &mut |b, s| {
            let sender = sender.take().expect("called once");
            b.blockheads_api_guest().call_run_command(s, id, sender, label, args).map(|o| out = Some(o))
        })?;
        Ok(bindings::command_outcome_from(out.expect("set by a successful call")))
    }

    fn run_task(&mut self, host: &mut dyn HostApi, id: TaskId) -> Result<(), CallError> {
        self.call(host, &mut |b, s| b.blockheads_api_guest().call_run_task(s, id))
    }
}
