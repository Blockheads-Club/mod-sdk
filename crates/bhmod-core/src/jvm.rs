//! The JVM runtime (JNI): mods written in Java, Kotlin or any JVM language, packaged as a jar (`runtime = "jvm"`).
//! Implements the mapping in `docs/jvm.md`.
//!
//! - One JVM per process, created on first use from the user's JRE (`libjvm` is loaded at run time, nothing links
//!   against it). Every mod gets its own class loader; all of them share the JVM.
//! - The Java side is `sdk/java` (compiled by `build.rs` into an embedded jar). The WIT bindings in it are generated
//!   by `scripts/gen-java.py`. This module only moves canonical CBOR across two functions: `Glue.invoke` calls a
//!   `guest` export of the mod, `Glue.host` (native, implemented here) is a WIT import.
//! - The host API of the call in progress is reached through a thread-local pointer, set when the host enters the
//!   mod and cleared before the call returns, like the Luau runtime does.
//! - A JVM cannot be sandboxed: mods have the permissions of the host process, which is why the manifest requires
//!   `permissions.native`. Time is the one budget it can enforce, and only by interrupting: a watchdog interrupts the
//!   calling thread at the hard deadline plus [`JvmConfig::grace`] (sleeps, waits and blocking I/O end; a loop that
//!   never blocks cannot be stopped). Heap use is capped for the whole JVM by [`JvmConfig::max_heap_mb`].
//! - On Android the JVM is ART, already running in the process: [`attach_existing`] adopts a `JavaVM` pointer
//!   and a class loader that sees the SDK classes (a dex of `sdk/java`), and mods ship `classes.dex` in their jar,
//!   which `InMemoryDexClassLoader` loads (see `docs/jvm.md`).
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::ffi::c_void;
#[cfg(not(target_os = "android"))]
use std::ffi::OsString;
use std::panic::{catch_unwind, AssertUnwindSafe};
#[cfg(not(target_os = "android"))]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JString, JValue};
use jni::sys::{jbyteArray, jint};
#[cfg(not(target_os = "android"))]
use jni::{InitArgsBuilder, JNIVersion};
use jni::{JNIEnv, JavaVM, NativeMethod};

use crate::channels::ChannelId;
use crate::remote::RemoteTarget;
use crate::runtime::TaskId;
use crate::runtime::{CallError, CommandId, HostApi, Instance, InstanceLimits, ModFiles, Runtime, SubscriptionId};
use crate::types::{
    ApiError, ChannelSource, ChannelTarget, CommandOutcome, CommandSender, CommandSpec, ContentKind, Event, EventKind,
    LogLevel, Mutation, Outcome, Pos, Priority, RejectReason, Role, ServiceError,
};
use crate::ui::{Button, ButtonKind, Dialog, DialogButton, Field, NoticeLevel, OverheadIcon, Slot, Widget, WidgetKind};
use crate::value::Value;
use crate::voice::{Activation, Caps, Icons, Mode, Route, Transport, TransportConfig, VoiceCall, VoiceReply};

/// The Java SDK (generated bindings and runtime glue), compiled by `build.rs`.
#[cfg(not(target_os = "android"))]
pub const SDK_JAR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bhmod-sdk.jar"));

const GLUE: &str = "club/blockheads/mod/rt/Glue";

/// Options of the JVM runtime. They apply when the process' JVM is created (the first [`JvmRuntime::new`]); later
/// runtimes share that JVM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JvmConfig {
    /// The JRE or JDK to use; `None` searches `BHMOD_JAVA_HOME`, `JAVA_HOME`, [`JvmConfig::bundled`] and `java` on
    /// the `PATH`.
    pub java_home: Option<PathBuf>,
    /// A runtime that ships with the application (for example `<app dir>/jre`), tried after the environment.
    pub bundled: Option<PathBuf>,
    /// Heap limit of the whole JVM in MiB (`-Xmx`).
    pub max_heap_mb: u32,
    /// Extra JVM options, for example `-Xss512k`.
    pub extra_args: Vec<String>,
    /// Where the embedded SDK jar is written for the class path; `None` uses the system temp directory.
    pub work_dir: Option<PathBuf>,
    /// How long past the hard deadline a call may run before the watchdog interrupts it (cold JVM code is slow).
    pub grace: Duration,
    /// Extra grace for `on-enable`, which loads the mod's classes and runs its static initialisers.
    pub enable_grace: Duration,
}

impl Default for JvmConfig {
    fn default() -> Self {
        JvmConfig {
            java_home: None,
            bundled: None,
            max_heap_mb: 256,
            extra_args: Vec::new(),
            work_dir: None,
            grace: Duration::from_millis(250),
            enable_grace: Duration::from_secs(5),
        }
    }
}

// -- locating the JRE ---------------------------------------------------------------------------------------------

/// The file name of the JVM library on this platform.
#[cfg(not(target_os = "android"))]
const LIBJVM: &str = if cfg!(windows) {
    "jvm.dll"
} else if cfg!(target_os = "macos") {
    "libjvm.dylib"
} else {
    "libjvm.so"
};

/// The `libjvm` file inside a JRE or JDK directory.
#[cfg(not(target_os = "android"))]
pub fn libjvm_in(home: &Path) -> Option<PathBuf> {
    let name = LIBJVM;
    let mut dirs = vec![
        home.join("lib/server"),
        home.join("bin/server"),
        home.join("jre/bin/server"),
        home.join("jre/lib/server"),
    ];
    for arch in ["amd64", "aarch64", "arm", "i386", "x86_64"] {
        dirs.push(home.join("lib").join(arch).join("server"));
        dirs.push(home.join("jre/lib").join(arch).join("server"));
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// The Java home behind a `java` executable on the `PATH` (symlinks resolved).
#[cfg(not(target_os = "android"))]
fn home_of_path(paths: Option<OsString>) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    std::env::split_paths(&paths?)
        .filter_map(|dir| dir.join(exe).canonicalize().ok())
        .filter_map(|real| real.parent()?.parent().map(Path::to_path_buf)) // <home>/bin/java
        .find(|home| libjvm_in(home).is_some())
}

/// Finds `libjvm`: the configured home, `BHMOD_JAVA_HOME`, `JAVA_HOME`, the bundled runtime, then `java` on the
/// `PATH`. The error lists what was tried.
#[cfg(not(target_os = "android"))]
pub fn locate(cfg: &JvmConfig) -> Result<PathBuf, String> {
    locate_in(cfg, &|key| std::env::var_os(key))
}

#[cfg(not(target_os = "android"))]
fn locate_in(cfg: &JvmConfig, var: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    let from_env = ["BHMOD_JAVA_HOME", "JAVA_HOME"].into_iter().filter_map(var).filter(|v| !v.is_empty());
    let homes: Vec<PathBuf> = (cfg.java_home.iter().cloned())
        .chain(from_env.map(PathBuf::from))
        .chain(cfg.bundled.clone())
        .chain(home_of_path(var("PATH")))
        .collect();
    let tried = homes.iter().map(|h| h.display().to_string()).collect::<Vec<_>>().join(", ");
    homes.iter().find_map(|h| libjvm_in(h)).ok_or_else(|| {
        format!(
            "no Java runtime found (tried {}); install a JRE 8 or newer and set JAVA_HOME",
            if tried.is_empty() { "nowhere" } else { &tried }
        )
    })
}

// -- the process' JVM ---------------------------------------------------------------------------------------------

/// The JVM of this process with the glue class.
struct Vm {
    jvm: JavaVM,
    glue: GlobalRef,
    watchdog: Watchdog,
}

static VM: OnceLock<Result<Arc<Vm>, String>> = OnceLock::new();

fn jerr(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Writes the embedded SDK jar where the JVM can read it and returns its path.
#[cfg(not(target_os = "android"))]
fn write_sdk(dir: &Path) -> Result<PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    SDK_JAR.hash(&mut h);
    let path = dir.join(format!("bhmod-sdk-{:016x}.jar", h.finish()));
    if std::fs::read(&path).is_ok_and(|b| b == SDK_JAR) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, SDK_JAR).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", path.display())
    })?;
    Ok(path)
}

/// Starts the JVM from `libjvm` with the SDK on the class path.
#[cfg(not(target_os = "android"))]
fn start(cfg: &JvmConfig) -> Result<Vm, String> {
    let lib = locate(cfg)?;
    let dir = cfg.work_dir.clone().unwrap_or_else(|| std::env::temp_dir().join("bhmod-jvm"));
    let jar = write_sdk(&dir)?;
    let mut b = InitArgsBuilder::new()
        .version(JNIVersion::V8)
        .option("-Xrs")
        .option("-Djava.awt.headless=true")
        .option(format!("-Xmx{}m", cfg.max_heap_mb))
        .option(format!("-Djava.class.path={}", jar.display()));
    for a in &cfg.extra_args {
        b = b.option(a.as_str());
    }
    let args = b.build().map_err(jerr)?;
    #[cfg(windows)]
    prepend_dll_dirs(&lib);
    let jvm = JavaVM::with_libjvm(args, || Ok::<_, jni::errors::StartJvmError>(lib.clone()))
        .map_err(|e| format!("cannot start the JVM from {}: {e}", lib.display()))?;
    restore_numeric_locale();
    adopt(jvm, None)
}

/// `jvm.dll` loads its sibling DLLs (`java.dll`, the C runtime) from the JRE's `bin`, which Windows only finds through
/// the `PATH` of the process: put `<home>\\bin` and `<home>\\bin\\server` first.
#[cfg(windows)]
fn prepend_dll_dirs(lib: &Path) {
    let Some(server) = lib.parent() else { return };
    let mut dirs = vec![server.to_path_buf()];
    dirs.extend(server.parent().map(Path::to_path_buf));
    dirs.extend(std::env::var_os("PATH").iter().flat_map(std::env::split_paths));
    if let Ok(joined) = std::env::join_paths(dirs) {
        std::env::set_var("PATH", joined);
    }
}

/// The JVM calls `setlocale(LC_ALL, "")` while it starts, which would make the C library of the whole process parse
/// and print numbers the user's way (`0,2` in Turkish or German): Luau's `tonumber("0.2")` would fail. Put the numeric
/// category back to `"C"`, as the process had it.
#[cfg(not(target_os = "android"))]
fn restore_numeric_locale() {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const LC_NUMERIC: std::ffi::c_int = 1;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const LC_NUMERIC: std::ffi::c_int = 4;
    extern "C" {
        fn setlocale(category: std::ffi::c_int, locale: *const std::ffi::c_char) -> *mut std::ffi::c_char;
    }
    // SAFETY: plain libc call with a static NUL-terminated string; no other thread of ours calls setlocale.
    unsafe {
        setlocale(LC_NUMERIC, c"C".as_ptr());
    }
}

/// Wraps a running JVM: finds the glue class (through `loader` when the application class path does not hold it),
/// registers the native `Glue.host` and starts the watchdog.
fn adopt(jvm: JavaVM, loader: Option<GlobalRef>) -> Result<Vm, String> {
    let mut env = jvm.attach_current_thread_permanently().map_err(jerr)?;
    let class = match loader {
        None => env.find_class(GLUE).map_err(jerr)?,
        Some(l) => {
            let name = env.new_string(GLUE.replace('/', ".")).map_err(jerr)?;
            let c = env
                .call_method(l.as_obj(), "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])
                .and_then(|v| v.l())
                .map_err(|e| format!("the SDK is not visible to the class loader: {e}"))?;
            JClass::from(c)
        }
    };
    let methods = [NativeMethod {
        name: "host".into(),
        sig: "(ILjava/lang/String;[B)[B".into(),
        fn_ptr: host_native as *mut c_void,
    }];
    env.register_native_methods(&class, &methods).map_err(jerr)?;
    let glue = env.new_global_ref(&class).map_err(jerr)?;
    Ok(Vm { jvm, glue, watchdog: Watchdog::default() })
}

fn vm_with(cfg: &JvmConfig, make: impl FnOnce(&JvmConfig) -> Result<Vm, String>) -> Result<Arc<Vm>, String> {
    let vm = VM.get_or_init(|| make(cfg).map(Arc::new));
    match vm {
        Ok(vm) => {
            vm.watchdog.ensure_running(vm);
            Ok(vm.clone())
        }
        Err(e) => Err(e.clone()),
    }
}

/// Adopts the JVM that already runs in this process (ART on Android) instead of creating one. `jvm` is the
/// `JavaVM*`, `loader` a class loader that can load the SDK classes (the loader that holds `sdk/java` as dex).
/// Fails when a JVM was already adopted or created.
///
/// # Safety
/// `jvm` must be a valid `JavaVM*` that stays valid for the life of the process, and `loader` a global reference to a
/// `java.lang.ClassLoader` of that VM.
pub unsafe fn attach_existing(
    jvm: *mut jni::sys::JavaVM,
    loader: GlobalRef,
    cfg: &JvmConfig,
) -> Result<JvmRuntime, String> {
    let make = |_: &JvmConfig| {
        // SAFETY: the caller guarantees the pointer (see above).
        let jvm = unsafe { JavaVM::from_raw(jvm) }.map_err(jerr)?;
        adopt(jvm, Some(loader))
    };
    Ok(JvmRuntime { vm: vm_with(cfg, make)?, cfg: cfg.clone() })
}

// -- the watchdog -------------------------------------------------------------------------------------------------

struct Arm {
    token: u64,
    inst: i32,
    at: Instant,
    fired: bool,
}

#[derive(Default)]
struct WdState {
    arms: Vec<Arm>,
    next: u64,
    started: bool,
}

/// Interrupts the thread of a call that runs past its deadline.
#[derive(Default)]
struct Watchdog {
    state: Mutex<WdState>,
    cv: Condvar,
}

impl Watchdog {
    fn ensure_running(&self, vm: &Arc<Vm>) {
        let mut s = self.state.lock().expect("watchdog lock");
        if s.started {
            return;
        }
        s.started = true;
        let vm = vm.clone();
        std::thread::Builder::new()
            .name("bhmod-jvm-watchdog".into())
            .spawn(move || watchdog_loop(vm))
            .expect("spawn the JVM watchdog");
    }

    fn arm(&self, inst: i32, at: Instant) -> u64 {
        let mut s = self.state.lock().expect("watchdog lock");
        s.next += 1;
        let token = s.next;
        s.arms.push(Arm { token, inst, at, fired: false });
        self.cv.notify_all();
        token
    }

    /// Removes an arm; true when the watchdog fired for it.
    fn disarm(&self, token: u64) -> bool {
        let mut s = self.state.lock().expect("watchdog lock");
        let i = s.arms.iter().position(|a| a.token == token).expect("an armed token");
        s.arms.remove(i).fired
    }
}

/// Runs for the life of the process (the JVM does too): waits for the earliest arm, interrupts its call.
fn watchdog_loop(vm: Arc<Vm>) -> ! {
    // A daemon: a JVM that wants to exit (app_process after main returns) must not wait for this thread.
    let mut env = vm.jvm.attach_current_thread_as_daemon().expect("attach the JVM watchdog thread");
    loop {
        let inst = {
            let mut s = vm.watchdog.state.lock().expect("watchdog lock");
            loop {
                let now = Instant::now();
                let due = s.arms.iter_mut().filter(|a| !a.fired).min_by_key(|a| a.at);
                match due {
                    None => s = vm.watchdog.cv.wait(s).expect("watchdog wait"),
                    Some(a) if a.at <= now => {
                        a.fired = true;
                        break a.inst;
                    }
                    Some(a) => {
                        let wait = a.at - now;
                        s = vm.watchdog.cv.wait_timeout(s, wait).expect("watchdog wait").0;
                    }
                }
            }
        };
        let _ = env.with_local_frame(8, |env| {
            env.call_static_method(glue_class(&vm), "interrupt", "(I)V", &[JValue::Int(inst)]).map(|_| ())
        });
    }
}

fn glue_class(vm: &Vm) -> &JClass<'static> {
    // SAFETY: `JClass` is a transparent wrapper of a `jclass`; the global reference keeps the class alive.
    unsafe { &*(vm.glue.as_obj() as *const JObject<'static> as *const JClass<'static>) }
}

// -- host calls (Java -> Rust) ------------------------------------------------------------------------------------

type HostPtr = *mut (dyn HostApi + 'static);

thread_local! {
    /// The host API of each mod instance that runs a call on this thread.
    static ACTIVE: RefCell<Vec<(i32, HostPtr)>> = const { RefCell::new(Vec::new()) };
}

/// Removes the entry again when the call ends, even if it unwinds.
struct Entered;

impl Drop for Entered {
    fn drop(&mut self) {
        ACTIVE.with(|a| {
            a.borrow_mut().pop();
        });
    }
}

fn enter(inst: i32, host: &mut dyn HostApi) -> Entered {
    let ptr: *mut (dyn HostApi + '_) = host;
    // SAFETY: only the lifetime is erased; the entry is removed by `Entered` before the borrow ends.
    let ptr: HostPtr = unsafe { std::mem::transmute::<*mut (dyn HostApi + '_), HostPtr>(ptr) };
    ACTIVE.with(|a| a.borrow_mut().push((inst, ptr)));
    Entered
}

extern "system" fn host_native<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    inst: jint,
    func: JString<'l>,
    args: JByteArray<'l>,
) -> jbyteArray {
    let r = catch_unwind(AssertUnwindSafe(|| -> Result<Vec<u8>, String> {
        let name: String = env.get_string(&func).map_err(jerr)?.into();
        let bytes = env.convert_byte_array(&args).map_err(jerr)?;
        host_call(inst, &name, &bytes)
    }))
    .unwrap_or_else(|p| Err(format!("the host panicked: {}", panic_text(&*p))));
    match r.and_then(|out| env.byte_array_from_slice(&out).map_err(jerr)) {
        Ok(a) => a.into_raw(),
        Err(msg) => {
            let _ = env.throw_new("java/lang/IllegalStateException", msg);
            std::ptr::null_mut()
        }
    }
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default()
}

/// Runs one WIT import for the mod instance `inst`.
fn host_call(inst: i32, name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let ptr = ACTIVE
        .with(|a| a.borrow().iter().rev().find(|e| e.0 == inst).map(|e| e.1))
        .ok_or("the host API can only be used while the host runs the mod (on the game thread)")?;
    let args = match Value::decode(bytes).map_err(jerr)? {
        Value::List(items) => items,
        _ => return Err("host call arguments must be a list".into()),
    };
    // SAFETY: `ptr` was created in `enter` from a `&mut dyn HostApi` that outlives the guest call, which is still
    // running (this function is called from it). The JVM calls back on this thread only, and the host does not use
    // its reference while the guest runs, so this is the only live reference derived from it.
    let host = unsafe { &mut *ptr };
    Ok(call_host(host, name, &args)?.encode())
}

fn arg(a: &[Value], i: usize) -> Result<&Value, String> {
    a.get(i).ok_or_else(|| format!("missing argument {i}"))
}

fn text(a: &[Value], i: usize) -> Result<&str, String> {
    match arg(a, i)? {
        Value::Text(t) => Ok(t),
        other => Err(format!("argument {i} must be text, found {other:?}")),
    }
}

fn int(a: &[Value], i: usize) -> Result<i64, String> {
    match arg(a, i)? {
        Value::Int(n) => Ok(*n),
        other => Err(format!("argument {i} must be an integer, found {other:?}")),
    }
}

fn unsigned(a: &[Value], i: usize) -> Result<u32, String> {
    u32::try_from(int(a, i)?).map_err(|_| format!("argument {i} is out of range"))
}

fn signed(a: &[Value], i: usize) -> Result<i32, String> {
    i32::try_from(int(a, i)?).map_err(|_| format!("argument {i} is out of range"))
}

fn flag(a: &[Value], i: usize) -> Result<bool, String> {
    match arg(a, i)? {
        Value::Bool(b) => Ok(*b),
        other => Err(format!("argument {i} must be a boolean, found {other:?}")),
    }
}

fn bytes_arg(a: &[Value], i: usize) -> Result<&[u8], String> {
    match arg(a, i)? {
        Value::Bytes(b) => Ok(b),
        other => Err(format!("argument {i} must be bytes, found {other:?}")),
    }
}

fn entry(tag: &str, v: Value) -> Value {
    Value::Map(vec![(tag.to_string(), v)])
}

fn api_error(e: &ApiError) -> Value {
    match e {
        ApiError::NotPermitted(p) => entry("not-permitted", Value::text(p)),
        ApiError::InvalidArgument(p) => entry("invalid-argument", Value::text(p)),
        ApiError::LimitExceeded(p) => entry("limit-exceeded", Value::text(p)),
        ApiError::StaleHandle => entry("stale-handle", Value::Null),
        ApiError::Unsupported => entry("unsupported", Value::Null),
        ApiError::Internal(p) => entry("internal", Value::text(p)),
    }
}

fn call_error(e: &ServiceError) -> Value {
    match e {
        ServiceError::NotFound => entry("not-found", Value::Null),
        ServiceError::VersionMismatch => entry("version-mismatch", Value::Null),
        ServiceError::NotPermitted => entry("not-permitted", Value::Null),
        ServiceError::Reentrant => entry("reentrant", Value::Null),
        ServiceError::ProviderError(m) => entry("provider-error", Value::text(m)),
        ServiceError::Trap => entry("trap", Value::Null),
        ServiceError::BadValue(m) => entry("bad-value", Value::text(m)),
    }
}

/// A WIT `result<T, api-error>`.
fn api_result<T>(r: Result<T, ApiError>, ok: impl FnOnce(T) -> Value) -> Value {
    match r {
        Ok(v) => entry("ok", ok(v)),
        Err(e) => entry("err", api_error(&e)),
    }
}

fn opt_text(s: Option<String>) -> Value {
    s.map_or(Value::Null, Value::Text)
}

fn content_kind(name: &str) -> Result<ContentKind, String> {
    ContentKind::parse(name).ok_or_else(|| format!("invalid content kind {name:?}"))
}

fn pos_value(p: Pos) -> Value {
    Value::Map(vec![("x".into(), Value::Int(p.x.into())), ("y".into(), Value::Int(p.y.into()))])
}

fn target_of(v: &Value) -> Result<ChannelTarget, String> {
    let Value::Map(m) = v else { return Err("a channel target is a one-entry map".into()) };
    match m.as_slice() {
        [(tag, payload)] => match (tag.as_str(), payload) {
            ("local", _) => Ok(ChannelTarget::Local),
            ("server", _) => Ok(ChannelTarget::Server),
            ("all-players", _) => Ok(ChannelTarget::AllPlayers),
            ("player", Value::Text(id)) => Ok(ChannelTarget::Player(id.clone())),
            _ => Err(format!("invalid channel target {tag:?}")),
        },
        _ => Err("a channel target is a one-entry map".into()),
    }
}

fn remote_target_of(v: &Value) -> Result<RemoteTarget, String> {
    let Value::Map(m) = v else { return Err("a remote target is a one-entry map".into()) };
    match m.as_slice() {
        [(tag, payload)] => match (tag.as_str(), payload) {
            ("server", _) => Ok(RemoteTarget::Server),
            ("player", Value::Text(id)) => Ok(RemoteTarget::Player(id.clone())),
            _ => Err(format!("invalid remote target {tag:?}")),
        },
        _ => Err("a remote target is a one-entry map".into()),
    }
}

fn spec_of(v: &Value) -> Result<CommandSpec, String> {
    let field = |k: &str| v.get(k).ok_or_else(|| format!("command spec has no {k}"));
    let as_text = |k: &str| match field(k)? {
        Value::Text(t) => Ok(t.clone()),
        _ => Err(format!("command spec {k} must be text")),
    };
    let aliases = match field("aliases")? {
        Value::List(items) => items
            .iter()
            .map(|i| match i {
                Value::Text(t) => Ok(t.clone()),
                _ => Err("command aliases must be text".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("command spec aliases must be a list".into()),
    };
    let role = as_text("min-role")?;
    Ok(CommandSpec {
        name: as_text("name")?,
        aliases,
        description: as_text("description")?,
        usage: as_text("usage")?,
        min_role: Role::parse(&role).ok_or_else(|| format!("unknown role {role:?}"))?,
        console: matches!(field("console")?, Value::Bool(true)),
    })
}

/// An enum argument given by its WIT name.
fn named<T>(a: &[Value], i: usize, parse: fn(&str) -> Option<T>) -> Result<T, String> {
    let name = text(a, i)?;
    parse(name).ok_or_else(|| format!("unknown name {name:?} in argument {i}"))
}

fn str_field(v: &Value, key: &str) -> Result<String, String> {
    match v.get(key) {
        Some(Value::Text(t)) => Ok(t.clone()),
        _ => Err(format!("{key} must be text")),
    }
}

fn opt_str_field(v: &Value, key: &str) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Text(t)) => Ok(Some(t.clone())),
        _ => Err(format!("{key} must be text or none")),
    }
}

fn widget_of(v: &Value) -> Result<Widget, String> {
    Ok(Widget {
        slot: named(&[v.get("slot").cloned().unwrap_or(Value::Null)], 0, Slot::parse)?,
        kind: named(&[v.get("kind").cloned().unwrap_or(Value::Null)], 0, WidgetKind::parse)?,
        icon: opt_str_field(v, "icon")?,
        title: opt_str_field(v, "title")?,
        text: str_field(v, "text")?,
    })
}

fn dialog_of(v: &Value) -> Result<Dialog, String> {
    let field = match v.get("field") {
        None | Some(Value::Null) => None,
        Some(f) => Some(Field {
            placeholder: str_field(f, "placeholder")?,
            initial: str_field(f, "initial")?,
            max_length: match f.get("max-length") {
                Some(Value::Int(n)) => u32::try_from(*n).map_err(|_| "max-length is out of range".to_string())?,
                _ => return Err("max-length must be an integer".into()),
            },
        }),
    };
    let Some(Value::List(items)) = v.get("buttons") else { return Err("buttons must be a list".into()) };
    let buttons = items
        .iter()
        .map(|b| {
            Ok(DialogButton {
                label: str_field(b, "label")?,
                kind: named(&[b.get("kind").cloned().unwrap_or(Value::Null)], 0, ButtonKind::parse)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Dialog { title: str_field(v, "title")?, text: str_field(v, "text")?, field, buttons })
}

fn num_field(v: &Value, key: &str) -> Result<f64, String> {
    match v.get(key) {
        Some(Value::Float(f)) => Ok(*f),
        Some(Value::Int(i)) => Ok(*i as f64),
        _ => Err(format!("{key} must be a number")),
    }
}

fn uint_field<T: TryFrom<i64>>(v: &Value, key: &str) -> Result<T, String> {
    match v.get(key) {
        Some(Value::Int(i)) => T::try_from(*i).map_err(|_| format!("{key} is out of range")),
        _ => Err(format!("{key} must be an integer")),
    }
}

fn bool_field(v: &Value, key: &str) -> Result<bool, String> {
    match v.get(key) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(format!("{key} must be a bool")),
    }
}

/// A `list<enum>` as a mask.
fn mask_of<T: Copy>(v: &Value, parse: fn(&str) -> Option<T>, bit: fn(T) -> u8) -> Result<u8, String> {
    let Value::List(items) = v else { return Err("expected a list of names".into()) };
    items.iter().try_fold(0, |m, i| Ok(m | bit(named(std::slice::from_ref(i), 0, parse)?)))
}

fn button_of(v: &Value) -> Result<Button, String> {
    Ok(Button {
        icon: str_field(v, "icon")?,
        label: opt_str_field(v, "label")?,
        x: num_field(v, "x")? as f32,
        y: num_field(v, "y")? as f32,
        size: uint_field(v, "size")?,
        opacity: uint_field(v, "opacity")?,
    })
}

fn icons_of(v: &Value) -> Result<Icons, String> {
    Ok(Icons {
        speaking: bool_field(v, "speaking")?,
        no_voice: bool_field(v, "no-voice")?,
        self_mute: bool_field(v, "self-mute")?,
        deafen: bool_field(v, "deafen")?,
        admin_mute: bool_field(v, "admin-mute")?,
    })
}

fn icons_value(i: Icons) -> Value {
    Value::Map(vec![
        ("admin-mute".into(), Value::Bool(i.admin_mute)),
        ("deafen".into(), Value::Bool(i.deafen)),
        ("no-voice".into(), Value::Bool(i.no_voice)),
        ("self-mute".into(), Value::Bool(i.self_mute)),
        ("speaking".into(), Value::Bool(i.speaking)),
    ])
}

/// How a `voice.*` reply becomes a value.
type ShowReply = fn(VoiceReply) -> Value;

/// `voice.*`: the call and how to show its reply.
fn voice_call(f: &str, a: &[Value]) -> Result<(VoiceCall, ShowReply), String> {
    let unit: ShowReply = |_| Value::Null;
    let boolean: ShowReply = |r| Value::Bool(r.as_bool());
    let percent: ShowReply = |r| Value::Int(r.as_percent().into());
    let p = |i| text(a, i).map(str::to_string);
    let pct = |i| u16::try_from(int(a, i)?).map_err(|_| format!("argument {i} is out of range"));
    Ok(match f {
        "voice.state" => (VoiceCall::State(p(0)?), |r| Value::text(r.as_state().name())),
        "voice.icons" => (VoiceCall::Icons, |r| icons_value(r.as_icons())),
        "voice.set-routes" => {
            let Value::List(items) = arg(a, 1)? else { return Err("routes must be a list".into()) };
            let routes = items
                .iter()
                .map(|r| {
                    Ok(Route {
                        listener: str_field(r, "listener")?,
                        modes: mask_of(r.get("modes").unwrap_or(&Value::Null), Mode::parse, Mode::bit)?,
                        gain: num_field(r, "gain")? as f32,
                        pan: num_field(r, "pan")? as f32,
                        muffle: num_field(r, "muffle")? as f32,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            (VoiceCall::SetRoutes(p(0)?, routes), unit)
        }
        "voice.mute" => {
            let until = match arg(a, 1)? {
                Value::Null => None,
                _ => Some(u64::try_from(int(a, 1)?).map_err(|_| "until-ms is out of range".to_string())?),
            };
            (VoiceCall::Mute(p(0)?, until), unit)
        }
        "voice.unmute" => (VoiceCall::Unmute(p(0)?), boolean),
        "voice.set-speaker-gain" => (VoiceCall::SetSpeakerGain(p(0)?, pct(1)?), unit),
        "voice.set-caps" => {
            let c = arg(a, 0)?;
            let caps = Caps {
                max_bitrate: uint_field(c, "max-bitrate")?,
                max_speakers: uint_field(c, "max-speakers")?,
                max_frames_per_sec: uint_field(c, "max-frames-per-sec")?,
            };
            (VoiceCall::SetCaps(caps), unit)
        }
        "voice.set-transport" => {
            let c = arg(a, 0)?;
            let mode = named(&[c.get("mode").cloned().unwrap_or(Value::Null)], 0, Transport::parse)?;
            let udp_port = match c.get("udp-port") {
                None | Some(Value::Null) => None,
                Some(_) => Some(uint_field(c, "udp-port")?),
            };
            (VoiceCall::SetTransport(TransportConfig { mode, udp_port }), unit)
        }
        "voice.set-allowed" => {
            let m = mask_of(arg(a, 1)?, Mode::parse, Mode::bit)?;
            let act = mask_of(arg(a, 2)?, Activation::parse, Activation::bit)?;
            (VoiceCall::SetAllowed(p(0)?, m, act), unit)
        }
        "voice.set-icons" => (VoiceCall::SetIcons(icons_of(arg(a, 0)?)?), unit),
        "voice.set-broadcast-overrides-deafen" => (VoiceCall::SetBroadcastOverridesDeafen(flag(a, 0)?), unit),
        "voice.set-default-mode" => (VoiceCall::SetDefaultMode(named(a, 0, Mode::parse)?), unit),
        "voice.set-activation" => (VoiceCall::SetActivation(named(a, 0, Activation::parse)?), unit),
        "voice.talk" => {
            let mode = match arg(a, 0)? {
                Value::Null => None,
                _ => Some(named(a, 0, Mode::parse)?),
            };
            (VoiceCall::Talk(mode, flag(a, 1)?), unit)
        }
        "voice.set-mic" => (VoiceCall::SetMic(flag(a, 0)?), unit),
        "voice.set-self-mute" => (VoiceCall::SetSelfMute(flag(a, 0)?), unit),
        "voice.set-deafen" => (VoiceCall::SetDeafen(flag(a, 0)?), unit),
        "voice.set-vad-threshold" => {
            let db = match arg(a, 0)? {
                Value::Float(f) => *f as f32,
                _ => int(a, 0)? as f32,
            };
            (VoiceCall::SetVadThreshold(db), unit)
        }
        "voice.set-master" => (VoiceCall::SetMaster(pct(0)?), unit),
        "voice.master" => (VoiceCall::Master, percent),
        "voice.set-player-volume" => (VoiceCall::SetPlayerVolume(p(0)?, pct(1)?), unit),
        "voice.player-volume" => (VoiceCall::PlayerVolume(p(0)?), percent),
        "voice.mute-player" => (VoiceCall::MutePlayer(p(0)?, flag(a, 1)?), unit),
        "voice.player-muted" => (VoiceCall::PlayerMuted(p(0)?), boolean),
        other => return Err(format!("unknown host function {other:?} (this host does not implement it)")),
    })
}

/// The WIT imports, by name (`interface.function`; resource methods `interface.resource.method` with the
/// resource's key first).
fn call_host(h: &mut dyn HostApi, f: &str, a: &[Value]) -> Result<Value, String> {
    Ok(match f {
        "host.api-version" => Value::Text(h.api_version()),
        "host.mod-id" => Value::Text(h.mod_id()),
        "host.mod-version" => Value::Text(h.mod_version()),
        "host.current-side" => Value::text(h.current_side().name()),
        "host.host-name" => Value::Text(h.host_name()),

        "log.log" => {
            let level = text(a, 0)?;
            let level = LogLevel::parse(level).ok_or_else(|| format!("invalid log level {level:?}"))?;
            h.log(level, text(a, 1)?);
            Value::Null
        }

        "players.get" => opt_text(h.players_get(text(a, 0)?)),
        "players.find-by-alias" => opt_text(h.players_find_by_alias(text(a, 0)?)),
        "players.from-value-handle" => opt_text(h.player_of_handle(int(a, 0)? as u64).ok()),
        "players.online" => Value::List(h.players_online().into_iter().map(Value::Text).collect()),
        "players.broadcast" => api_result(h.players_broadcast(text(a, 0)?), |()| Value::Null),
        "players.player.id" => Value::text(text(a, 0)?),
        "players.player.alias" => Value::Text(h.player_alias(text(a, 0)?).unwrap_or_default()),
        "players.player.is-online" => Value::Bool(h.player_is_online(text(a, 0)?)),
        "players.player.role" => Value::text(h.player_role(text(a, 0)?).unwrap_or(Role::Player).name()),
        "players.player.send-chat" => api_result(h.player_send_chat(text(a, 0)?, text(a, 1)?), |()| Value::Null),
        "players.player.pos" => h.player_pos(text(a, 0)?).ok().flatten().map_or(Value::Null, pos_value),
        "players.player.value-handle" => Value::Int(h.handle_of_player(text(a, 0)?) as i64),
        "players.player.kick" => api_result(h.player_kick(text(a, 0)?, text(a, 1)?), |()| Value::Null),

        "events.subscribe" => {
            let kind = text(a, 0)?;
            let kind = EventKind::parse(kind).ok_or_else(|| format!("unknown event kind {kind:?}"))?;
            let prio = text(a, 1)?;
            let prio = Priority::parse(prio).ok_or_else(|| format!("unknown priority {prio:?}"))?;
            api_result(h.subscribe(kind, prio, flag(a, 2)?), |id| Value::Int(id.into()))
        }
        "events.subscribe-custom" => {
            let prio = text(a, 1)?;
            let prio = Priority::parse(prio).ok_or_else(|| format!("unknown priority {prio:?}"))?;
            api_result(h.subscribe_custom(text(a, 0)?, prio, flag(a, 2)?), |id| Value::Int(id.into()))
        }
        "events.unsubscribe" => Value::Bool(h.unsubscribe(unsigned(a, 0)?)),
        "events.fire-custom" => api_result(h.fire_custom(text(a, 0)?, bytes_arg(a, 1)?), Value::Bool),

        "commands.register" => api_result(h.register_command(spec_of(arg(a, 0)?)?), |id| Value::Int(id.into())),
        "commands.unregister" => Value::Bool(h.unregister_command(unsigned(a, 0)?)),

        "scheduler.run-later" => api_result(h.run_later(unsigned(a, 0)?), |id| Value::Int(id as i64)),
        "scheduler.run-every" => api_result(h.run_every(unsigned(a, 0)?, unsigned(a, 1)?), |id| Value::Int(id as i64)),
        "scheduler.run-next-poll" => api_result(h.run_next_poll(), |id| Value::Int(id as i64)),
        "scheduler.cancel" => Value::Bool(h.cancel_task(int(a, 0)? as TaskId)),

        "settings.get" => h.settings_get(text(a, 0)?).map_or(Value::Null, Value::Bytes),
        "settings.all" => Value::Bytes(h.settings_all()),
        "settings.set" => api_result(h.settings_set(text(a, 0)?, bytes_arg(a, 1)?), |()| Value::Null),
        "storage.get" => h.storage_get(text(a, 0)?).map_or(Value::Null, Value::Bytes),
        "storage.set" => api_result(h.storage_set(text(a, 0)?, bytes_arg(a, 1)?), |()| Value::Null),
        "storage.delete" => Value::Bool(h.storage_delete(text(a, 0)?)),
        "storage.keys" => Value::List(h.storage_keys(text(a, 0)?).into_iter().map(Value::Text).collect()),

        "services.provide" => api_result(h.provide_service(text(a, 0)?, text(a, 1)?), |()| Value::Null),
        "services.call" => match h.call_service(text(a, 0)?, text(a, 1)?, text(a, 2)?, bytes_arg(a, 3)?) {
            Ok(v) => entry("ok", Value::Bytes(v)),
            Err(e) => entry("err", call_error(&e)),
        },

        "services.call-remote" => {
            let to = remote_target_of(arg(a, 0)?)?;
            let id = h.call_remote(to, text(a, 1)?, text(a, 2)?, text(a, 3)?, bytes_arg(a, 4)?, unsigned(a, 5)?);
            api_result(id, |id| Value::Int(id.into()))
        }
        "services.cancel-remote" => Value::Bool(h.cancel_remote(unsigned(a, 0)?)),
        "services.caller" => h.remote_caller().map_or(Value::Null, |c| {
            Value::Map(vec![("mod-id".into(), Value::Text(c.mod_id)), ("player".into(), opt_text(c.player))])
        }),

        "channels.open" => api_result(h.channel_open(text(a, 0)?), |id| Value::Int(id.into())),
        "channels.send" => {
            let target = target_of(arg(a, 1)?)?;
            api_result(h.channel_send(unsigned(a, 0)?, target, bytes_arg(a, 2)?), |()| Value::Null)
        }
        "channels.close" => Value::Bool(h.channel_close(unsigned(a, 0)?)),

        "content.id-of" => {
            h.content_id(content_kind(text(a, 0)?)?, text(a, 1)?).map_or(Value::Null, |id| Value::Int(id as i64))
        }
        "content.key-of" => opt_text(h.content_key(content_kind(text(a, 0)?)?, u64::from(unsigned(a, 1)?))),
        "content.cosmetics" => Value::List(
            h.content_cosmetics()
                .into_iter()
                .map(|c| {
                    Value::Map(vec![
                        ("key".into(), Value::Text(c.key)),
                        ("mod-id".into(), Value::Text(c.mod_id)),
                        ("slot".into(), Value::Text(c.slot)),
                        ("id".into(), c.id.map_or(Value::Null, |id| Value::Int(id as i64))),
                    ])
                })
                .collect(),
        ),
        "content.equipped" => api_result(h.content_equipped(text(a, 0)?), |worn| {
            Value::List(
                worn.into_iter().map(|(slot, key)| Value::List(vec![Value::Text(slot), Value::Text(key)])).collect(),
            )
        }),
        "content.equip" => api_result(h.content_equip(text(a, 0)?, text(a, 1)?), |()| Value::Null),
        "content.unequip" => api_result(h.content_unequip(text(a, 0)?, text(a, 1)?), |()| Value::Null),
        "content.give-item" => {
            api_result(h.content_give_item(text(a, 0)?, text(a, 1)?, unsigned(a, 2)?), |()| Value::Null)
        }
        "content.place-block" => {
            api_result(h.content_place_block(signed(a, 0)?, signed(a, 1)?, text(a, 2)?), |()| Value::Null)
        }
        "content.block-at" => opt_text(h.content_block_at(signed(a, 0)?, signed(a, 1)?)),

        "ui.set-widget" => api_result(h.ui_set_widget(text(a, 0)?, widget_of(arg(a, 1)?)?), |()| Value::Null),
        "ui.remove-widget" => Value::Bool(h.ui_remove_widget(text(a, 0)?)),
        "ui.toast" => api_result(h.ui_toast(text(a, 0)?), |()| Value::Null),
        "ui.notice" => {
            let level = named(a, 1, NoticeLevel::parse)?;
            api_result(h.ui_notice(text(a, 0)?, level), |()| Value::Null)
        }
        "ui.show-dialog" => api_result(h.ui_show_dialog(dialog_of(arg(a, 0)?)?), |id| Value::Int(id.into())),
        "ui.close-dialog" => Value::Bool(h.ui_close_dialog(unsigned(a, 0)?)),
        "ui.set-overhead" => {
            api_result(h.ui_set_overhead(text(a, 0)?, named(a, 1, OverheadIcon::parse)?), |()| Value::Null)
        }
        "ui.clear-overhead" => Value::Bool(h.ui_clear_overhead(text(a, 0)?)),
        "ui.set-button" => api_result(h.ui_set_button(text(a, 0)?, button_of(arg(a, 1)?)?), |()| Value::Null),
        "ui.remove-button" => Value::Bool(h.ui_remove_button(text(a, 0)?)),
        "ui.open-settings" => api_result(h.ui_open_settings(text(a, 0)?), |()| Value::Null),

        "terrain.solid-count" => {
            let n = h.terrain_solid_count(signed(a, 0)?, signed(a, 1)?, signed(a, 2)?, signed(a, 3)?);
            api_result(n, |n| Value::Int(n.into()))
        }
        "input.bind" => api_result(h.input_bind(text(a, 0)?, text(a, 1)?), |()| Value::Null),
        "input.unbind" => Value::Bool(h.input_unbind(text(a, 0)?)),
        "input.key-of" => opt_text(h.input_key_of(text(a, 0)?)),

        f if f.starts_with("voice.") => {
            let (call, show) = voice_call(f, a)?;
            api_result(h.voice(call), show)
        }

        other => return Err(format!("unknown host function {other:?} (this host does not implement it)")),
    })
}

// -- guest calls (Rust -> Java) -----------------------------------------------------------------------------------

fn player_list(ids: &[String]) -> Value {
    Value::List(ids.iter().cloned().map(Value::Text).collect())
}

fn sender_value(s: &CommandSender) -> Value {
    match s {
        CommandSender::Console => entry("console", Value::Null),
        CommandSender::Player(id) => entry("player", Value::text(id)),
    }
}

fn source_value(s: &ChannelSource) -> Value {
    match s {
        ChannelSource::Local(m) => entry("local", Value::text(m)),
        ChannelSource::Server => entry("server", Value::Null),
        ChannelSource::Player(id) => entry("player", Value::text(id)),
    }
}

fn record(fields: Vec<(&str, Value)>) -> Value {
    Value::Map(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The `events.event` variant the mod's `handle-event` receives.
pub fn event_value(ev: &Event) -> Value {
    let opt = |p: &Option<String>| p.clone().map_or(Value::Null, Value::Text);
    let (tag, rec) = match ev {
        Event::PlayerJoin { player } => ("player-join", record(vec![("player", Value::text(player))])),
        Event::PlayerQuit { player, was_kick } => {
            ("player-quit", record(vec![("player", Value::text(player)), ("was-kick", Value::Bool(*was_kick))]))
        }
        Event::Chat { player, text, cancelled } => (
            "chat",
            record(vec![
                ("player", Value::text(player)),
                ("text", Value::text(text)),
                ("cancelled", Value::Bool(*cancelled)),
            ]),
        ),
        Event::Command { sender, name, args, raw, cancelled } => (
            "command",
            record(vec![
                ("sender", sender_value(sender)),
                ("name", Value::text(name)),
                ("args", player_list(args)),
                ("raw", Value::text(raw)),
                ("cancelled", Value::Bool(*cancelled)),
            ]),
        ),
        Event::Tick { tick, dt } => {
            ("tick", record(vec![("tick", Value::Int(*tick as i64)), ("dt", Value::Float(f64::from(*dt)))]))
        }
        Event::WorldLoaded { world_name, world_id } => {
            ("world-loaded", record(vec![("world-name", Value::text(world_name)), ("world-id", Value::text(world_id))]))
        }
        Event::WorldSave { phase } => ("world-save", record(vec![("phase", Value::text(phase.name()))])),
        Event::PlayerPreLogin { player, alias, cancelled, reason } => (
            "player-pre-login",
            record(vec![
                ("player-id", Value::text(player)),
                ("alias", Value::text(alias)),
                ("cancelled", Value::Bool(*cancelled)),
                ("reason", Value::text(reason.name())),
            ]),
        ),
        Event::BlockPlace { player, pos, item, data_a, data_b, cancelled } => (
            "block-place",
            record(vec![
                ("player", Value::text(player)),
                ("at", pos_value(*pos)),
                ("item", Value::Int(i64::from(*item))),
                ("data-a", Value::Int(i64::from(*data_a))),
                ("data-b", Value::Int(i64::from(*data_b))),
                ("cancelled", Value::Bool(*cancelled)),
            ]),
        ),
        Event::BlockBreak { player, pos, layer, tile, cancelled } => (
            "block-break",
            record(vec![
                ("player", Value::text(player)),
                ("at", pos_value(*pos)),
                ("layer", Value::text(layer.name())),
                ("tile", Value::Int(i64::from(*tile))),
                ("cancelled", Value::Bool(*cancelled)),
            ]),
        ),
        Event::BlockChanged { pos, tile, player } => (
            "block-changed",
            record(vec![("at", pos_value(*pos)), ("tile", Value::Int(i64::from(*tile))), ("player", opt(player))]),
        ),
        Event::ObjectPlace { player, pos, object, item, cancelled } => (
            "object-place",
            record(vec![
                ("player", Value::text(player)),
                ("at", pos_value(*pos)),
                ("object", Value::text(object.name())),
                ("item", Value::Int(i64::from(*item))),
                ("cancelled", Value::Bool(*cancelled)),
            ]),
        ),
        Event::Damage { player, blockhead, amount, attacker, cancelled } => (
            "damage",
            record(vec![
                ("player", opt(player)),
                ("blockhead", Value::Int(*blockhead as i64)),
                ("amount", Value::Float(f64::from(*amount))),
                ("attacker", opt(attacker)),
                ("cancelled", Value::Bool(*cancelled)),
            ]),
        ),
        Event::Death { player, blockhead } => {
            ("death", record(vec![("player", opt(player)), ("blockhead", Value::Int(*blockhead as i64))]))
        }
        Event::PlayerMove { player, from, to } => (
            "player-move",
            record(vec![("player", Value::text(player)), ("from", pos_value(*from)), ("to", pos_value(*to))]),
        ),
        Event::Custom { name, source, payload, cancelled } => (
            "custom",
            record(vec![
                ("name", Value::text(name)),
                ("source", Value::text(source)),
                ("payload", Value::Bytes(payload.clone())),
                ("cancelled", Value::Bool(*cancelled)),
            ]),
        ),
    };
    entry(tag, rec)
}

fn outcome_of(v: &Value) -> Result<Outcome, String> {
    let cancelled = matches!(v.get("cancelled"), Some(Value::Bool(true)));
    let mutation = match v.get("mutation") {
        None | Some(Value::Null) => None,
        Some(Value::Map(m)) => match m.as_slice() {
            [(tag, payload)] => Some(match (tag.as_str(), payload) {
                ("chat-text", Value::Text(t)) => Mutation::ChatText(t.clone()),
                ("custom-payload", Value::Bytes(b)) => Mutation::CustomPayload(b.clone()),
                ("reject-reason", Value::Text(t)) => Mutation::RejectReason(
                    RejectReason::parse(t).ok_or_else(|| format!("invalid argument: reject reason {t:?}"))?,
                ),
                ("damage-amount", Value::Float(f)) => Mutation::DamageAmount(*f as f32),
                ("damage-amount", Value::Int(i)) => Mutation::DamageAmount(*i as f32),
                _ => return Err(format!("invalid mutation {tag:?}")),
            }),
            _ => return Err("a mutation is a one-entry map".into()),
        },
        Some(_) => return Err("a mutation is a one-entry map".into()),
    };
    Ok(Outcome { cancelled, mutation })
}

fn command_outcome_of(v: &Value) -> Result<CommandOutcome, String> {
    let handled = matches!(v.get("handled"), Some(Value::Bool(true)));
    let reply = match v.get("reply") {
        Some(Value::List(items)) => items
            .iter()
            .map(|i| match i {
                Value::Text(t) => Ok(t.clone()),
                _ => Err("a command reply line must be text".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("a command outcome needs a reply list".into()),
    };
    Ok(CommandOutcome { handled, reply })
}

// -- the runtime --------------------------------------------------------------------------------------------------

/// The JVM runtime.
pub struct JvmRuntime {
    vm: Arc<Vm>,
    cfg: JvmConfig,
}

impl JvmRuntime {
    /// Starts (or joins) the process' JVM. Fails with a readable message when no Java runtime is found. (Not on
    /// Android, where the JVM is ART: use [`attach_existing`].)
    #[cfg(not(target_os = "android"))]
    pub fn new(cfg: JvmConfig) -> Result<JvmRuntime, String> {
        let vm = vm_with(&cfg, start)?;
        Ok(JvmRuntime { vm, cfg })
    }

    /// The `libjvm` that [`JvmRuntime::new`] would load with this configuration, without starting anything.
    #[cfg(not(target_os = "android"))]
    pub fn find_jvm(cfg: &JvmConfig) -> Result<PathBuf, String> {
        locate(cfg)
    }
}

static NEXT_INSTANCE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

impl Runtime for JvmRuntime {
    fn kind(&self) -> bhmod_manifest::manifest::Runtime {
        bhmod_manifest::manifest::Runtime::Jvm
    }

    fn instantiate(
        &self,
        mod_id: &str,
        files: &ModFiles,
        _limits: InstanceLimits,
    ) -> Result<Box<dyn Instance>, String> {
        let Some(jar) = files.files.get(&files.entry) else {
            return Err(format!("{mod_id}: entry point {} is missing", files.entry));
        };
        let inst = NEXT_INSTANCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let vm = self.vm.clone();
        let create = |env: &mut JNIEnv| {
            let id = env.new_string(mod_id)?;
            let jar = env.byte_array_from_slice(jar)?;
            let args = [JValue::Int(inst), JValue::Object(&id), JValue::Object(&jar)];
            invoke_static(env, &vm, "create", "(ILjava/lang/String;[B)[B", &args)
        };
        let r = with_env(&vm, create)?;
        match status(&r)? {
            (0, _) => Ok(Box::new(JvmInstance { vm, inst, cfg: self.cfg.clone() })),
            (_, msg) => Err(format!("{mod_id}: {}", payload_text(&msg))),
        }
    }
}

/// Runs `f` on an attached thread inside a local frame.
fn with_env<T>(vm: &Vm, f: impl FnOnce(&mut JNIEnv) -> jni::errors::Result<T>) -> Result<T, String> {
    let mut env = vm.jvm.attach_current_thread_permanently().map_err(jerr)?;
    let r = env.with_local_frame(32, |env| f(env));
    r.map_err(|e| {
        let text = take_exception(&mut env).unwrap_or_else(|| e.to_string());
        format!("JNI call failed: {text}")
    })
}

/// The pending Java exception as text; cleared.
fn take_exception(env: &mut JNIEnv) -> Option<String> {
    if !env.exception_check().unwrap_or(false) {
        return None;
    }
    let ex = env.exception_occurred().ok()?;
    let _ = env.exception_clear();
    let s = env.call_method(&ex, "toString", "()Ljava/lang/String;", &[]).ok()?.l().ok()?;
    let s: String = env.get_string(&JString::from(s)).ok()?.into();
    Some(s)
}

fn invoke_static(env: &mut JNIEnv, vm: &Vm, name: &str, sig: &str, args: &[JValue]) -> jni::errors::Result<Vec<u8>> {
    let out = env.call_static_method(glue_class(vm), name, sig, args)?.l()?;
    env.convert_byte_array(JByteArray::from(out))
}

/// `[status, payload]` from the glue: status 0 ok, 1 exception, 2 fatal.
fn status(bytes: &[u8]) -> Result<(i64, Value), String> {
    match Value::decode(bytes).map_err(jerr)? {
        Value::List(mut items) if items.len() == 2 => {
            let payload = items.pop().expect("two items");
            match items.pop().expect("two items") {
                Value::Int(s) => Ok((s, payload)),
                _ => Err("the JVM glue returned a malformed status".into()),
            }
        }
        _ => Err("the JVM glue returned a malformed result".into()),
    }
}

/// The glue encodes a service result itself, so anything but bytes means the glue is broken.
fn bytes_of(v: Value) -> Result<Vec<u8>, String> {
    match v {
        Value::Bytes(b) => Ok(b),
        other => Err(format!("call-service returned {other:?}, not a value")),
    }
}

fn payload_text(v: &Value) -> String {
    match v {
        Value::Text(t) => t.clone(),
        other => format!("{other:?}"),
    }
}

/// One loaded JVM mod.
pub struct JvmInstance {
    vm: Arc<Vm>,
    inst: i32,
    cfg: JvmConfig,
}

impl JvmInstance {
    /// Calls a `guest` export. `soft`: a Java exception is [`CallError::Failed`] (a deliberate error), not a fault.
    fn call(&mut self, host: &mut dyn HostApi, func: &str, args: Vec<Value>, soft: bool) -> Result<Value, CallError> {
        let grace = self.cfg.grace + if func == "on-enable" { self.cfg.enable_grace } else { Duration::ZERO };
        let token = self.vm.watchdog.arm(self.inst, host.deadline() + grace);
        let entered = enter(self.inst, host);
        let r = with_env(&self.vm, |env| {
            let name = env.new_string(func)?;
            let a = env.byte_array_from_slice(&Value::List(args).encode())?;
            invoke_static(
                env,
                &self.vm,
                "invoke",
                "(ILjava/lang/String;[B)[B",
                &[JValue::Int(self.inst), JValue::Object(&name), JValue::Object(&a)],
            )
        });
        drop(entered);
        let fired = self.vm.watchdog.disarm(token);
        if fired {
            return Err(CallError::Timeout);
        }
        let (code, payload) = status(&r.map_err(CallError::Trap)?).map_err(CallError::Trap)?;
        let msg = || payload_text(&payload);
        match code {
            0 => Ok(payload),
            1 if soft => Err(CallError::Failed(msg())),
            _ => Err(CallError::Trap(msg())),
        }
    }
}

impl Drop for JvmInstance {
    fn drop(&mut self) {
        let _ = with_env(&self.vm, |env| {
            env.call_static_method(glue_class(&self.vm), "destroy", "(I)V", &[JValue::Int(self.inst)]).map(|_| ())
        });
    }
}

impl Instance for JvmInstance {
    fn on_enable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        self.call(host, "on-enable", vec![], true).map(|_| ()).map_err(|e| match e {
            CallError::Trap(m) => CallError::Failed(m),
            other => other,
        })
    }

    fn on_disable(&mut self, host: &mut dyn HostApi) -> Result<(), CallError> {
        self.call(host, "on-disable", vec![], false).map(|_| ())
    }

    fn handle_event(&mut self, host: &mut dyn HostApi, sub: SubscriptionId, ev: &Event) -> Result<Outcome, CallError> {
        let v = self.call(host, "handle-event", vec![Value::Int(sub.into()), event_value(ev)], false)?;
        let mut out = outcome_of(&v).map_err(CallError::Trap)?;
        out.cancelled &= ev.kind().cancellable();
        Ok(out)
    }

    fn run_command(
        &mut self,
        host: &mut dyn HostApi,
        id: CommandId,
        sender: &CommandSender,
        label: &str,
        args: &[String],
    ) -> Result<CommandOutcome, CallError> {
        let a = vec![Value::Int(id.into()), sender_value(sender), Value::text(label), player_list(args)];
        let v = self.call(host, "run-command", a, false)?;
        command_outcome_of(&v).map_err(CallError::Trap)
    }

    fn run_task(&mut self, host: &mut dyn HostApi, id: TaskId) -> Result<(), CallError> {
        self.call(host, "run-task", vec![Value::Int(id as i64)], false).map(|_| ())
    }

    fn call_service(
        &mut self,
        host: &mut dyn HostApi,
        name: &str,
        method: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, CallError> {
        let a = vec![Value::text(name), Value::text(method), Value::Bytes(args.to_vec())];
        bytes_of(self.call(host, "call-service", a, true)?).map_err(CallError::Trap)
    }

    fn on_channel_message(
        &mut self,
        host: &mut dyn HostApi,
        channel: ChannelId,
        source: &ChannelSource,
        data: &[u8],
    ) -> Result<(), CallError> {
        let a = vec![Value::Int(channel.into()), source_value(source), Value::Bytes(data.to_vec())];
        self.call(host, "on-channel-message", a, false).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::RemoteCaller;
    use crate::types::{BreakLayer, ObjectKind, SavePhase, Side};

    /// A host API that records what it is asked and fails on demand.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        err: Option<ApiError>,
        svc_err: Option<ServiceError>,
        caller: Option<RemoteCaller>,
    }

    impl Fake {
        fn res<T>(&mut self, what: &str, ok: T) -> Result<T, ApiError> {
            self.calls.push(what.to_string());
            self.err.clone().map_or(Ok(ok), Err)
        }
    }

    impl HostApi for Fake {
        fn deadline(&self) -> Instant {
            Instant::now() + Duration::from_secs(1)
        }
        fn api_version(&self) -> String {
            "0.2.0".into()
        }
        fn mod_id(&self) -> String {
            "m".into()
        }
        fn mod_version(&self) -> String {
            "1.0.0".into()
        }
        fn current_side(&self) -> Side {
            Side::Server
        }
        fn host_name(&self) -> String {
            "fake".into()
        }
        fn log(&mut self, level: LogLevel, message: &str) {
            self.calls.push(format!("log {} {message}", level.name()));
        }
        fn player_alias(&mut self, id: &str) -> Result<String, ApiError> {
            self.res("alias", format!("alias-{id}"))
        }
        fn player_is_online(&mut self, _id: &str) -> bool {
            true
        }
        fn player_role(&mut self, _id: &str) -> Result<Role, ApiError> {
            self.res("role", Role::Admin)
        }
        fn player_send_chat(&mut self, id: &str, text: &str) -> Result<(), ApiError> {
            self.res(&format!("send-chat {id} {text}"), ())
        }
        fn player_kick(&mut self, id: &str, reason: &str) -> Result<(), ApiError> {
            self.res(&format!("kick {id} {reason}"), ())
        }
        fn players_get(&mut self, id: &str) -> Option<String> {
            (id == "p1").then(|| "p1".to_string())
        }
        fn players_find_by_alias(&mut self, alias: &str) -> Option<String> {
            (alias == "A").then(|| "p1".to_string())
        }
        fn players_online(&mut self) -> Vec<String> {
            vec!["p1".into(), "p2".into()]
        }
        fn players_broadcast(&mut self, text: &str) -> Result<(), ApiError> {
            self.res(&format!("broadcast {text}"), ())
        }
        fn subscribe(&mut self, kind: EventKind, priority: Priority, ignore: bool) -> Result<SubscriptionId, ApiError> {
            self.res(&format!("subscribe {} {} {ignore}", kind.name(), priority.name()), 7)
        }
        fn unsubscribe(&mut self, sub: SubscriptionId) -> bool {
            sub == 7
        }
        fn register_command(&mut self, spec: CommandSpec) -> Result<CommandId, ApiError> {
            self.res(&format!("register {} {:?} {}", spec.name, spec.aliases, spec.console), 3)
        }
        fn unregister_command(&mut self, id: CommandId) -> bool {
            id == 3
        }
        fn run_later(&mut self, delay: u32) -> Result<TaskId, ApiError> {
            self.res(&format!("later {delay}"), 11)
        }
        fn run_every(&mut self, delay: u32, period: u32) -> Result<TaskId, ApiError> {
            self.res(&format!("every {delay} {period}"), 12)
        }
        fn run_next_poll(&mut self) -> Result<TaskId, ApiError> {
            self.res("poll", 13)
        }
        fn cancel_task(&mut self, id: TaskId) -> bool {
            id == 11
        }
        fn storage_get(&mut self, key: &str) -> Option<Vec<u8>> {
            (key == "k").then(|| vec![1, 2])
        }
        fn storage_set(&mut self, key: &str, value: &[u8]) -> Result<(), ApiError> {
            self.res(&format!("set {key} {value:?}"), ())
        }
        fn storage_delete(&mut self, key: &str) -> bool {
            key == "k"
        }
        fn settings_get(&mut self, key: &str) -> Option<Vec<u8>> {
            (key == "radius").then(|| vec![0x18, 32])
        }
        fn settings_all(&mut self) -> Vec<u8> {
            vec![0xa0]
        }
        fn settings_set(&mut self, key: &str, value: &[u8]) -> Result<(), ApiError> {
            self.res(&format!("setting {key} {value:?}"), ())
        }
        fn storage_keys(&mut self, prefix: &str) -> Vec<String> {
            vec![format!("{prefix}a")]
        }
        fn player_pos(&mut self, id: &str) -> Result<Option<Pos>, ApiError> {
            match id {
                "p1" => Ok(Some(Pos { x: 3, y: -4 })),
                "p2" => Ok(None),
                _ => Err(ApiError::StaleHandle),
            }
        }
        fn handle_of_player(&mut self, _id: &str) -> u64 {
            42
        }
        fn player_of_handle(&mut self, handle: u64) -> Result<String, ApiError> {
            if handle == 42 {
                Ok("p1".into())
            } else {
                Err(ApiError::StaleHandle)
            }
        }
        fn subscribe_custom(&mut self, name: &str, _p: Priority, _i: bool) -> Result<SubscriptionId, ApiError> {
            self.res(&format!("custom {name}"), 8)
        }
        fn fire_custom(&mut self, name: &str, payload: &[u8]) -> Result<bool, ApiError> {
            self.res(&format!("fire {name} {payload:?}"), true)
        }
        fn provide_service(&mut self, name: &str, version: &str) -> Result<(), ApiError> {
            self.res(&format!("provide {name} {version}"), ())
        }
        fn call_service(&mut self, name: &str, req: &str, method: &str, args: &[u8]) -> Result<Vec<u8>, ServiceError> {
            self.calls.push(format!("call {name} {req} {method} {args:?}"));
            self.svc_err.clone().map_or(Ok(vec![0xf5]), Err)
        }
        fn call_remote(
            &mut self,
            to: RemoteTarget,
            name: &str,
            req: &str,
            method: &str,
            args: &[u8],
            timeout_ms: u32,
        ) -> Result<u32, ApiError> {
            self.res(&format!("remote {to:?} {name} {req} {method} {args:?} {timeout_ms}"), 11)
        }
        fn cancel_remote(&mut self, id: u32) -> bool {
            id == 11
        }
        fn remote_caller(&mut self) -> Option<RemoteCaller> {
            self.calls.push("caller".into());
            self.caller.clone()
        }
        fn channel_open(&mut self, name: &str) -> Result<ChannelId, ApiError> {
            self.res(&format!("open {name}"), 5)
        }
        fn channel_send(&mut self, c: ChannelId, to: ChannelTarget, data: &[u8]) -> Result<(), ApiError> {
            self.res(&format!("send {c} {to:?} {data:?}"), ())
        }
        fn channel_close(&mut self, c: ChannelId) -> bool {
            c == 5
        }
        fn ui_set_widget(&mut self, id: &str, w: Widget) -> Result<(), ApiError> {
            self.res(&format!("widget {id} {w:?}"), ())
        }
        fn ui_remove_widget(&mut self, id: &str) -> bool {
            id == "w"
        }
        fn content_id(&mut self, kind: ContentKind, key: &str) -> Option<u64> {
            (kind == ContentKind::Cosmetic && key == "m:hat").then_some(5)
        }
        fn content_key(&mut self, kind: ContentKind, id: u64) -> Option<String> {
            (kind == ContentKind::Cosmetic && id == 5).then(|| "m:hat".to_string())
        }
        fn content_cosmetics(&mut self) -> Vec<crate::types::CosmeticInfo> {
            vec![crate::types::CosmeticInfo {
                key: "m:hat".into(),
                mod_id: "m".into(),
                slot: "hat".into(),
                id: Some(5),
            }]
        }
        fn content_equipped(&mut self, id: &str) -> Result<Vec<(String, String)>, ApiError> {
            self.res(&format!("equipped {id}"), vec![("hat".to_string(), "m:hat".to_string())])
        }
        fn content_equip(&mut self, id: &str, key: &str) -> Result<(), ApiError> {
            self.res(&format!("equip {id} {key}"), ())
        }
        fn content_unequip(&mut self, id: &str, slot: &str) -> Result<(), ApiError> {
            self.res(&format!("unequip {id} {slot}"), ())
        }
        fn content_give_item(&mut self, id: &str, key: &str, count: u32) -> Result<(), ApiError> {
            self.res(&format!("give {id} {key} {count}"), ())
        }
        fn content_place_block(&mut self, x: i32, y: i32, key: &str) -> Result<(), ApiError> {
            self.res(&format!("place {x} {y} {key}"), ())
        }
        fn content_block_at(&mut self, x: i32, y: i32) -> Option<String> {
            (x == 3 && y == -4).then(|| "m:brick".to_string())
        }
        fn ui_toast(&mut self, text: &str) -> Result<(), ApiError> {
            self.res(&format!("toast {text}"), ())
        }
        fn ui_notice(&mut self, text: &str, level: NoticeLevel) -> Result<(), ApiError> {
            self.res(&format!("notice {text} {}", level.name()), ())
        }
        fn ui_show_dialog(&mut self, d: Dialog) -> Result<u32, ApiError> {
            self.res(&format!("dialog {d:?}"), 9)
        }
        fn ui_close_dialog(&mut self, id: u32) -> bool {
            id == 9
        }
    }

    fn t(s: &str) -> Value {
        Value::text(s)
    }

    fn call(h: &mut Fake, f: &str, a: &[Value]) -> Value {
        call_host(h, f, a).expect(f)
    }

    fn ok(v: Value) -> Value {
        entry("ok", v)
    }

    #[test]
    fn host_facts_log_and_players() {
        let mut h = Fake::default();
        assert_eq!(call(&mut h, "host.api-version", &[]), t("0.2.0"));
        assert_eq!(call(&mut h, "host.mod-id", &[]), t("m"));
        assert_eq!(call(&mut h, "host.mod-version", &[]), t("1.0.0"));
        assert_eq!(call(&mut h, "host.current-side", &[]), t("server"));
        assert_eq!(call(&mut h, "host.host-name", &[]), t("fake"));
        assert!(h.deadline() > Instant::now());
        assert_eq!(call(&mut h, "log.log", &[t("warn"), t("hi")]), Value::Null);
        assert_eq!(h.calls.last().unwrap(), "log warn hi");
        assert!(call_host(&mut h, "log.log", &[t("loud"), t("hi")]).unwrap_err().contains("log level"));

        assert_eq!(call(&mut h, "players.get", &[t("p1")]), t("p1"));
        assert_eq!(call(&mut h, "players.get", &[t("zz")]), Value::Null);
        assert_eq!(call(&mut h, "players.find-by-alias", &[t("A")]), t("p1"));
        assert_eq!(call(&mut h, "players.from-value-handle", &[Value::Int(42)]), t("p1"));
        assert_eq!(call(&mut h, "players.from-value-handle", &[Value::Int(1)]), Value::Null);
        assert_eq!(call(&mut h, "players.online", &[]), Value::List(vec![t("p1"), t("p2")]));
        assert_eq!(call(&mut h, "players.broadcast", &[t("yo")]), ok(Value::Null));
        assert_eq!(call(&mut h, "players.player.id", &[t("p1")]), t("p1"));
        assert_eq!(call(&mut h, "players.player.alias", &[t("p1")]), t("alias-p1"));
        assert_eq!(call(&mut h, "players.player.is-online", &[t("p1")]), Value::Bool(true));
        assert_eq!(call(&mut h, "players.player.role", &[t("p1")]), t("admin"));
        assert_eq!(call(&mut h, "players.player.send-chat", &[t("p1"), t("x")]), ok(Value::Null));
        assert_eq!(call(&mut h, "players.player.kick", &[t("p1"), t("bye")]), ok(Value::Null));
        assert_eq!(call(&mut h, "players.player.value-handle", &[t("p1")]), Value::Int(42));
        let pos = call(&mut h, "players.player.pos", &[t("p1")]);
        assert_eq!(pos.get("x"), Some(&Value::Int(3)));
        assert_eq!(pos.get("y"), Some(&Value::Int(-4)));
        assert_eq!(call(&mut h, "players.player.pos", &[t("p2")]), Value::Null);
        assert_eq!(call(&mut h, "players.player.pos", &[t("gone")]), Value::Null);
    }

    #[test]
    fn events_commands_scheduler_storage() {
        let mut h = Fake::default();
        assert_eq!(call(&mut h, "events.subscribe", &[t("chat"), t("low"), Value::Bool(true)]), ok(Value::Int(7)));
        assert!(h.calls.contains(&"subscribe chat low true".to_string()));
        assert!(call_host(&mut h, "events.subscribe", &[t("nope"), t("low"), Value::Bool(true)]).is_err());
        assert!(call_host(&mut h, "events.subscribe", &[t("chat"), t("nope"), Value::Bool(true)]).is_err());
        assert_eq!(
            call(&mut h, "events.subscribe-custom", &[t("m:x"), t("normal"), Value::Bool(false)]),
            ok(Value::Int(8))
        );
        assert!(call_host(&mut h, "events.subscribe-custom", &[t("m:x"), t("nope"), Value::Bool(false)]).is_err());
        assert_eq!(call(&mut h, "events.unsubscribe", &[Value::Int(7)]), Value::Bool(true));
        assert_eq!(call(&mut h, "events.fire-custom", &[t("m:x"), Value::Bytes(vec![1])]), ok(Value::Bool(true)));

        let spec = record(vec![
            ("name", t("go")),
            ("aliases", Value::List(vec![t("g")])),
            ("description", t("d")),
            ("usage", t("/go")),
            ("min-role", t("moderator")),
            ("console", Value::Bool(false)),
        ]);
        assert_eq!(call(&mut h, "commands.register", std::slice::from_ref(&spec)), ok(Value::Int(3)));
        assert!(h.calls.contains(&"register go [\"g\"] false".to_string()));
        assert_eq!(call(&mut h, "commands.unregister", &[Value::Int(3)]), Value::Bool(true));
        for (key, bad) in [
            ("name", Value::Int(1)),
            ("aliases", Value::Int(1)),
            ("aliases", Value::List(vec![Value::Int(1)])),
            ("min-role", t("god")),
        ] {
            let Value::Map(mut m) = spec.clone() else { unreachable!() };
            m.retain(|(k, _)| k != key);
            m.push((key.to_string(), bad));
            assert!(call_host(&mut h, "commands.register", &[Value::Map(m)]).is_err(), "{key}");
        }
        let Value::Map(mut m) = spec else { unreachable!() };
        m.retain(|(k, _)| k != "usage");
        assert!(call_host(&mut h, "commands.register", &[Value::Map(m)]).unwrap_err().contains("usage"));

        assert_eq!(call(&mut h, "scheduler.run-later", &[Value::Int(4)]), ok(Value::Int(11)));
        assert_eq!(call(&mut h, "scheduler.run-every", &[Value::Int(4), Value::Int(2)]), ok(Value::Int(12)));
        assert_eq!(call(&mut h, "scheduler.run-next-poll", &[]), ok(Value::Int(13)));
        assert_eq!(call(&mut h, "scheduler.cancel", &[Value::Int(11)]), Value::Bool(true));

        assert_eq!(call(&mut h, "storage.get", &[t("k")]), Value::Bytes(vec![1, 2]));
        assert_eq!(call(&mut h, "storage.get", &[t("z")]), Value::Null);
        assert_eq!(call(&mut h, "storage.set", &[t("k"), Value::Bytes(vec![9])]), ok(Value::Null));
        assert_eq!(call(&mut h, "storage.delete", &[t("k")]), Value::Bool(true));
        assert_eq!(call(&mut h, "storage.keys", &[t("p")]), Value::List(vec![t("pa")]));
        assert_eq!(call(&mut h, "settings.get", &[t("radius")]), Value::Bytes(vec![0x18, 32]));
        assert_eq!(call(&mut h, "settings.get", &[t("z")]), Value::Null);
        assert_eq!(call(&mut h, "settings.all", &[]), Value::Bytes(vec![0xa0]));
        assert_eq!(call(&mut h, "settings.set", &[t("radius"), Value::Bytes(vec![1])]), ok(Value::Null));
        assert!(h.calls.contains(&"setting radius [1]".to_string()));
    }

    #[test]
    fn content_functions() {
        let mut h = Fake::default();
        assert_eq!(call(&mut h, "content.id-of", &[t("cosmetic"), t("m:hat")]), Value::Int(5));
        assert_eq!(call(&mut h, "content.id-of", &[t("block"), t("m:hat")]), Value::Null);
        assert_eq!(call(&mut h, "content.key-of", &[t("cosmetic"), Value::Int(5)]), t("m:hat"));
        assert_eq!(call(&mut h, "content.key-of", &[t("item"), Value::Int(5)]), Value::Null);
        assert_eq!(
            call(&mut h, "content.cosmetics", &[]),
            Value::List(vec![record(vec![
                ("key", t("m:hat")),
                ("mod-id", t("m")),
                ("slot", t("hat")),
                ("id", Value::Int(5)),
            ])])
        );
        assert_eq!(
            call(&mut h, "content.equipped", &[t("p")]),
            ok(Value::List(vec![Value::List(vec![t("hat"), t("m:hat")])]))
        );
        assert_eq!(call(&mut h, "content.equip", &[t("p"), t("m:hat")]), ok(Value::Null));
        assert_eq!(call(&mut h, "content.unequip", &[t("p"), t("hat")]), ok(Value::Null));
        assert!(h.calls.contains(&"equip p m:hat".to_string()));
        assert_eq!(call(&mut h, "content.give-item", &[t("p"), t("m:lamp"), Value::Int(3)]), ok(Value::Null));
        assert!(h.calls.contains(&"give p m:lamp 3".to_string()));
        assert_eq!(
            call(&mut h, "content.place-block", &[Value::Int(1), Value::Int(-2), t("m:brick")]),
            ok(Value::Null)
        );
        assert!(h.calls.contains(&"place 1 -2 m:brick".to_string()));
        assert_eq!(call(&mut h, "content.block-at", &[Value::Int(3), Value::Int(-4)]), t("m:brick"));
        assert_eq!(call(&mut h, "content.block-at", &[Value::Int(0), Value::Int(0)]), Value::Null);
        assert!(call_host(&mut h, "content.block-at", &[Value::Int(i64::MAX), Value::Int(0)])
            .unwrap_err()
            .contains("out of range"));
        assert!(call_host(&mut h, "content.id-of", &[t("sound"), t("m:hat")]).unwrap_err().contains("content kind"));
        assert!(call_host(&mut h, "content.key-of", &[t("cosmetic"), Value::Int(-1)]).is_err());
    }

    #[test]
    fn ui_functions() {
        let mut h = Fake::default();
        let widget = record(vec![
            ("slot", t("top-right")),
            ("kind", t("panel")),
            ("icon", Value::Null),
            ("title", t("T")),
            ("text", t("hi")),
        ]);
        assert_eq!(call(&mut h, "ui.set-widget", &[t("w"), widget.clone()]), ok(Value::Null));
        assert!(h.calls.last().unwrap().contains("TopRight"));
        assert_eq!(call(&mut h, "ui.remove-widget", &[t("w")]), Value::Bool(true));
        assert_eq!(call(&mut h, "ui.toast", &[t("yo")]), ok(Value::Null));
        assert_eq!(call(&mut h, "ui.notice", &[t("n"), t("success")]), ok(Value::Null));
        let dialog = record(vec![
            ("title", t("D")),
            ("text", t("body")),
            ("field", record(vec![("placeholder", t("p")), ("initial", t("i")), ("max-length", Value::Int(20))])),
            ("buttons", Value::List(vec![record(vec![("label", t("OK")), ("kind", t("primary"))])])),
        ]);
        assert_eq!(call(&mut h, "ui.show-dialog", std::slice::from_ref(&dialog)), ok(Value::Int(9)));
        assert_eq!(call(&mut h, "ui.close-dialog", &[Value::Int(9)]), Value::Bool(true));
        let Value::Map(plain) = dialog.clone() else { unreachable!() };
        let mut plain = plain;
        plain.retain(|(k, _)| k != "field");
        assert!(call_host(&mut h, "ui.show-dialog", &[Value::Map(plain)]).is_ok());
        // Bad shapes are errors.
        let bad = |key: &str, v: Value, base: &Value| {
            let Value::Map(mut m) = base.clone() else { unreachable!() };
            m.retain(|(k, _)| k != key);
            m.push((key.to_string(), v));
            Value::Map(m)
        };
        for w in [
            bad("slot", t("middle"), &widget),
            bad("kind", Value::Null, &widget),
            bad("icon", Value::Int(1), &widget),
            bad("text", Value::Int(1), &widget),
        ] {
            assert!(call_host(&mut h, "ui.set-widget", &[t("w"), w]).is_err());
        }
        assert!(call_host(&mut h, "ui.notice", &[t("n"), t("loud")]).is_err());
        let field = |f: Value| bad("field", f, &dialog);
        let fld = |k: &str, v: Value| {
            let base = record(vec![("placeholder", t("p")), ("initial", t("i")), ("max-length", Value::Int(20))]);
            bad(k, v, &base)
        };
        for d in [
            field(fld("max-length", Value::Int(-1))),
            field(fld("max-length", t("x"))),
            bad("buttons", Value::Int(1), &dialog),
            bad("buttons", Value::List(vec![record(vec![("label", t("OK")), ("kind", t("loud"))])]), &dialog),
            bad("title", Value::Int(1), &dialog),
        ] {
            assert!(call_host(&mut h, "ui.show-dialog", &[d]).is_err());
        }
    }

    #[test]
    fn services_channels_and_errors() {
        let mut h = Fake::default();
        assert_eq!(call(&mut h, "services.provide", &[t("eco"), t("1.0.0")]), ok(Value::Null));
        assert_eq!(
            call(&mut h, "services.call", &[t("eco"), t("^1"), t("pay"), Value::Bytes(vec![0x80])]),
            ok(Value::Bytes(vec![0xf5]))
        );
        for (e, tag) in [
            (ServiceError::NotFound, "not-found"),
            (ServiceError::VersionMismatch, "version-mismatch"),
            (ServiceError::NotPermitted, "not-permitted"),
            (ServiceError::Reentrant, "reentrant"),
            (ServiceError::ProviderError("no".into()), "provider-error"),
            (ServiceError::Trap, "trap"),
            (ServiceError::BadValue("bad".into()), "bad-value"),
        ] {
            h.svc_err = Some(e);
            let r = call(&mut h, "services.call", &[t("eco"), t("^1"), t("pay"), Value::Bytes(vec![0x80])]);
            assert!(r.get("err").unwrap().get(tag).is_some(), "{tag}");
        }
        assert_eq!(call(&mut h, "channels.open", &[t("m:c")]), ok(Value::Int(5)));
        for target in [
            entry("local", Value::Null),
            entry("server", Value::Null),
            entry("all-players", Value::Null),
            entry("player", t("p1")),
        ] {
            assert_eq!(call(&mut h, "channels.send", &[Value::Int(5), target, Value::Bytes(vec![1])]), ok(Value::Null));
        }
        assert!(h.calls.contains(&"send 5 Player(\"p1\") [1]".to_string()));
        // remote service calls
        for (to, want) in [
            (entry("server", Value::Null), "remote Server s ^1 m [128] 0"),
            (entry("player", t("p1")), "remote Player(\"p1\") s ^1 m [128] 7000"),
        ] {
            let ms = if want.ends_with(" 0") { 0 } else { 7000 };
            let args = [to, t("s"), t("^1"), t("m"), Value::Bytes(vec![0x80]), Value::Int(ms)];
            assert_eq!(call(&mut h, "services.call-remote", &args), ok(Value::Int(11)));
            assert!(h.calls.contains(&want.to_string()), "{want}");
        }
        for bad in [t("server"), Value::Map(vec![]), entry("player", Value::Int(1)), entry("moon", Value::Null)] {
            let args = [bad, t("s"), t("^1"), t("m"), Value::Bytes(vec![0x80]), Value::Int(0)];
            assert!(call_host(&mut h, "services.call-remote", &args).is_err());
        }
        assert_eq!(call(&mut h, "services.cancel-remote", &[Value::Int(11)]), Value::Bool(true));
        assert_eq!(call(&mut h, "services.cancel-remote", &[Value::Int(12)]), Value::Bool(false));
        assert_eq!(call(&mut h, "services.caller", &[]), Value::Null);
        h.caller = Some(RemoteCaller { mod_id: "app".into(), player: Some("p1".into()) });
        let c = call(&mut h, "services.caller", &[]);
        assert_eq!((c.get("mod-id"), c.get("player")), (Some(&t("app")), Some(&t("p1"))));
        h.caller = Some(RemoteCaller { mod_id: "app".into(), player: None });
        assert_eq!(call(&mut h, "services.caller", &[]).get("player"), Some(&Value::Null));
        assert_eq!(call(&mut h, "channels.close", &[Value::Int(5)]), Value::Bool(true));
        for bad in [
            t("local"),
            Value::Map(vec![]),
            Value::Map(vec![("a".into(), Value::Null), ("b".into(), Value::Null)]),
            entry("player", Value::Int(1)),
            entry("moon", Value::Null),
        ] {
            assert!(call_host(&mut h, "channels.send", &[Value::Int(5), bad, Value::Bytes(vec![])]).is_err());
        }

        // Every kind of API error crosses as a tagged variant.
        for (e, tag) in [
            (ApiError::NotPermitted("x".into()), "not-permitted"),
            (ApiError::InvalidArgument("x".into()), "invalid-argument"),
            (ApiError::LimitExceeded("x".into()), "limit-exceeded"),
            (ApiError::StaleHandle, "stale-handle"),
            (ApiError::Unsupported, "unsupported"),
            (ApiError::Internal("x".into()), "internal"),
        ] {
            h.err = Some(e);
            let r = call(&mut h, "players.broadcast", &[t("x")]);
            assert!(r.get("err").unwrap().get(tag).is_some(), "{tag}");
        }
    }

    #[test]
    fn bad_arguments_are_errors() {
        let mut h = Fake::default();
        assert!(call_host(&mut h, "nope.nothing", &[]).unwrap_err().contains("unknown host function"));
        assert!(call_host(&mut h, "players.get", &[]).unwrap_err().contains("missing argument 0"));
        assert!(call_host(&mut h, "players.get", &[Value::Int(1)]).unwrap_err().contains("must be text"));
        assert!(call_host(&mut h, "scheduler.run-later", &[t("x")]).unwrap_err().contains("integer"));
        assert!(call_host(&mut h, "scheduler.run-later", &[Value::Int(-1)]).unwrap_err().contains("out of range"));
        assert!(call_host(&mut h, "events.subscribe", &[t("chat"), t("low"), Value::Int(1)])
            .unwrap_err()
            .contains("boolean"));
        assert!(call_host(&mut h, "storage.set", &[t("k"), t("v")]).unwrap_err().contains("bytes"));
    }

    #[test]
    fn host_call_needs_an_active_call() {
        let bytes = Value::List(vec![]).encode();
        assert!(host_call(9999, "host.mod-id", &bytes).unwrap_err().contains("only be used while the host runs"));
        let mut h = Fake::default();
        let guard = enter(9999, &mut h);
        assert_eq!(Value::decode(&host_call(9999, "host.mod-id", &bytes).unwrap()).unwrap(), t("m"));
        assert!(host_call(9999, "host.mod-id", &Value::Int(1).encode()).unwrap_err().contains("must be a list"));
        assert!(host_call(9999, "host.mod-id", &[0xff]).is_err());
        drop(guard);
        assert!(host_call(9999, "host.mod-id", &bytes).is_err());
    }

    #[test]
    fn events_cross_as_wit_variants() {
        let pos = Pos { x: 1, y: 2 };
        let events = vec![
            Event::PlayerJoin { player: "p".into() },
            Event::PlayerQuit { player: "p".into(), was_kick: true },
            Event::Chat { player: "p".into(), text: "t".into(), cancelled: false },
            Event::Command {
                sender: CommandSender::Console,
                name: "n".into(),
                args: vec!["a".into()],
                raw: "/n a".into(),
                cancelled: false,
            },
            Event::Command {
                sender: CommandSender::Player("p".into()),
                name: "n".into(),
                args: vec![],
                raw: "/n".into(),
                cancelled: true,
            },
            Event::Tick { tick: 5, dt: 0.2 },
            Event::WorldLoaded { world_name: "W".into(), world_id: "w".into() },
            Event::WorldSave { phase: SavePhase::Post },
            Event::PlayerPreLogin {
                player: "p".into(),
                alias: "A".into(),
                cancelled: false,
                reason: RejectReason::Kicked,
            },
            Event::BlockPlace { player: "p".into(), pos, item: 1, data_a: 2, data_b: 3, cancelled: false },
            Event::BlockBreak { player: "p".into(), pos, layer: BreakLayer::BackWall, tile: 9, cancelled: false },
            Event::BlockChanged { pos, tile: 4, player: Some("p".into()) },
            Event::BlockChanged { pos, tile: 4, player: None },
            Event::ObjectPlace { player: "p".into(), pos, object: ObjectKind::Workbench, item: 7, cancelled: false },
            Event::Damage { player: None, blockhead: 77, amount: 0.5, attacker: Some("q".into()), cancelled: false },
            Event::Death { player: Some("p".into()), blockhead: 1 },
            Event::PlayerMove { player: "p".into(), from: pos, to: Pos { x: 2, y: 2 } },
            Event::Custom { name: "m:x".into(), source: "m".into(), payload: vec![0xf6], cancelled: false },
        ];
        for ev in &events {
            let v = event_value(ev);
            let Value::Map(m) = &v else { panic!("{v:?}") };
            assert_eq!(m.len(), 1);
            assert_eq!(m[0].0, ev.kind().name(), "{ev:?}");
            // The wire form is canonical CBOR the Java side can read.
            assert_eq!(Value::decode(&v.encode()).unwrap().encode(), v.encode());
        }
        let join = event_value(&events[0]);
        assert_eq!(join.get("player-join").unwrap().get("player"), Some(&t("p")));
    }

    #[test]
    fn outcomes_and_replies_decode() {
        let o = |c: bool, m: Value| record(vec![("cancelled", Value::Bool(c)), ("mutation", m)]);
        assert_eq!(outcome_of(&o(true, Value::Null)).unwrap(), Outcome { cancelled: true, mutation: None });
        assert_eq!(
            outcome_of(&o(false, entry("chat-text", t("x")))).unwrap().mutation,
            Some(Mutation::ChatText("x".into()))
        );
        assert_eq!(
            outcome_of(&o(false, entry("custom-payload", Value::Bytes(vec![1])))).unwrap().mutation,
            Some(Mutation::CustomPayload(vec![1]))
        );
        assert_eq!(
            outcome_of(&o(false, entry("reject-reason", t("banned")))).unwrap().mutation,
            Some(Mutation::RejectReason(RejectReason::Banned))
        );
        assert_eq!(
            outcome_of(&o(false, entry("damage-amount", Value::Float(0.5)))).unwrap().mutation,
            Some(Mutation::DamageAmount(0.5))
        );
        assert_eq!(
            outcome_of(&o(false, entry("damage-amount", Value::Int(2)))).unwrap().mutation,
            Some(Mutation::DamageAmount(2.0))
        );
        assert_eq!(outcome_of(&Value::Map(vec![])).unwrap(), Outcome::default());
        for bad in [
            entry("reject-reason", t("weird")),
            entry("chat-text", Value::Int(1)),
            Value::Map(vec![("a".into(), Value::Null), ("b".into(), Value::Null)]),
            Value::Int(1),
        ] {
            assert!(outcome_of(&o(false, bad)).is_err());
        }

        let c = record(vec![("handled", Value::Bool(true)), ("reply", Value::List(vec![t("a"), t("b")]))]);
        assert_eq!(
            command_outcome_of(&c).unwrap(),
            CommandOutcome { handled: true, reply: vec!["a".into(), "b".into()] }
        );
        assert!(!command_outcome_of(&record(vec![("reply", Value::List(vec![]))])).unwrap().handled);
        assert!(command_outcome_of(&record(vec![("handled", Value::Bool(true))])).is_err());
        assert!(command_outcome_of(&record(vec![("reply", Value::List(vec![Value::Int(1)]))])).is_err());

        assert_eq!(sender_value(&CommandSender::Console), entry("console", Value::Null));
        assert_eq!(source_value(&ChannelSource::Local("m".into())), entry("local", t("m")));
        assert_eq!(source_value(&ChannelSource::Server), entry("server", Value::Null));
        assert_eq!(source_value(&ChannelSource::Player("p".into())), entry("player", t("p")));
    }

    #[test]
    fn glue_results_are_checked() {
        assert_eq!(status(&Value::List(vec![Value::Int(0), t("x")]).encode()).unwrap(), (0, t("x")));
        assert!(status(&Value::List(vec![t("0"), t("x")]).encode()).is_err());
        assert!(status(&Value::Int(1).encode()).is_err());
        assert!(status(&[0xff]).is_err());
        assert_eq!(bytes_of(Value::Bytes(vec![1])).unwrap(), vec![1]);
        assert!(bytes_of(Value::Int(1)).unwrap_err().contains("not a value"));
        assert_eq!(payload_text(&t("m")), "m");
        assert!(payload_text(&Value::Int(1)).contains("Int"));
        assert!(panic_text(&"boom").contains("boom"));
        assert!(panic_text(&String::from("bang")).contains("bang"));
        assert_eq!(panic_text(&5), "");
    }

    fn env<'a>(vars: &'a [(&'static str, OsString)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| vars.iter().find(|v| v.0 == k).map(|v| v.1.clone())
    }

    /// A fake Java home with a `libjvm` in `lib/server`; returns the home and the library path.
    fn fake_home(parent: &Path, name: &str) -> (PathBuf, PathBuf) {
        let home = parent.join(name);
        std::fs::create_dir_all(home.join("lib/server")).unwrap();
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::write(home.join("lib/server").join(LIBJVM), b"x").unwrap();
        std::fs::write(home.join("bin/java"), b"x").unwrap();
        (home.clone(), home.join("lib/server").join(LIBJVM))
    }

    #[test]
    fn libjvm_is_found_in_the_usual_places() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(libjvm_in(tmp.path()), None);
        let old = tmp.path().join("old");
        std::fs::create_dir_all(old.join("jre/lib/amd64/server")).unwrap();
        std::fs::write(old.join("jre/lib/amd64/server").join(LIBJVM), b"x").unwrap();
        assert_eq!(libjvm_in(&old), Some(old.join("jre/lib/amd64/server").join(LIBJVM)));

        let (a, a_lib) = fake_home(tmp.path(), "a");
        let (b, b_lib) = fake_home(tmp.path(), "b");
        let (c, c_lib) = fake_home(tmp.path(), "c");
        let cfg = JvmConfig { java_home: Some(a.clone()), bundled: Some(c.clone()), ..JvmConfig::default() };
        assert_eq!(locate(&cfg).unwrap(), a_lib);
        assert_eq!(JvmRuntime::find_jvm(&cfg).unwrap(), a_lib);
        // The configured home wins over the environment, the environment over the bundled runtime.
        let vars = [("JAVA_HOME", b.clone().into_os_string())];
        assert_eq!(locate_in(&cfg, &env(&vars)).unwrap(), a_lib);
        let cfg = JvmConfig { bundled: Some(c.clone()), ..JvmConfig::default() };
        assert_eq!(locate_in(&cfg, &env(&vars)).unwrap(), b_lib);
        let vars = [("BHMOD_JAVA_HOME", b.clone().into_os_string()), ("JAVA_HOME", OsString::from("/nope"))];
        assert_eq!(locate_in(&cfg, &env(&vars)).unwrap(), b_lib);
        // Empty variables are skipped, then the bundled runtime, then `java` on the PATH.
        let vars = [("JAVA_HOME", OsString::new())];
        assert_eq!(locate_in(&cfg, &env(&vars)).unwrap(), c_lib);
        let path = std::env::join_paths([tmp.path().join("nothing"), b.join("bin")]).unwrap();
        let vars = [("PATH", path.clone())];
        let none = JvmConfig::default();
        assert_eq!(locate_in(&none, &env(&vars)).unwrap(), b_lib);
        // Nothing found: the error says where it looked.
        let vars = [("JAVA_HOME", OsString::from("/nope")), ("PATH", OsString::from("/nope/bin"))];
        let e = locate_in(&none, &env(&vars)).unwrap_err();
        assert!(e.contains("no Java runtime found (tried /nope)"), "{e}");
        assert!(locate_in(&none, &env(&[])).unwrap_err().contains("tried nowhere"));
    }

    #[test]
    fn jni_failures_are_reported_with_the_java_exception() {
        let rt = JvmRuntime::new(JvmConfig::default()).unwrap();
        let e = with_env(&rt.vm, |env| env.call_static_method(glue_class(&rt.vm), "nope", "()V", &[]).map(|_| ()))
            .unwrap_err();
        assert!(e.starts_with("JNI call failed: java.lang.NoSuchMethodError"), "{e}");
        // Nothing pending: the plain JNI error text is used.
        let e = with_env(&rt.vm, |_| Err::<(), _>(jni::errors::Error::NullPtr("test"))).unwrap_err();
        assert!(e.starts_with("JNI call failed:"), "{e}");
    }

    #[test]
    fn the_sdk_jar_is_written_once() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_sdk(tmp.path()).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), SDK_JAR);
        assert_eq!(write_sdk(tmp.path()).unwrap(), p);
        std::fs::write(&p, b"stale").unwrap();
        write_sdk(tmp.path()).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), SDK_JAR);
        let file = tmp.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert!(write_sdk(&file.join("sub")).unwrap_err().contains("cannot create"));
        // The target path is taken by a directory: the rename fails.
        let blocked = tmp.path().join("blocked");
        std::fs::create_dir_all(blocked.join(p.file_name().unwrap())).unwrap();
        std::fs::write(blocked.join(p.file_name().unwrap()).join("x"), b"x").unwrap();
        assert!(write_sdk(&blocked).unwrap_err().contains("cannot write"));
    }
}
