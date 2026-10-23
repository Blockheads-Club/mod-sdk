//! Runs the mod host with the JVM runtime adopted from ART (the VM this library is loaded into) and a Java mod whose
//! jar carries `classes.dex`, and returns a transcript. See run.sh.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use bhmod_core::jvm::{attach_existing, JvmConfig};
use bhmod_core::types::{CommandSender, Event, LogLevel, Role, Side};
use bhmod_core::{GameBridge, HostOptions, ModHost};
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;

#[derive(Default)]
struct Game {
    out: RefCell<Vec<String>>,
}

impl GameBridge for Game {
    fn side(&self) -> Side {
        Side::Server
    }
    fn host_name(&self) -> String {
        "art-smoke 1.0".into()
    }
    fn log(&self, mod_id: Option<&str>, level: LogLevel, message: &str) {
        self.out.borrow_mut().push(format!("[{}] {}: {message}", level.name(), mod_id.unwrap_or("-")));
    }
    fn online_players(&self) -> Vec<String> {
        vec!["p1".into()]
    }
    fn player_alias(&self, id: &str) -> Option<String> {
        (id == "p1").then(|| "ALICE".to_string())
    }
    fn player_role(&self, _id: &str) -> Role {
        Role::Player
    }
    fn send_chat(&self, id: &str, text: &str) -> bool {
        self.out.borrow_mut().push(format!("chat to {id}: {text}"));
        true
    }
    fn broadcast(&self, text: &str) {
        self.out.borrow_mut().push(format!("broadcast: {text}"));
    }
    fn kick(&self, _id: &str, _reason: &str) -> bool {
        false
    }
    fn is_builtin_command(&self, _name: &str) -> bool {
        false
    }
}

fn load(rt: Box<dyn bhmod_core::Runtime>, dir: PathBuf, tune: impl FnOnce(&mut HostOptions)) -> (ModHost, Rc<Game>) {
    let game = Rc::new(Game::default());
    let mut opts = HostOptions::new(&dir, Side::Server);
    opts.runtimes = vec![rt];
    tune(&mut opts);
    (ModHost::load(opts, game.clone()), game)
}

fn states(host: &ModHost) -> String {
    host.mods().iter().map(|m| format!("{} {:?}", m.id, m.state)).collect::<Vec<_>>().join(", ")
}

fn smoke(env: &mut JNIEnv, class: &JClass, root: PathBuf) -> Result<String, String> {
    let e = |x: jni::errors::Error| x.to_string();
    let vm = env.get_java_vm().map_err(e)?;
    let loader = env.call_method(class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[]).and_then(|v| v.l()).map_err(e)?;
    let loader = env.new_global_ref(loader).map_err(e)?;
    let cfg = JvmConfig { grace: std::time::Duration::from_millis(20), ..JvmConfig::default() };
    // SAFETY: `vm` is the JavaVM of this process and `loader` a global reference to one of its class loaders.
    let rt = unsafe { attach_existing(vm.get_java_vm_pointer(), loader.clone(), &cfg) }?;
    let rt2 = || -> Result<Box<dyn bhmod_core::Runtime>, String> {
        // SAFETY: as above.
        Ok(Box::new(unsafe { attach_existing(vm.get_java_vm_pointer(), loader.clone(), &cfg) }?))
    };
    let p1 = || CommandSender::Player("p1".into());
    let mut lines = Vec::new();

    // hello-java: events, commands, storage.
    let (mut host, game) = load(Box::new(rt), root.join("hello"), |_| {});
    host.enable("World", "wid");
    lines.push(format!("hello mods: {}", states(&host)));
    host.fire(Event::PlayerJoin { player: "p1".into() });
    let ev = host.fire(Event::Chat { player: "p1".into(), text: "Lua is BAD".into(), cancelled: false });
    lines.push(format!("hello chat cancelled: {}", ev.cancelled()));
    lines.push(format!("hello /hello: {:?}", host.command(p1(), "/hello")));
    host.disable_all();
    lines.extend(game.out.borrow().iter().map(|l| format!("hello log: {l}")));

    // Mutations.
    let (mut host, _) = load(rt2()?, root.join("mutator"), |_| {});
    host.enable("World", "wid");
    let ev = host.fire(Event::Chat { player: "p1".into(), text: "quiet".into(), cancelled: false });
    lines.push(format!("mutator mods: {} / {:?}", states(&host), ev));

    // A service between two Java mods.
    let (mut host, _) = load(rt2()?, root.join("services"), |_| {});
    host.enable("World", "wid");
    lines.push(format!("services mods: {}", states(&host)));
    lines.push(format!("services add: {:?}", host.command(p1(), "/calc add 2 40")));
    lines.push(format!("services name: {:?}", host.command(p1(), "/calc name")));
    lines.push(format!("services fail: {:?}", host.command(p1(), "/calc fail")));

    // The watchdog interrupts a sleeping handler on ART as well.
    let (mut host, game) = load(rt2()?, root.join("faulty"), |o| o.budgets.hard = std::time::Duration::from_millis(50));
    host.enable("World", "wid");
    let start = std::time::Instant::now();
    host.fire(Event::Chat { player: "p1".into(), text: "sleep".into(), cancelled: false });
    lines.push(format!("faulty interrupted within 5 s: {}", start.elapsed() < std::time::Duration::from_secs(5)));
    lines.extend(game.out.borrow().iter().filter(|l| l.contains("budget") || l.contains("trap")).map(|l| format!("faulty log: {l}")));
    host.disable_all();
    Ok(lines.join("\n"))
}

#[no_mangle]
pub extern "system" fn Java_ArtSmoke_run(mut env: JNIEnv, class: JClass, mods: JString) -> jstring {
    let dir: String = env.get_string(&mods).map(Into::into).unwrap_or_default();
    let text = smoke(&mut env, &class, PathBuf::from(dir)).unwrap_or_else(|e| format!("FAILED: {e}"));
    env.new_string(text).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}
