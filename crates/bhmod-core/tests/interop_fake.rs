//! API 0.2 host behaviour that Luau cannot reach (a WASM runtime can): raw CBOR checks on service values, custom
//! kind subscriptions, and the defaults of the runtime and bridge traits that keep 0.1 code compiling.

mod common;

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;
use std::time::Instant;

use bhmod_core::channels::ChannelId;
use bhmod_core::runtime::{CommandId, InstanceLimits, ModFiles, SubscriptionId, TaskId};
use bhmod_core::types::*;
use bhmod_core::value::Value;
use bhmod_core::{CallError, GameBridge, HostApi, HostOptions, Instance, ModHost, Runtime};
use common::*;

type Script = Rc<dyn Fn(&mut dyn HostApi) -> Vec<String>>;

/// Runs `enable` in on-enable, `script` as its command handler, and answers service calls with raw bytes.
struct Raw {
    enable: Script,
    script: Option<Script>,
    reply: Vec<u8>,
}

impl Instance for Raw {
    fn on_enable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        (self.enable)(host);
        Ok(())
    }
    fn on_disable(&mut self, _: &mut dyn HostApi) -> Result<(), CallError> {
        Ok(())
    }
    fn handle_event(&mut self, _: &mut dyn HostApi, _: SubscriptionId, _: &Event) -> Result<Outcome, CallError> {
        Ok(Outcome::default())
    }
    fn run_command(
        &mut self,
        host: &mut dyn HostApi,
        _: CommandId,
        _: &CommandSender,
        _: &str,
        _: &[String],
    ) -> Result<CommandOutcome, CallError> {
        let reply = self.script.as_ref().map(|s| s(host)).unwrap_or_default();
        Ok(CommandOutcome { handled: true, reply })
    }
    fn run_task(&mut self, _: &mut dyn HostApi, _: TaskId) -> Result<(), CallError> {
        Ok(())
    }
    fn call_service(&mut self, _: &mut dyn HostApi, _: &str, method: &str, _: &[u8]) -> Result<Vec<u8>, CallError> {
        if method == "panic" {
            panic!("provider panicked");
        }
        Ok(self.reply.clone())
    }
}

/// Panics in every event and channel callback.
struct Panicky;

impl Instance for Panicky {
    fn on_enable(&mut self, h: &mut dyn HostApi) -> Result<(), CallError> {
        h.subscribe_custom("user:x", Priority::Normal, false).unwrap();
        h.subscribe_custom("panicky:y", Priority::Normal, false).unwrap();
        h.register_command(CommandSpec::new("boom")).unwrap();
        let ch = h.channel_open("panicky:c").unwrap();
        h.channel_send(ch, ChannelTarget::Local, b"x").unwrap();
        Ok(())
    }
    fn on_disable(&mut self, _: &mut dyn HostApi) -> Result<(), CallError> {
        Ok(())
    }
    fn handle_event(&mut self, _: &mut dyn HostApi, _: SubscriptionId, _: &Event) -> Result<Outcome, CallError> {
        panic!("event")
    }
    fn run_command(
        &mut self,
        h: &mut dyn HostApi,
        _: CommandId,
        _: &CommandSender,
        _: &str,
        _: &[String],
    ) -> Result<CommandOutcome, CallError> {
        // Its own handler of this event runs deferred, after this call.
        assert_eq!(h.fire_custom("panicky:y", &[0xf6]), Ok(false));
        Ok(CommandOutcome::default())
    }
    fn run_task(&mut self, _: &mut dyn HostApi, _: TaskId) -> Result<(), CallError> {
        Ok(())
    }
    fn on_channel_message(
        &mut self,
        _: &mut dyn HostApi,
        _: ChannelId,
        _: &ChannelSource,
        _: &[u8],
    ) -> Result<(), CallError> {
        panic!("channel")
    }
}

/// Uses every default of [`Instance`].
struct Plain;

impl Instance for Plain {
    fn on_enable(&mut self, _: &mut dyn HostApi) -> Result<(), CallError> {
        Ok(())
    }
    fn on_disable(&mut self, _: &mut dyn HostApi) -> Result<(), CallError> {
        Ok(())
    }
    fn handle_event(&mut self, _: &mut dyn HostApi, _: SubscriptionId, _: &Event) -> Result<Outcome, CallError> {
        Ok(Outcome::default())
    }
    fn run_command(
        &mut self,
        _: &mut dyn HostApi,
        _: CommandId,
        _: &CommandSender,
        _: &str,
        _: &[String],
    ) -> Result<CommandOutcome, CallError> {
        Ok(CommandOutcome::default())
    }
    fn run_task(&mut self, _: &mut dyn HostApi, _: TaskId) -> Result<(), CallError> {
        Ok(())
    }
}

type Prepared = (String, Box<dyn Instance>);

#[derive(Clone, Default)]
struct Rt {
    made: Rc<RefCell<Vec<Prepared>>>,
}

impl Runtime for Rt {
    fn kind(&self) -> bhmod_manifest::manifest::Runtime {
        bhmod_manifest::manifest::Runtime::Wasm
    }
    fn instantiate(&self, mod_id: &str, _: &ModFiles, _: InstanceLimits) -> Result<Box<dyn Instance>, String> {
        let mut m = self.made.borrow_mut();
        let i = m.iter().position(|(id, _)| id == mod_id).expect("instance prepared");
        Ok(m.remove(i).1)
    }
}

fn wasm_mod(mods: &std::path::Path, id: &str, extra: &str) {
    let d = mods.join(id);
    fs::create_dir_all(d.join("server")).unwrap();
    fs::create_dir_all(d.join("wit")).unwrap();
    fs::write(d.join("wit/svc.wit"), "package x:y;\n").unwrap();
    fs::write(
        d.join("mod.toml"),
        format!(
            "schema = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = {{ en = \"N\" }}\nauthors = [\"a\"]\nlicense = \"MIT\"\napi = \"^0.2\"\n[sides]\nserver = true\nclient = false\n[entry]\nserver = {{ runtime = \"wasm\", path = \"server/mod.wasm\" }}\n{extra}"
        ),
    )
    .unwrap();
    fs::write(d.join("server/mod.wasm"), b"\0asm\x0d\x00\x01\x00").unwrap();
}

#[test]
fn raw_values_and_custom_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    wasm_mod(
        &mods,
        "prov",
        "[provides]\nservices = [{ name = \"svc\", version = \"1.0.0\", wit = \"wit/svc.wit\" }]\n",
    );
    wasm_mod(
        &mods,
        "plain",
        "[provides]\nservices = [{ name = \"plainsvc\", version = \"1.0.0\", wit = \"wit/svc.wit\" }]\n",
    );
    wasm_mod(&mods, "user", "[dependencies]\nprov = \"^1\"\nplain = \"^1\"\n");
    let rt = Rt::default();
    let script: Script = Rc::new(|h: &mut dyn HostApi| {
        let list = Value::List(vec![]).encode();
        vec![
            format!("{:?}", h.subscribe(EventKind::Custom, Priority::Normal, false)),
            format!("{:?}", h.call_service("svc", "^1", "x", &Value::Int(1).encode())),
            format!("{:?}", h.call_service("svc", "^1", "x", &[0xff])),
            format!("{:?}", h.call_service("svc", "^1", "x", &list)),
            format!("{:?}", h.call_service("plainsvc", "^1", "x", &list)),
            format!("{:?}", h.fire_custom("user:x", &[0xff])),
            format!("{:?}", h.call_service("svc", "^1", "panic", &list)),
        ]
    });
    let reg: Script = Rc::new(|h: &mut dyn HostApi| {
        h.register_command(CommandSpec::new("go")).unwrap();
        vec![]
    });
    let prov: Script = Rc::new(|h: &mut dyn HostApi| {
        h.provide_service("svc", "1.0.0").unwrap();
        vec![]
    });
    rt.made.borrow_mut().push(("prov".into(), Box::new(Raw { enable: prov, script: None, reply: vec![0xff] })));
    rt.made.borrow_mut().push(("user".into(), Box::new(Raw { enable: reg, script: Some(script), reply: vec![] })));
    rt.made.borrow_mut().push(("plain".into(), Box::new(Plain)));
    let b = FakeBridge::new();
    let mut o = HostOptions::new(&mods, Side::Server);
    o.runtimes = vec![Box::new(rt.clone())];
    let mut h = ModHost::load(o, b.clone());
    h.enable("W", "w");
    assert_eq!(h.mods().len(), 3, "{}", b.log_text());
    assert_eq!(
        h.command(CommandSender::Console, "/go").unwrap(),
        vec![
            "Err(InvalidArgument(\"subscribe to custom events by name\"))",
            "Err(BadValue(\"arguments must be a list\"))",
            "Err(BadValue(\"arguments: unsupported simple value\"))",
            "Err(BadValue(\"result: unsupported simple value\"))",
            "Err(NotFound)",
            "Err(InvalidArgument(\"payload: unsupported simple value\"))",
            "Err(Trap)",
        ],
        "{}",
        b.log_text()
    );
}

struct NullApi;

impl HostApi for NullApi {
    fn deadline(&self) -> Instant {
        unreachable!()
    }
    fn api_version(&self) -> String {
        unreachable!()
    }
    fn mod_id(&self) -> String {
        unreachable!()
    }
    fn mod_version(&self) -> String {
        unreachable!()
    }
    fn current_side(&self) -> Side {
        unreachable!()
    }
    fn host_name(&self) -> String {
        unreachable!()
    }
    fn log(&mut self, _: LogLevel, _: &str) {}
    fn player_alias(&mut self, _: &str) -> Result<String, ApiError> {
        unreachable!()
    }
    fn player_is_online(&mut self, _: &str) -> bool {
        unreachable!()
    }
    fn player_role(&mut self, _: &str) -> Result<Role, ApiError> {
        unreachable!()
    }
    fn player_send_chat(&mut self, _: &str, _: &str) -> Result<(), ApiError> {
        unreachable!()
    }
    fn player_kick(&mut self, _: &str, _: &str) -> Result<(), ApiError> {
        unreachable!()
    }
    fn players_get(&mut self, _: &str) -> Option<String> {
        unreachable!()
    }
    fn players_find_by_alias(&mut self, _: &str) -> Option<String> {
        unreachable!()
    }
    fn players_online(&mut self) -> Vec<String> {
        unreachable!()
    }
    fn players_broadcast(&mut self, _: &str) -> Result<(), ApiError> {
        unreachable!()
    }
    fn subscribe(&mut self, _: EventKind, _: Priority, _: bool) -> Result<SubscriptionId, ApiError> {
        unreachable!()
    }
    fn unsubscribe(&mut self, _: SubscriptionId) -> bool {
        unreachable!()
    }
    fn register_command(&mut self, _: CommandSpec) -> Result<CommandId, ApiError> {
        unreachable!()
    }
    fn unregister_command(&mut self, _: CommandId) -> bool {
        unreachable!()
    }
    fn run_later(&mut self, _: u32) -> Result<TaskId, ApiError> {
        unreachable!()
    }
    fn run_every(&mut self, _: u32, _: u32) -> Result<TaskId, ApiError> {
        unreachable!()
    }
    fn run_next_poll(&mut self) -> Result<TaskId, ApiError> {
        unreachable!()
    }
    fn cancel_task(&mut self, _: TaskId) -> bool {
        unreachable!()
    }
    fn storage_get(&mut self, _: &str) -> Option<Vec<u8>> {
        unreachable!()
    }
    fn storage_set(&mut self, _: &str, _: &[u8]) -> Result<(), ApiError> {
        unreachable!()
    }
    fn storage_delete(&mut self, _: &str) -> bool {
        unreachable!()
    }
    fn storage_keys(&mut self, _: &str) -> Vec<String> {
        unreachable!()
    }
}

struct NullBridge;

impl GameBridge for NullBridge {
    fn side(&self) -> Side {
        Side::Client
    }
    fn host_name(&self) -> String {
        String::new()
    }
    fn log(&self, _: Option<&str>, _: LogLevel, _: &str) {}
    fn online_players(&self) -> Vec<String> {
        vec![]
    }
    fn player_alias(&self, _: &str) -> Option<String> {
        None
    }
    fn player_role(&self, _: &str) -> Role {
        Role::Player
    }
    fn send_chat(&self, _: &str, _: &str) -> bool {
        false
    }
    fn broadcast(&self, _: &str) {}
    fn kick(&self, _: &str, _: &str) -> bool {
        false
    }
    fn is_builtin_command(&self, _: &str) -> bool {
        false
    }
}

#[test]
fn panics_in_deferred_and_channel_deliveries_are_faults() {
    let tmp = tempfile::tempdir().unwrap();
    let mods = tmp.path().join("mods");
    wasm_mod(&mods, "panicky", "[permissions]\nchannels = true\n");
    fs::write(mods.join("mods.toml"), "[panicky]\ngrant = [\"channels\"]\n").unwrap();
    let rt = Rt::default();
    rt.made.borrow_mut().push(("panicky".into(), Box::new(Panicky)));
    let b = FakeBridge::new();
    let mut o = HostOptions::new(&mods, Side::Server);
    o.runtimes = vec![Box::new(rt.clone())];
    let mut h = ModHost::load(o, b.clone());
    h.enable("W", "w");
    // A message from the network on its channel, and one of its own custom events received while it runs.
    h.fire(Event::Custom { name: "user:x".into(), source: "user".into(), payload: vec![0xf6], cancelled: false });
    assert!(b.has_log("user:x handler: trap: panic: event"), "{}", b.log_text());
    h.command(CommandSender::Console, "/boom");
    assert!(b.has_log("custom handler: trap: panic: event"), "{}", b.log_text());
    // The third fault disables it; the fourth queued message finds no instance and is dropped.
    for _ in 0..2 {
        h.channel_receive("panicky:c", ChannelSource::Server, vec![1]);
    }
    h.poll();
    assert!(b.has_log("channel message: trap: panic: channel"), "{}", b.log_text());
    assert!(matches!(&h.mods()[0].state, bhmod_core::ModState::Disabled(_)), "{}", b.log_text());
}

#[test]
fn trait_defaults_for_0_1_code() {
    let mut api = NullApi;
    assert_eq!(api.player_pos("p"), Err(ApiError::Unsupported));
    assert_eq!(api.handle_of_player("p"), 0);
    assert_eq!(api.player_of_handle(1), Err(ApiError::StaleHandle));
    assert_eq!(api.subscribe_custom("a:b", Priority::Normal, false), Err(ApiError::Unsupported));
    assert_eq!(api.fire_custom("a:b", &[0xf6]), Err(ApiError::Unsupported));
    assert_eq!(api.provide_service("s", "1.0.0"), Err(ApiError::Unsupported));
    assert_eq!(api.call_service("s", "^1", "m", &[0x80]), Err(ServiceError::NotFound));
    assert_eq!(api.channel_open("a:b"), Err(ApiError::Unsupported));
    assert_eq!(api.channel_send(1 as ChannelId, ChannelTarget::Local, b""), Err(ApiError::Unsupported));
    assert!(!api.channel_close(1));
    use bhmod_core::types::ContentKind;
    assert_eq!(api.content_id(ContentKind::Cosmetic, "a:b"), None);
    assert_eq!(api.content_key(ContentKind::Cosmetic, 1), None);
    assert!(api.content_cosmetics().is_empty());
    assert_eq!(api.content_equipped("p"), Err(ApiError::Unsupported));
    assert_eq!(api.content_equip("p", "a:b"), Err(ApiError::Unsupported));
    assert_eq!(api.content_unequip("p", "hat"), Err(ApiError::Unsupported));
    assert_eq!(api.content_give_item("p", "a:b", 1), Err(ApiError::Unsupported));
    assert_eq!(api.content_place_block(0, 0, "a:b"), Err(ApiError::Unsupported));
    assert_eq!(api.content_block_at(0, 0), None);
    let widget = bhmod_core::ui::Widget {
        slot: bhmod_core::ui::Slot::TopLeft,
        kind: bhmod_core::ui::WidgetKind::Text,
        icon: None,
        title: None,
        text: String::new(),
    };
    let dialog = bhmod_core::ui::Dialog { title: String::new(), text: String::new(), field: None, buttons: vec![] };
    assert_eq!(api.ui_set_widget("w", widget), Err(ApiError::Unsupported));
    assert!(!api.ui_remove_widget("w"));
    assert_eq!(api.ui_toast("t"), Err(ApiError::Unsupported));
    assert_eq!(api.ui_notice("t", bhmod_core::ui::NoticeLevel::Info), Err(ApiError::Unsupported));
    assert_eq!(api.ui_show_dialog(dialog), Err(ApiError::Unsupported));
    assert!(!api.ui_close_dialog(1));
    let to = bhmod_core::RemoteTarget::Server;
    assert_eq!(api.call_remote(to, "s", "^1", "m", &[0x80], 0), Err(ApiError::Unsupported));
    assert!(!api.cancel_remote(1));
    assert_eq!(api.remote_caller(), None);
    let mut p = Plain;
    assert!(
        matches!(p.call_service(&mut api, "s", "m", &[0x80]), Err(CallError::Failed(m)) if m.contains("does not implement"))
    );
    assert_eq!(p.on_channel_message(&mut api, 1, &ChannelSource::Server, b"x"), Ok(()));
    let nb = NullBridge;
    assert_eq!(nb.player_pos("p"), None);
    assert_eq!(nb.channel_send("m", "m:x", &ChannelTarget::Server, b""), Err(ApiError::Unsupported));
    assert_eq!(nb.ui("m", &bhmod_core::ui::UiCommand::ClearMod), Err(ApiError::Unsupported));
    let call = bhmod_core::RemoteCall {
        id: 1,
        target: bhmod_core::RemoteTarget::Server,
        mod_id: "m".into(),
        service: "s".into(),
        version_req: "*".into(),
        method: "x".into(),
        args: vec![0x80],
    };
    assert_eq!(nb.service_send(&call), Err(bhmod_core::RemoteError::Unavailable));
}
