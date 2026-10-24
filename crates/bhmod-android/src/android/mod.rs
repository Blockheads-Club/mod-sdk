//! Android glue: `JNI_OnLoad`, the Objective-C hooks into the game, the [`Platform`] that reaches the game and
//! the Java UI, and the native methods of `com.blockheadsclub.modloader.Native`. Only built for Android; the logic
//! it calls is in [`crate::loader`].
//!
//! Hooks (installed in `JNI_OnLoad` before the game runs). The loader lives on the thread that runs `drawFrame`;
//! network callbacks that come on another thread (a phone receives packets on a second thread while a world loads)
//! are queued in [`INBOX`] and handled at the next frame:
//! - `-[EvolutionViewController drawFrame]`: frame tick, world open/close detection, queued network callbacks.
//! - `-[BHClient match:didReceiveData:fromPlayer:]`: 0x23 starts a session (HELLO goes out first); 0xBC packets
//!   go to the loader and never reach the game.
//! - `-[BHNetClientMatch sendPlayerInformationToServer:password:]`: the 0x1f is held until the 0xBC flow decides,
//!   then sent with `bhcModLoader` added to a copy of `myPlayerInfo`.
//! - `-[BHNetClientMatch disconnect]`: the session ends.

pub mod cosmetics;
pub mod items;
pub mod menus;
pub mod native;
pub mod objc;
pub mod weapons;

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use bhmod_core::types::LogLevel;
use bhmod_proto::{hex, Hash, PLIST_KEY, PROTO};
use jni::objects::{GlobalRef, JClass, JString, JValue};
use jni::sys::{jboolean, jint, jstring, JNI_FALSE, JNI_TRUE, JNI_VERSION_1_6};
use jni::{JNIEnv, JavaVM};

use crate::inbox::{Inbox, NetEvent};
use crate::loader::{Dirs, Loader, Platform, UiEvent};
use objc::{Id, Imp, Sel};

const TAG: &str = "BHModLoader";
const BRIDGE: &str = "com/blockheadsclub/modloader/Bridge";

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Writes to logcat.
pub fn log(level: LogLevel, msg: &str) {
    let prio = match level {
        LogLevel::Trace => 2,
        LogLevel::Debug => 3,
        LogLevel::Info => 4,
        LogLevel::Warn => 5,
        LogLevel::Error => 6,
    };
    let tag = CString::new(TAG).expect("no NUL");
    let text = CString::new(msg.replace('\0', " ")).expect("no NUL");
    unsafe { __android_log_write(prio, tag.as_ptr(), text.as_ptr()) };
}

static VM: OnceLock<JavaVM> = OnceLock::new();
static BRIDGE_CLASS: OnceLock<GlobalRef> = OnceLock::new();
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
/// The loader thread and network callbacks from the game's other threads.
static INBOX: Inbox = Inbox::new();
/// The connection the game is joining, set by the receive hook on whatever thread the game receives on.
static CONN: Mutex<Option<Conn>> = Mutex::new(None);
/// Answers from the Java UI threads, drained every frame.
static EVENTS: Mutex<Vec<UiEvent>> = Mutex::new(Vec::new());

struct Originals {
    draw_frame: Imp,
    receive: Imp,
    send_info: Imp,
    disconnect: Imp,
}
// SAFETY: code addresses, written once in JNI_OnLoad.
unsafe impl Send for Originals {}
unsafe impl Sync for Originals {}
static ORIG: OnceLock<Originals> = OnceLock::new();

/// The connection the game is joining (raw pointers owned by the game; cleared on disconnect).
#[derive(Clone, Copy)]
struct Conn {
    client: Id,
    matc: Id,
    peer: Id,
}
// SAFETY: plain pointers to game objects; only the loader thread sends messages to them.
unsafe impl Send for Conn {}

fn conn() -> Option<Conn> {
    *CONN.lock().unwrap_or_else(|e| e.into_inner())
}

/// This client's own player id (`-[BHNetClientMatch localPlayerID]`) while a server connection is up: the game
/// leaves `clientID` empty on the blockheads of this client, the server's `COSMETICS` name it by this id.
pub fn local_player() -> Option<String> {
    let c = conn()?;
    let id = objc::string(objc::send0(c.matc, "localPlayerID"));
    (!id.is_empty()).then_some(id)
}

fn set_conn(c: Option<Conn>) {
    *CONN.lock().unwrap_or_else(|e| e.into_inner()) = c;
}

/// A held `sendPlayerInformationToServer:password:` call (arguments retained).
#[derive(Clone, Copy)]
struct HeldJoin {
    matc: Id,
    server_id: Id,
    password: Id,
}

/// Work that re-enters the game, run after the loader is released.
enum Deferred {
    Join(Option<Hash>),
    Abort,
}

thread_local! {
    static LOADER: RefCell<Option<Loader<AndroidPlatform>>> = const { RefCell::new(None) };
    static HELD: RefCell<Option<HeldJoin>> = const { RefCell::new(None) };
    static DEFERRED: RefCell<Vec<Deferred>> = const { RefCell::new(Vec::new()) };
    static WORLD: RefCell<usize> = const { RefCell::new(0) };
}

fn data_dir() -> PathBuf {
    DATA_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("/data/local/tmp/bhmods"))
}

/// Calls `Bridge.<name>(String)` on the Java side.
pub(crate) fn java_post(json: &str) {
    let (Some(vm), Some(cls)) = (VM.get(), BRIDGE_CLASS.get()) else { return };
    let Ok(mut env) = vm.attach_current_thread_permanently() else { return };
    let Ok(s) = env.new_string(json) else { return };
    let class: &JClass = cls.as_obj().into();
    if env.call_static_method(class, "post", "(Ljava/lang/String;)V", &[JValue::Object(&s)]).is_err() {
        let _ = env.exception_clear();
    }
    let _ = env.delete_local_ref(s);
}

/// The game, as the loader sees it.
pub struct AndroidPlatform;

impl Platform for AndroidPlatform {
    fn now_ms(&self) -> u64 {
        START.get_or_init(Instant::now).elapsed().as_millis() as u64
    }
    fn log(&self, level: LogLevel, msg: &str) {
        log(level, msg);
    }
    fn send(&self, packet: &[u8]) {
        let Some(c) = conn() else { return };
        let data = objc::nsdata(packet);
        unsafe {
            // -[BHNetClientMatch sendData:toPlayers:withDataMode:error:], mode 0 = reliable
            objc::msg::<unsafe extern "C" fn(Id, Sel, Id, Id, i32, *mut Id) -> i8>()(
                c.matc,
                objc::sel("sendData:toPlayers:withDataMode:error:"),
                data,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
            );
        }
    }
    fn release_join(&self, mod_loader: Option<Hash>) {
        DEFERRED.with(|d| d.borrow_mut().push(Deferred::Join(mod_loader)));
    }
    fn abort_join(&self) {
        DEFERRED.with(|d| d.borrow_mut().push(Deferred::Abort));
    }
    fn ui(&self, json: String) {
        java_post(&json);
    }
    fn load_native(&self, mod_id: &str, lib: &std::path::Path, data_dir: &std::path::Path) -> Result<(), String> {
        native::load(mod_id, lib, data_dir)
    }
    fn unload_native(&self, mod_id: &str) {
        native::unload(mod_id);
    }
}

/// Runs `f` on the loader (created on first use), then the deferred game calls. `None` off the loader thread.
fn with_loader<R>(f: impl FnOnce(&mut Loader<AndroidPlatform>) -> R) -> Option<R> {
    if !INBOX.is_owner() {
        return None;
    }
    let r = LOADER.with(|l| {
        let mut l = l.try_borrow_mut().ok()?;
        if l.is_none() {
            let dirs = Dirs::under(&data_dir());
            let wasm_cache = data_dir().join("wasm-cache");
            let loader = Loader::new(
                Rc::new(AndroidPlatform),
                dirs,
                crate::caps(),
                crate::options(),
                Box::new(move || crate::runtimes(&wasm_cache)),
            )
            .with_budgets(crate::budgets());
            // The blockhead hooks run on the game's other threads: they read the cosmetics through this handle.
            let _ = cosmetics::SHARED.set(loader.content());
            *l = Some(loader);
        }
        Some(f(l.as_mut().expect("just set")))
    });
    run_deferred();
    r
}

fn run_deferred() {
    let work: Vec<Deferred> = DEFERRED.with(|d| d.borrow_mut().drain(..).collect());
    for w in work {
        match w {
            Deferred::Join(key) => send_held_join(key),
            Deferred::Abort => abort(),
        }
    }
}

/// Sends the held 0x1f through the original method, with `bhcModLoader` in a temporary copy of `myPlayerInfo`.
fn send_held_join(key: Option<Hash>) {
    let Some(h) = HELD.with(|h| h.borrow_mut().take()) else { return };
    let orig = ORIG.get().expect("hooks").send_info;
    let slot = objc::ivar_ptr(h.matc, "BHNetClientMatch", "myPlayerInfo");
    let saved = if slot.is_null() { std::ptr::null_mut() } else { unsafe { *slot } };
    if let (Some(hash), false) = (key, slot.is_null()) {
        let info = objc::send1(objc::class("NSMutableDictionary"), "dictionaryWithDictionary:", saved);
        let ml = objc::send0(objc::class("NSMutableDictionary"), "dictionary");
        objc::send2(ml, "setObject:forKey:", objc::nsstring(&hex(&hash)), objc::nsstring("acceptedHash"));
        objc::send2(ml, "setObject:forKey:", objc::nsnumber(PROTO as i32), objc::nsstring("proto"));
        objc::send2(info, "setObject:forKey:", ml, objc::nsstring(PLIST_KEY));
        unsafe { *slot = info };
    }
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, Id, Id) = std::mem::transmute(orig);
        f(h.matc, objc::sel("sendPlayerInformationToServer:password:"), h.server_id, h.password);
        if !slot.is_null() {
            *slot = saved;
        }
    }
    for o in [h.matc, h.server_id, h.password] {
        if !o.is_null() {
            objc::send0(o, "release");
        }
    }
}

/// The game's `GameView`, seen by the drawFrame hook (the game's own "cancel" for a pending join lives there).
static GAME_VIEW: Mutex<usize> = Mutex::new(0);

/// Leaves the game's join screen the way its own Cancel button does (`-[GameView
/// cancelSelectedWhileAttemptingToConnect]`: exit the pending world, back to the menu selection), so the player sees
/// only the loader's message and not a vanilla "rejected" alert. Falls back to feeding the game a vanilla 0x26
/// (reason 3) when the view is not known.
fn abort() {
    HELD.with(|h| h.borrow_mut().take());
    let gv = *GAME_VIEW.lock().unwrap_or_else(|e| e.into_inner());
    if gv != 0 {
        objc::send0(gv as Id, "cancelSelectedWhileAttemptingToConnect");
        return;
    }
    let Some(c) = conn() else { return };
    let orig = ORIG.get().expect("hooks").receive;
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, Id, Id, Id) = std::mem::transmute(orig);
        f(c.client, objc::sel("match:didReceiveData:fromPlayer:"), c.matc, objc::nsdata(&[0x26, 3]), c.peer);
    }
}

pub(crate) fn guard(what: &str, f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        log(LogLevel::Error, &format!("panic in {what}; the loader keeps going"));
    }
}

extern "C" fn hook_draw_frame(this: Id, cmd: Sel) {
    guard("drawFrame", || {
        INBOX.claim();
        for ev in INBOX.drain() {
            if ev == NetEvent::Disconnect {
                release_held();
            }
            with_loader(|l| l.on_net(ev));
        }
        let events: Vec<UiEvent> = EVENTS.lock().map(|mut e| e.drain(..).collect()).unwrap_or_default();
        let gv = objc::ivar(this, "EvolutionViewController", "gameView");
        *GAME_VIEW.lock().unwrap_or_else(|e| e.into_inner()) = gv as usize;
        let world = objc::ivar(gv, "GameView", "world");
        let changed = WORLD.with(|w| std::mem::replace(&mut *w.borrow_mut(), world as usize) != world as usize);
        with_loader(|l| {
            for e in events {
                l.on_ui(e);
            }
            if changed {
                cosmetics::reset();
                if world.is_null() {
                    l.on_world(None);
                } else {
                    let name = objc::string(objc::ivar(world, "World", "worldName"));
                    l.on_world(Some(&name));
                }
            }
            l.on_frame();
            items::sync();
        });
        // After the loader ran: mods may have just loaded or unloaded their weapon cosmetics.
        weapons::sync();
    });
    let orig = ORIG.get().expect("hooks").draw_frame;
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel) = std::mem::transmute(orig);
        f(this, cmd);
    }
}

extern "C" fn hook_receive(this: Id, cmd: Sel, matc: Id, data: Id, peer: Id) {
    let mut consumed = false;
    guard("didReceiveData", || {
        let bytes = objc::data_bytes(data);
        set_conn(Some(Conn { client: this, matc, peer }));
        let owner = INBOX.is_owner();
        match bytes.first() {
            Some(0x23) => {
                let host = objc::string(objc::ivar(matc, "BHMatch", "host"));
                let port = objc::string(objc::ivar(matc, "BHMatch", "port"));
                let server = format!("{host}:{port}");
                if owner {
                    with_loader(|l| l.on_contact(&server));
                } else {
                    INBOX.push(NetEvent::Contact(server));
                }
            }
            Some(0xBC) if owner => consumed = with_loader(|l| l.on_packet(&bytes)).unwrap_or(true),
            Some(0xBC) => {
                INBOX.push(NetEvent::Packet(bytes));
                consumed = true;
            }
            _ => {}
        }
    });
    if consumed {
        return;
    }
    let orig = ORIG.get().expect("hooks").receive;
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, Id, Id, Id) = std::mem::transmute(orig);
        f(this, cmd, matc, data, peer);
    }
}

extern "C" fn hook_send_info(this: Id, cmd: Sel, server_id: Id, password: Id) {
    let mut held = false;
    guard("sendPlayerInformationToServer", || {
        if !INBOX.is_owner() {
            log(LogLevel::Warn, "the game joins from another thread; joining without mods");
            return;
        }
        for o in [this, server_id, password] {
            if !o.is_null() {
                objc::send0(o, "retain");
            }
        }
        HELD.with(|h| *h.borrow_mut() = Some(HeldJoin { matc: this, server_id, password }));
        held = with_loader(|l| l.on_join_request()).is_some();
    });
    if held {
        return;
    }
    // Another thread, or the loader was busy or panicked: behave like vanilla.
    release_held();
    let orig = ORIG.get().expect("hooks").send_info;
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, Id, Id) = std::mem::transmute(orig);
        f(this, cmd, server_id, password);
    }
}

/// Drops a held 0x1f (the connection closed before the 0xBC flow decided).
fn release_held() {
    if let Some(h) = HELD.with(|h| h.borrow_mut().take()) {
        for o in [h.matc, h.server_id, h.password] {
            if !o.is_null() {
                objc::send0(o, "release");
            }
        }
    }
}

extern "C" fn hook_disconnect(this: Id, cmd: Sel) {
    guard("disconnect", || {
        set_conn(None);
        if INBOX.is_owner() {
            release_held();
            with_loader(|l| l.on_disconnect());
        } else {
            INBOX.push(NetEvent::Disconnect);
        }
    });
    let orig = ORIG.get().expect("hooks").disconnect;
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel) = std::mem::transmute(orig);
        f(this, cmd);
    }
}

fn install_hooks() -> bool {
    if !objc::available() {
        log(LogLevel::Error, "the Objective-C runtime or libApplication.so is not loaded; mods are off");
        return false;
    }
    let draw_frame = objc::swizzle("EvolutionViewController", "drawFrame", hook_draw_frame as Imp);
    let receive = objc::swizzle("BHClient", "match:didReceiveData:fromPlayer:", hook_receive as Imp);
    let send_info = objc::swizzle("BHNetClientMatch", "sendPlayerInformationToServer:password:", hook_send_info as Imp);
    let disconnect = objc::swizzle("BHNetClientMatch", "disconnect", hook_disconnect as Imp);
    let ok = [draw_frame, receive, send_info, disconnect].iter().all(|p| !p.is_null());
    menus::install();
    let _ = ORIG.set(Originals { draw_frame, receive, send_info, disconnect });
    if !ok {
        log(LogLevel::Error, "a game method was not found; this game version is not supported, mods may not work");
    }
    ok
}

fn java_data_dir(env: &mut JNIEnv, cls: &JClass) -> Option<PathBuf> {
    let v = env.call_static_method(cls, "dataDir", "()Ljava/lang/String;", &[]).ok()?.l().ok()?;
    let s: String = env.get_string(&JString::from(v)).ok()?.into();
    Some(PathBuf::from(s))
}

/// The class loader of the loader's own dex (it holds the Java UI and, in a `--jvm` build, the SDK classes).
#[cfg(feature = "jvm")]
static DEX_LOADER: OnceLock<GlobalRef> = OnceLock::new();

#[cfg(feature = "jvm")]
fn keep_loader(env: &mut JNIEnv, bridge: &JClass) {
    let loader = env.call_method(bridge, "getClassLoader", "()Ljava/lang/ClassLoader;", &[]).and_then(|v| v.l());
    match loader.and_then(|l| env.new_global_ref(l)) {
        Ok(g) => {
            let _ = DEX_LOADER.set(g);
        }
        Err(_) => {
            let _ = env.exception_clear();
        }
    }
}

/// The JVM runtime on ART: the VM already runs in this process, the SDK classes come from the loader's dex and every
/// mod brings its own `classes.dex` (see bhmod-core `jvm::attach_existing`).
#[cfg(feature = "jvm")]
fn jvm_factory() -> Result<Box<dyn bhmod_core::runtime::Runtime>, String> {
    let fail = |m: &str| {
        log(LogLevel::Error, &format!("JVM mods are on but unavailable: {m}"));
        m.to_string()
    };
    let vm = VM.get().ok_or_else(|| fail("no JavaVM"))?;
    let loader = DEX_LOADER.get().ok_or_else(|| fail("no class loader for the SDK"))?.clone();
    // SAFETY: the pointer belongs to the process' JavaVM (valid for the life of the process) and `loader` is a global
    // reference to a class loader of that VM.
    let rt = unsafe { bhmod_core::jvm::attach_existing(vm.get_java_vm_pointer(), loader, &Default::default()) }
        .map_err(|e| fail(&e))?;
    Ok(Box::new(rt))
}

/// Entry point: the game loads the library on its main thread, after `libApplication.so`.
#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut c_void) -> jint {
    START.get_or_init(Instant::now);
    let r = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(mut env) = vm.get_env() {
            match env.find_class(BRIDGE) {
                Ok(cls) => {
                    if let Some(d) = java_data_dir(&mut env, &cls) {
                        let _ = DATA_DIR.set(d);
                    }
                    #[cfg(feature = "jvm")]
                    keep_loader(&mut env, &cls);
                    if let Ok(g) = env.new_global_ref(cls) {
                        let _ = BRIDGE_CLASS.set(g);
                    }
                }
                Err(_) => {
                    let _ = env.exception_clear();
                    log(LogLevel::Warn, "Java UI classes not found; the loader runs without its UI");
                }
            }
        }
        let _ = VM.set(vm);
        #[cfg(feature = "jvm")]
        if data_dir().join("jvm-enabled").exists() {
            crate::set_jvm(Some(jvm_factory));
            log(LogLevel::Warn, "JVM mods are on (jvm-enabled): they run unsandboxed");
        }
        let hooks = install_hooks();
        let cosmetics = hooks && cosmetics::install();
        if hooks && !cosmetics {
            log(LogLevel::Warn, "a Blockhead method was not found; cosmetics are off");
        }
        log(
            LogLevel::Info,
            &format!(
                "mod loader {} loaded (hooks {}, data {}, runtimes {:?})",
                crate::VERSION,
                if hooks { "on" } else { "off" },
                data_dir().display(),
                crate::options().runtimes
            ),
        );
        java_post(&serde_json::json!({"t": "ready", "version": crate::VERSION, "hooks": hooks}).to_string());
    }));
    if r.is_err() {
        log(LogLevel::Error, "panic while starting the mod loader");
    }
    JNI_VERSION_1_6
}

fn jstr(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    env.get_string(s).ok().map(Into::into)
}

fn out(env: &mut JNIEnv, s: String) -> jstring {
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn installed_dir() -> PathBuf {
    Dirs::under(&data_dir()).installed
}

/// `Native.decide(boolean)`: the join prompt was answered.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_decide(_env: JNIEnv, _c: JClass, accept: jboolean) {
    if let Ok(mut e) = EVENTS.lock() {
        e.push(UiEvent::Decide(accept != JNI_FALSE));
    }
}

/// `Native.dialogAnswer(dialog, button, text)`: a mod dialog was answered (`text` null without a field).
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_dialogAnswer(
    mut env: JNIEnv,
    _c: JClass,
    dialog: jint,
    button: jint,
    text: JString,
) {
    let text = jstr(&mut env, &text);
    if let Ok(mut e) = EVENTS.lock() {
        e.push(UiEvent::Dialog { dialog: dialog as u32, button: button as u32, text });
    }
}

/// `Native.cancel()`: the player cancelled the modded join.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_cancel(_env: JNIEnv, _c: JClass) {
    if let Ok(mut e) = EVENTS.lock() {
        e.push(UiEvent::Cancel);
    }
}

/// `Native.font(id)`: the bytes of an embedded font ([`crate::fonts`]), empty for an unknown id.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_font(
    env: JNIEnv,
    _c: JClass,
    id: jint,
) -> jni::sys::jbyteArray {
    let bytes = crate::fonts::get(id).unwrap_or(&[]);
    env.byte_array_from_slice(bytes).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut())
}

/// `Native.setModsLabel(text)`: the label of the Mods buttons in the game's menus.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_setModsLabel(
    mut env: JNIEnv,
    _c: JClass,
    text: JString,
) {
    if let Some(s) = jstr(&mut env, &text) {
        menus::set_label(&s);
    }
}

/// `Native.version()`.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_version(mut env: JNIEnv, _c: JClass) -> jstring {
    out(&mut env, crate::VERSION.to_string())
}

/// `Native.listInstalled()`: JSON.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_listInstalled(mut env: JNIEnv, _c: JClass) -> jstring {
    let s = crate::api::list(&installed_dir(), &crate::caps());
    out(&mut env, s)
}

/// `Native.inspect(path, sha256)`: JSON.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_inspect(
    mut env: JNIEnv,
    _c: JClass,
    path: JString,
    sha: JString,
) -> jstring {
    let p = jstr(&mut env, &path).unwrap_or_default();
    let sha = jstr(&mut env, &sha);
    let s = crate::api::inspect(&PathBuf::from(p), sha.as_deref(), &crate::caps());
    out(&mut env, s)
}

/// `Native.install(path, sha256, source, nativeConfirmed)`: JSON.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_install(
    mut env: JNIEnv,
    _c: JClass,
    path: JString,
    sha: JString,
    source: JString,
    native_confirmed: jboolean,
) -> jstring {
    let p = jstr(&mut env, &path).unwrap_or_default();
    let sha = jstr(&mut env, &sha);
    let source = jstr(&mut env, &source);
    let s = crate::api::install(
        &installed_dir(),
        &PathBuf::from(p),
        sha.as_deref(),
        source.as_deref(),
        native_confirmed != JNI_FALSE,
        &crate::caps(),
    );
    out(&mut env, s)
}

/// `Native.setEnabled(id, on)`.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_setEnabled(
    mut env: JNIEnv,
    _c: JClass,
    id: JString,
    on: jboolean,
) -> jboolean {
    let id = jstr(&mut env, &id).unwrap_or_default();
    if crate::api::set_enabled(&installed_dir(), &id, on != JNI_FALSE, &crate::caps()) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// `Native.remove(id)`.
#[no_mangle]
pub extern "system" fn Java_com_blockheadsclub_modloader_Native_remove(
    mut env: JNIEnv,
    _c: JClass,
    id: JString,
) -> jboolean {
    let id = jstr(&mut env, &id).unwrap_or_default();
    if crate::api::remove(&installed_dir(), &id, &crate::caps()) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}
