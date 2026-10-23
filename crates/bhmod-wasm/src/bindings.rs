//! The `blockheads:api` imports implemented on top of [`HostApi`], the guest export calls, and the conversions
//! between the generated WIT types and the core types, for API 0.2 ([`v02`]) and 0.1 ([`v01`]). Public for other
//! hosts of WASM mods (the client loaders); most users only need [`crate::WasmRuntime`].
//!
//! Both versions are generated from one macro, so they cannot drift apart: 0.2 only adds event kinds, mutations
//! and player functions. Every import reaches the host through the [`HostApi`] of the guest call in progress.
//! Outside a call (while the component is instantiated) there is none and every import traps.
#![allow(missing_docs)]

use wasmtime::component::Resource;
use wasmtime::Store;

use bhmod_core::types as core;
use bhmod_core::HostApi;

use crate::state::State;
pub use crate::wit::PlayerRes;

pub type R<T> = wasmtime::Result<T>;

/// Lends a player handle to the guest for the call in progress.
pub type Lend<'a> = &'a mut dyn FnMut(&str) -> Resource<PlayerRes>;

impl State {
    /// The host API of the call in progress, or a trap outside a call (a start function calling an import).
    fn api(&mut self) -> R<&mut dyn HostApi> {
        Ok(self.host_api().ok_or(wasmtime::Trap::CannotEnterComponent)?)
    }

    /// Wraps a player id in a new resource owned by the guest.
    fn new_player(&mut self, id: String) -> R<Resource<PlayerRes>> {
        Ok(self.table.push(PlayerRes(id))?)
    }

    fn player_id(&self, p: &Resource<PlayerRes>) -> R<String> {
        Ok(self.table.get(p)?.0.clone())
    }
}

/// A match between two enums with the same case names, as a function.
macro_rules! enum_fn {
    ($name:ident: $from:ty => $to:ty { $($case:ident),* $(,)? }) => {
        pub fn $name(x: $from) -> $to {
            match x { $(<$from>::$case => <$to>::$case),* }
        }
    };
}

/// Everything that exists in both API versions; `$api` is the generated `blockheads::api` module of one version.
macro_rules! api_version {
    ($api:path; kinds { $($kind:ident),* }; extra { $($extra:tt)* }) => {
        use super::*;
        use $api as api;
        pub use api::{channels, commands, events, host, log, players, scheduler, services, storage, types};

        enum_fn!(side_to: core::Side => types::Side { Server, Client });
        enum_fn!(role_to: core::Role => players::Role { Player, Moderator, Admin, Owner });
        enum_fn!(role_from: players::Role => core::Role { Player, Moderator, Admin, Owner });
        enum_fn!(priority_from: types::Priority => core::Priority { Lowest, Low, Normal, High, Highest, Monitor });
        enum_fn!(level_from: log::Level => core::LogLevel { Trace, Debug, Info, Warn, Error });
        enum_fn!(kind_from: events::EventKind => core::EventKind { $($kind),* });
        enum_fn!(phase_to: core::SavePhase => events::SavePhase { Pre, Post });

        /// Core API error to WIT.
        pub fn api_error(e: core::ApiError) -> types::ApiError {
            match e { core::ApiError::NotPermitted(p) => types::ApiError::NotPermitted(p), core::ApiError::InvalidArgument(a) => types::ApiError::InvalidArgument(a), core::ApiError::LimitExceeded(l) => types::ApiError::LimitExceeded(l), core::ApiError::StaleHandle => types::ApiError::StaleHandle, core::ApiError::Unsupported => types::ApiError::Unsupported, core::ApiError::Internal(m) => types::ApiError::Internal(m) }
        }

        /// Core service error to WIT.
        pub fn service_error(e: core::ServiceError) -> services::CallError {
            match e { core::ServiceError::NotFound => services::CallError::NotFound, core::ServiceError::VersionMismatch => services::CallError::VersionMismatch, core::ServiceError::NotPermitted => services::CallError::NotPermitted, core::ServiceError::Reentrant => services::CallError::Reentrant, core::ServiceError::ProviderError(m) => services::CallError::ProviderError(m), core::ServiceError::Trap => services::CallError::Trap, core::ServiceError::BadValue(m) => services::CallError::BadValue(m) }
        }

        /// WIT command outcome to core.
        pub fn command_outcome_from(o: commands::CommandOutcome) -> core::CommandOutcome {
            core::CommandOutcome { handled: o.handled, reply: o.reply }
        }

        /// Core command sender to WIT.
        pub fn sender_to(s: &core::CommandSender, lend: Lend<'_>) -> events::CommandSender {
            match s { core::CommandSender::Console => events::CommandSender::Console, core::CommandSender::Player(id) => events::CommandSender::Player(lend(id)) }
        }

        /// Core channel source to WIT.
        pub fn source_to(s: &core::ChannelSource, lend: Lend<'_>) -> channels::Source {
            match s { core::ChannelSource::Local(m) => channels::Source::Local(m.clone()), core::ChannelSource::Server => channels::Source::Server, core::ChannelSource::Player(id) => channels::Source::Player(lend(id)) }
        }

        impl types::Host for State {}

        impl host::Host for State {
            fn api_version(&mut self) -> R<String> {
                Ok(self.api()?.api_version())
            }
            fn mod_id(&mut self) -> R<String> {
                Ok(self.api()?.mod_id())
            }
            fn mod_version(&mut self) -> R<String> {
                Ok(self.api()?.mod_version())
            }
            fn current_side(&mut self) -> R<types::Side> {
                Ok(side_to(self.api()?.current_side()))
            }
            fn host_name(&mut self) -> R<String> {
                Ok(self.api()?.host_name())
            }
        }

        impl log::Host for State {
            fn log(&mut self, level: log::Level, message: String) -> R<()> {
                self.api()?.log(level_from(level), &message);
                Ok(())
            }
        }

        impl players::HostPlayer for State {
            fn id(&mut self, p: Resource<PlayerRes>) -> R<String> {
                self.player_id(&p)
            }
            fn alias(&mut self, p: Resource<PlayerRes>) -> R<String> {
                let id = self.player_id(&p)?;
                Ok(self.api()?.player_alias(&id).unwrap_or_default())
            }
            fn is_online(&mut self, p: Resource<PlayerRes>) -> R<bool> {
                let id = self.player_id(&p)?;
                Ok(self.api()?.player_is_online(&id))
            }
            fn role(&mut self, p: Resource<PlayerRes>) -> R<players::Role> {
                let id = self.player_id(&p)?;
                Ok(role_to(self.api()?.player_role(&id).unwrap_or(core::Role::Player)))
            }
            fn send_chat(&mut self, p: Resource<PlayerRes>, text: String) -> R<Result<(), types::ApiError>> {
                let id = self.player_id(&p)?;
                Ok(self.api()?.player_send_chat(&id, &text).map_err(api_error))
            }
            fn kick(&mut self, p: Resource<PlayerRes>, reason: String) -> R<Result<(), types::ApiError>> {
                let id = self.player_id(&p)?;
                Ok(self.api()?.player_kick(&id, &reason).map_err(api_error))
            }
            fn drop(&mut self, p: Resource<PlayerRes>) -> R<()> {
                self.table.delete(p)?;
                Ok(())
            }
            $($extra)*
        }

        impl players::Host for State {
            fn get(&mut self, id: String) -> R<Option<Resource<PlayerRes>>> {
                let found = self.api()?.players_get(&id);
                found.map(|id| self.new_player(id)).transpose()
            }
            fn find_by_alias(&mut self, alias: String) -> R<Option<Resource<PlayerRes>>> {
                let found = self.api()?.players_find_by_alias(&alias);
                found.map(|id| self.new_player(id)).transpose()
            }
            fn online(&mut self) -> R<Vec<Resource<PlayerRes>>> {
                let ids = self.api()?.players_online();
                ids.into_iter().map(|id| self.new_player(id)).collect()
            }
            fn broadcast(&mut self, text: String) -> R<Result<(), types::ApiError>> {
                Ok(self.api()?.players_broadcast(&text).map_err(api_error))
            }
            players_extra!();
        }

        impl events::Host for State {
            fn subscribe(
                &mut self,
                kind: events::EventKind,
                priority: types::Priority,
                ignore_cancelled: bool,
            ) -> R<Result<u32, types::ApiError>> {
                let r = self.api()?.subscribe(kind_from(kind), priority_from(priority), ignore_cancelled);
                Ok(r.map_err(api_error))
            }
            fn subscribe_custom(
                &mut self,
                name: String,
                priority: types::Priority,
                ignore_cancelled: bool,
            ) -> R<Result<u32, types::ApiError>> {
                let r = self.api()?.subscribe_custom(&name, priority_from(priority), ignore_cancelled);
                Ok(r.map_err(api_error))
            }
            fn unsubscribe(&mut self, sub: u32) -> R<bool> {
                Ok(self.api()?.unsubscribe(sub))
            }
            fn fire_custom(&mut self, name: String, payload: Vec<u8>) -> R<Result<bool, types::ApiError>> {
                Ok(self.api()?.fire_custom(&name, &payload).map_err(api_error))
            }
        }

        impl commands::Host for State {
            fn register(&mut self, spec: commands::CommandSpec) -> R<Result<u32, types::ApiError>> {
                let spec = core::CommandSpec {
                    name: spec.name,
                    aliases: spec.aliases,
                    description: spec.description,
                    usage: spec.usage,
                    min_role: role_from(spec.min_role),
                    console: spec.console,
                };
                Ok(self.api()?.register_command(spec).map_err(api_error))
            }
            fn unregister(&mut self, id: u32) -> R<bool> {
                Ok(self.api()?.unregister_command(id))
            }
        }

        impl scheduler::Host for State {
            fn run_later(&mut self, delay: u32) -> R<Result<u64, types::ApiError>> {
                Ok(self.api()?.run_later(delay).map_err(api_error))
            }
            fn run_every(&mut self, delay: u32, period: u32) -> R<Result<u64, types::ApiError>> {
                Ok(self.api()?.run_every(delay, period).map_err(api_error))
            }
            fn run_next_poll(&mut self) -> R<Result<u64, types::ApiError>> {
                Ok(self.api()?.run_next_poll().map_err(api_error))
            }
            fn cancel(&mut self, id: u64) -> R<bool> {
                Ok(self.api()?.cancel_task(id))
            }
        }

        impl storage::Host for State {
            fn get(&mut self, key: String) -> R<Option<Vec<u8>>> {
                Ok(self.api()?.storage_get(&key))
            }
            fn set(&mut self, key: String, value: Vec<u8>) -> R<Result<(), types::ApiError>> {
                Ok(self.api()?.storage_set(&key, &value).map_err(api_error))
            }
            fn delete(&mut self, key: String) -> R<bool> {
                Ok(self.api()?.storage_delete(&key))
            }
            fn keys(&mut self, prefix: String) -> R<Vec<String>> {
                Ok(self.api()?.storage_keys(&prefix))
            }
        }

        impl services::Host for State {
            fn provide(&mut self, name: String, version: String) -> R<Result<(), types::ApiError>> {
                Ok(self.api()?.provide_service(&name, &version).map_err(api_error))
            }
            fn call(
                &mut self,
                name: String,
                version_req: String,
                method: String,
                args: Vec<u8>,
            ) -> R<Result<Vec<u8>, services::CallError>> {
                Ok(self.api()?.call_service(&name, &version_req, &method, &args).map_err(service_error))
            }
            services_extra!();
        }

        impl channels::Host for State {
            fn open(&mut self, name: String) -> R<Result<u32, types::ApiError>> {
                Ok(self.api()?.channel_open(&name).map_err(api_error))
            }
            fn send(&mut self, channel: u32, to: channels::Target, data: Vec<u8>) -> R<Result<(), types::ApiError>> {
                    let to = match to { channels::Target::Local => core::ChannelTarget::Local, channels::Target::Server => core::ChannelTarget::Server, channels::Target::Player(p) => core::ChannelTarget::Player(self.player_id(&p)?), channels::Target::AllPlayers => core::ChannelTarget::AllPlayers };
                Ok(self.api()?.channel_send(channel, to, &data).map_err(api_error))
            }
            fn close(&mut self, channel: u32) -> R<bool> {
                Ok(self.api()?.channel_close(channel))
            }
        }

        /// Adds this version's imports to a linker.
        pub fn add_to_linker(linker: &mut wasmtime::component::Linker<State>) -> R<()> {
            ServerMod::add_to_linker::<_, wasmtime::component::HasSelf<_>>(linker, |s| s)
        }

        /// `guest.on-enable`.
        pub fn on_enable(b: &ServerMod, s: &mut Store<State>) -> R<Result<(), String>> {
            b.blockheads_api_guest().call_on_enable(s)
        }
        /// `guest.on-disable`.
        pub fn on_disable(b: &ServerMod, s: &mut Store<State>) -> R<()> {
            b.blockheads_api_guest().call_on_disable(s)
        }
        /// `guest.handle-event`.
        pub fn handle_event(b: &ServerMod, s: &mut Store<State>, sub: u32, ev: &events::Event) -> R<core::Outcome> {
            b.blockheads_api_guest().call_handle_event(s, sub, ev).map(outcome_from)
        }
        /// `guest.run-command`.
        pub fn run_command(
            b: &ServerMod,
            s: &mut Store<State>,
            id: u32,
            sender: events::CommandSender,
            label: &str,
            args: &[String],
        ) -> R<core::CommandOutcome> {
            b.blockheads_api_guest().call_run_command(s, id, sender, label, args).map(command_outcome_from)
        }
        /// `guest.run-task`.
        pub fn run_task(b: &ServerMod, s: &mut Store<State>, id: u64) -> R<()> {
            b.blockheads_api_guest().call_run_task(s, id)
        }
        /// `guest.call-service`.
        pub fn call_service(
            b: &ServerMod,
            s: &mut Store<State>,
            name: &str,
            method: &str,
            args: &[u8],
        ) -> R<Result<Vec<u8>, String>> {
            b.blockheads_api_guest().call_call_service(s, name, method, &args.to_vec())
        }
        /// `guest.on-channel-message`.
        pub fn on_channel_message(
            b: &ServerMod,
            s: &mut Store<State>,
            channel: u32,
            source: channels::Source,
            data: &[u8],
        ) -> R<()> {
            b.blockheads_api_guest().call_on_channel_message(s, channel, &source, data)
        }
    };
}

/// The events both versions have; evaluates to `Option<events::Event>` (`None` for the others).
macro_rules! common_event_to {
    ($ev:expr, $lend:expr) => {{
        use events as e;
        let lend = $lend;
        Some(match $ev {
            core::Event::PlayerJoin { player } => e::Event::PlayerJoin(e::PlayerJoinEvent { player: lend(&player) }),
            core::Event::PlayerQuit { player, was_kick } => {
                e::Event::PlayerQuit(e::PlayerQuitEvent { player: lend(&player), was_kick })
            }
            core::Event::Chat { player, text, cancelled } => {
                e::Event::Chat(e::ChatEvent { player: lend(&player), text, cancelled })
            }
            core::Event::Command { sender, name, args, raw, cancelled } => {
                e::Event::Command(e::CommandEvent { sender: sender_to(&sender, lend), name, args, raw, cancelled })
            }
            core::Event::Tick { tick, dt } => e::Event::Tick(e::TickEvent { tick, dt }),
            core::Event::WorldLoaded { world_name, world_id } => {
                e::Event::WorldLoaded(e::WorldLoadedEvent { world_name, world_id })
            }
            core::Event::WorldSave { phase } => e::Event::WorldSave(e::WorldSaveEvent { phase: phase_to(phase) }),
            core::Event::Custom { name, source, payload, cancelled } => {
                e::Event::Custom(e::CustomEvent { name, source, payload, cancelled })
            }
            _ => return None,
        })
    }};
}

/// What API 0.2 and 0.3 add to 0.1: the gameplay events and their reject, layer and object enums.
macro_rules! gameplay_api {
    () => {
        enum_fn!(reject_to: core::RejectReason => events::RejectReason { ClientTooOld, ServerFull, Blacklisted,
            NotWhitelisted, IncorrectPassword, Kicked, Banned, InvalidUsername, StillLoading });
        enum_fn!(reject_from: events::RejectReason => core::RejectReason { ClientTooOld, ServerFull, Blacklisted,
            NotWhitelisted, IncorrectPassword, Kicked, Banned, InvalidUsername, StillLoading });
        enum_fn!(layer_to: core::BreakLayer => events::BreakLayer { Block, BackWall });
        enum_fn!(object_to: core::ObjectKind => events::ObjectKind { Workbench, InteractionObject });

        fn pos_to(p: core::Pos) -> types::Pos {
            types::Pos { x: p.x, y: p.y }
        }

        /// WIT handler outcome to core.
        pub fn outcome_from(o: events::Outcome) -> core::Outcome {
            #[rustfmt::skip]
            let mutation = o.mutation.map(|m| match m { events::Mutation::ChatText(t) => core::Mutation::ChatText(t), events::Mutation::CustomPayload(p) => core::Mutation::CustomPayload(p), events::Mutation::RejectReason(r) => core::Mutation::RejectReason(reject_from(r)), events::Mutation::DamageAmount(a) => core::Mutation::DamageAmount(a) });
            core::Outcome { cancelled: o.cancelled, mutation }
        }

        /// Core event to WIT; players are lent for the call.
        pub fn event_to(ev: &core::Event, lend: Lend<'_>) -> Option<events::Event> {
            use events as e;
            Some(match ev.clone() {
                core::Event::PlayerPreLogin { player, alias, cancelled, reason } => {
                    e::Event::PlayerPreLogin(e::PlayerPreLoginEvent {
                        player_id: player,
                        alias,
                        cancelled,
                        reason: reject_to(reason),
                    })
                }
                core::Event::BlockPlace { player, pos, item, data_a, data_b, cancelled } => {
                    e::Event::BlockPlace(e::BlockPlaceEvent {
                        player: lend(&player),
                        at: pos_to(pos),
                        item,
                        data_a,
                        data_b,
                        cancelled,
                    })
                }
                core::Event::BlockBreak { player, pos, layer, tile, cancelled } => {
                    e::Event::BlockBreak(e::BlockBreakEvent {
                        player: lend(&player),
                        at: pos_to(pos),
                        layer: layer_to(layer),
                        tile,
                        cancelled,
                    })
                }
                core::Event::BlockChanged { pos, tile, player } => {
                    e::Event::BlockChanged(e::BlockChangedEvent { at: pos_to(pos), tile, player: player.map(|p| lend(&p)) })
                }
                core::Event::ObjectPlace { player, pos, object, item, cancelled } => {
                    e::Event::ObjectPlace(e::ObjectPlaceEvent {
                        player: lend(&player),
                        at: pos_to(pos),
                        object: object_to(object),
                        item,
                        cancelled,
                    })
                }
                core::Event::Damage { player, blockhead, amount, attacker, cancelled } => {
                    let player = player.map(|p| lend(&p));
                    e::Event::Damage(e::DamageEvent {
                        player,
                        blockhead,
                        amount,
                        attacker: attacker.map(|p| lend(&p)),
                        cancelled,
                    })
                }
                core::Event::Death { player, blockhead } => {
                    e::Event::Death(e::DeathEvent { player: player.map(|p| lend(&p)), blockhead })
                }
                core::Event::PlayerMove { player, from, to } => {
                    e::Event::PlayerMove(e::PlayerMoveEvent { player: lend(&player), from: pos_to(from), to: pos_to(to) })
                }
                other => return common_event_to!(other, lend),
            })
        }
    };
}

/// `blockheads:api@0.2.0`.
pub mod v02 {
    pub use crate::wit::v02::ServerMod;

    macro_rules! services_extra {
        () => {};
    }

    macro_rules! players_extra {
        () => {
            fn from_value_handle(&mut self, handle: u64) -> R<Option<Resource<PlayerRes>>> {
                let found = self.api()?.player_of_handle(handle).ok();
                found.map(|id| self.new_player(id)).transpose()
            }
        };
    }

    api_version!(crate::wit::v02::blockheads::api;
    kinds { PlayerJoin, PlayerQuit, Chat, Command, Tick, WorldLoaded, WorldSave, PlayerPreLogin, BlockPlace,
        BlockBreak, BlockChanged, ObjectPlace, Damage, Death, PlayerMove };
    extra {
        fn pos(&mut self, p: Resource<PlayerRes>) -> R<Option<types::Pos>> {
            let id = self.player_id(&p)?;
            let pos = self.api()?.player_pos(&id).ok().flatten();
            Ok(pos.map(|p| types::Pos { x: p.x, y: p.y }))
        }
        fn value_handle(&mut self, p: Resource<PlayerRes>) -> R<u64> {
            let id = self.player_id(&p)?;
            Ok(self.api()?.handle_of_player(&id))
        }
    });

    gameplay_api!();
}

/// `blockheads:api@0.3.1`: 0.2 plus the `ui` interface (only client mods import it) and the `content` interface. Components of either world
/// are bound with the `client-mod` world (a superset of the imports, the same `guest` export).
pub mod v03 {
    pub use crate::wit::ClientMod as ServerMod;

    /// The remote service calls added in 0.3.1 (a 0.3.0 component never imports them).
    macro_rules! services_extra {
        () => {
            fn call_remote(
                &mut self,
                to: services::RemoteTarget,
                name: String,
                version_req: String,
                method: String,
                args: Vec<u8>,
                timeout_ms: u32,
            ) -> R<Result<u32, types::ApiError>> {
                let to = match to {
                    services::RemoteTarget::Server => bhmod_core::RemoteTarget::Server,
                    services::RemoteTarget::Player(p) => bhmod_core::RemoteTarget::Player(self.player_id(&p)?),
                };
                Ok(self.api()?.call_remote(to, &name, &version_req, &method, &args, timeout_ms).map_err(api_error))
            }
            fn cancel_remote(&mut self, id: u32) -> R<bool> {
                Ok(self.api()?.cancel_remote(id))
            }
            fn caller(&mut self) -> R<Option<services::RemoteCaller>> {
                let Some(c) = self.api()?.remote_caller() else { return Ok(None) };
                let player = c.player.map(|id| self.new_player(id)).transpose()?;
                Ok(Some(services::RemoteCaller { mod_id: c.mod_id, player }))
            }
        };
    }

    macro_rules! players_extra {
        () => {
            fn from_value_handle(&mut self, handle: u64) -> R<Option<Resource<PlayerRes>>> {
                let found = self.api()?.player_of_handle(handle).ok();
                found.map(|id| self.new_player(id)).transpose()
            }
        };
    }

    api_version!(crate::wit::blockheads::api;
    kinds { PlayerJoin, PlayerQuit, Chat, Command, Tick, WorldLoaded, WorldSave, PlayerPreLogin, BlockPlace,
        BlockBreak, BlockChanged, ObjectPlace, Damage, Death, PlayerMove };
    extra {
        fn pos(&mut self, p: Resource<PlayerRes>) -> R<Option<types::Pos>> {
            let id = self.player_id(&p)?;
            let pos = self.api()?.player_pos(&id).ok().flatten();
            Ok(pos.map(|p| types::Pos { x: p.x, y: p.y }))
        }
        fn value_handle(&mut self, p: Resource<PlayerRes>) -> R<u64> {
            let id = self.player_id(&p)?;
            Ok(self.api()?.handle_of_player(&id))
        }
    });

    gameplay_api!();

    use crate::wit::blockheads::api::ui;
    use bhmod_core::ui as core_ui;

    enum_fn!(slot_from: ui::Slot => core_ui::Slot { TopLeft, TopRight });
    enum_fn!(widget_kind_from: ui::WidgetKind => core_ui::WidgetKind { Text, Counter, Panel });
    enum_fn!(notice_level_from: ui::NoticeLevel => core_ui::NoticeLevel { Info, Warning, Success, Error });
    enum_fn!(button_kind_from: ui::ButtonKind => core_ui::ButtonKind { Primary, Secondary, Danger });

    impl ui::Host for State {
        fn set_widget(&mut self, id: String, w: ui::Widget) -> R<Result<(), types::ApiError>> {
            let w = core_ui::Widget {
                slot: slot_from(w.slot),
                kind: widget_kind_from(w.kind),
                icon: w.icon,
                title: w.title,
                text: w.text,
            };
            Ok(self.api()?.ui_set_widget(&id, w).map_err(api_error))
        }
        fn remove_widget(&mut self, id: String) -> R<bool> {
            Ok(self.api()?.ui_remove_widget(&id))
        }
        fn toast(&mut self, text: String) -> R<Result<(), types::ApiError>> {
            Ok(self.api()?.ui_toast(&text).map_err(api_error))
        }
        fn notice(&mut self, text: String, level: ui::NoticeLevel) -> R<Result<(), types::ApiError>> {
            Ok(self.api()?.ui_notice(&text, notice_level_from(level)).map_err(api_error))
        }
        fn show_dialog(&mut self, d: ui::Dialog) -> R<Result<u32, types::ApiError>> {
            let d = core_ui::Dialog {
                title: d.title,
                text: d.text,
                field: d.field.map(|f| core_ui::Field {
                    placeholder: f.placeholder,
                    initial: f.initial,
                    max_length: f.max_length,
                }),
                buttons: d
                    .buttons
                    .into_iter()
                    .map(|b| core_ui::DialogButton { label: b.label, kind: button_kind_from(b.kind) })
                    .collect(),
            };
            Ok(self.api()?.ui_show_dialog(d).map_err(api_error))
        }
        fn close_dialog(&mut self, id: u32) -> R<bool> {
            Ok(self.api()?.ui_close_dialog(id))
        }
    }

    use crate::wit::blockheads::api::content;

    enum_fn!(content_kind_from: content::Kind => core::ContentKind { Block, Item, Cosmetic });

    impl content::Host for State {
        fn id_of(&mut self, kind: content::Kind, key: String) -> R<Option<u32>> {
            let id = self.api()?.content_id(content_kind_from(kind), &key);
            Ok(id.and_then(|id| u32::try_from(id).ok()))
        }
        fn key_of(&mut self, kind: content::Kind, id: u32) -> R<Option<String>> {
            Ok(self.api()?.content_key(content_kind_from(kind), u64::from(id)))
        }
        fn cosmetics(&mut self) -> R<Vec<content::CosmeticInfo>> {
            let list = self.api()?.content_cosmetics();
            Ok(list
                .into_iter()
                .map(|c| content::CosmeticInfo {
                    key: c.key,
                    mod_id: c.mod_id,
                    slot: c.slot,
                    id: c.id.and_then(|id| u32::try_from(id).ok()),
                })
                .collect())
        }
        fn equipped(&mut self, p: Resource<PlayerRes>) -> R<Result<Vec<(String, String)>, types::ApiError>> {
            let id = self.player_id(&p)?;
            Ok(self.api()?.content_equipped(&id).map_err(api_error))
        }
        fn equip(&mut self, p: Resource<PlayerRes>, key: String) -> R<Result<(), types::ApiError>> {
            let id = self.player_id(&p)?;
            Ok(self.api()?.content_equip(&id, &key).map_err(api_error))
        }
        fn unequip(&mut self, p: Resource<PlayerRes>, slot: String) -> R<Result<(), types::ApiError>> {
            let id = self.player_id(&p)?;
            Ok(self.api()?.content_unequip(&id, &slot).map_err(api_error))
        }
    }
}

/// `blockheads:api@0.1.0`: the events of 0.2 that 0.1 does not know are never delivered (0.1 mods cannot
/// subscribe to them).
pub mod v01 {
    pub use crate::wit::v01::ServerMod;

    macro_rules! services_extra {
        () => {};
    }

    macro_rules! players_extra {
        () => {};
    }

    api_version!(crate::wit::v01::blockheads::api;
        kinds { PlayerJoin, PlayerQuit, Chat, Command, Tick, WorldLoaded, WorldSave };
        extra {});

    /// WIT handler outcome to core.
    pub fn outcome_from(o: events::Outcome) -> core::Outcome {
        #[rustfmt::skip]
        let mutation = o.mutation.map(|m| match m { events::Mutation::ChatText(t) => core::Mutation::ChatText(t), events::Mutation::CustomPayload(p) => core::Mutation::CustomPayload(p) });
        core::Outcome { cancelled: o.cancelled, mutation }
    }

    /// Core event to WIT, `None` for events 0.1 does not have.
    pub fn event_to(ev: &core::Event, lend: Lend<'_>) -> Option<events::Event> {
        let ev = ev.clone();
        common_event_to!(ev, lend)
    }
}
