//! Items of content mods on Android: what the game shows for an item type above the vanilla ones, and that it accepts
//! such an item in an inventory. The same hooks as the PC port (`bhpc-run/src/items.rs`; see there for why):
//!
//! - `NSStringFromItemType`, duplicated into twelve compile units of the 1.7.6 build ([`NAME_FUNCS`]): each copy is
//!   replaced by a function that answers an `NSString` for a mod item and runs the original for everything else;
//! - the exported `itemTypeIsValidFillItem`, false for every id above the vanilla ones, answers yes for mod items.
//!
//! The hooks are inline hooks of the loader's engine ([`super::native::with_engine`]), installed from the frame hook
//! as soon as a loaded mod declares an item and the build's function prologues match, removed when the content goes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, RwLock};

use bhmod_core::types::LogLevel;

use super::objc::{self, Id};
use super::{log, native};

/// The engine owner of these hooks (mods count from 0, the weapon hook is another feature).
const OWNER: u32 = u32::MAX - 2;
const MODULE: &str = "libApplication.so";
/// `NSStringFromItemType` in each of the twelve compile units that have a copy (offsets in the 1.7.6 build).
pub const NAME_FUNCS: [u32; 12] = [
    0x4d_b268, 0x4e_430c, 0x5d_faa8, 0x63_e970, 0x66_a300, 0x69_ff94, 0x71_913c, 0x72_c10c, 0x74_a4a8, 0xa8_64d0,
    0xb7_6660, 0xce_c8e4,
];
/// The first two instructions of each copy (`sub sp, sp, #24; ldr r1, [pc, #0xffc]`).
pub const NAME_PROLOGUE: [u32; 2] = [0xe24d_d018, 0xe59f_1ffc];
const VALID_FILL: &str = "_Z23itemTypeIsValidFillItem8ItemType";

static NAMES: LazyLock<RwLock<HashMap<u16, String>>> = LazyLock::new(|| RwLock::new(HashMap::new()));
/// Retained `NSString`s made for the names (by item type).
static STRINGS: Mutex<Option<HashMap<u16, usize>>> = Mutex::new(None);
static ORIGS: [AtomicUsize; 12] = [const { AtomicUsize::new(0) }; 12];
static VALID_ORIG: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static GAVE_UP: AtomicBool = AtomicBool::new(false);
static SERIAL: AtomicU64 = AtomicU64::new(u64::MAX);

fn name_of(item: u32) -> Option<String> {
    let id = u16::try_from(item).ok()?;
    NAMES.read().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
}

fn is_mod_item(item: u32) -> bool {
    u16::try_from(item).is_ok_and(|id| NAMES.read().unwrap_or_else(|e| e.into_inner()).contains_key(&id))
}

fn item_name(i: usize, item: u32) -> Id {
    if let Some(text) = name_of(item) {
        let id = item as u16;
        let known = STRINGS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&id).copied());
        let obj = known.unwrap_or_else(|| {
            let s = objc::send0(objc::nsstring(&text), "retain") as usize;
            STRINGS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id, s);
            s
        });
        return obj as Id;
    }
    let orig = ORIGS[i].load(Ordering::Relaxed);
    // SAFETY: the trampoline of the original `NSString *(ItemType)`.
    unsafe { std::mem::transmute::<usize, extern "C" fn(u32) -> Id>(orig)(item) }
}

macro_rules! name_fns {
    ($($f:ident = $i:expr),* $(,)?) => {
        $(extern "C" fn $f(item: u32) -> Id { item_name($i, item) })*
        const NAME_REPLACEMENTS: [extern "C" fn(u32) -> Id; 12] = [$($f),*];
    };
}
name_fns!(n0 = 0, n1 = 1, n2 = 2, n3 = 3, n4 = 4, n5 = 5, n6 = 6, n7 = 7, n8 = 8, n9 = 9, n10 = 10, n11 = 11);

extern "C" fn item_valid(item: u32) -> u32 {
    if is_mod_item(item) {
        return 1;
    }
    let orig = VALID_ORIG.load(Ordering::Relaxed);
    // SAFETY: the trampoline of the original `bool (ItemType)`.
    unsafe { std::mem::transmute::<usize, extern "C" fn(u32) -> u32>(orig)(item) }
}

/// Once per frame: takes the names of the loaded mods' items and installs or removes the hooks.
pub fn sync() {
    let Some(shared) = super::cosmetics::SHARED.get() else { return };
    let (serial, names) = {
        let c = shared.lock().unwrap_or_else(|e| e.into_inner());
        if SERIAL.load(Ordering::Relaxed) == c.serial() {
            return;
        }
        let names: HashMap<u16, String> = c
            .items()
            .all()
            .filter_map(|i| Some((u16::try_from(i.id).ok()?, c.item_name(i.id, &language())?)))
            .collect();
        (c.serial(), names)
    };
    SERIAL.store(serial, Ordering::Relaxed);
    let any = !names.is_empty();
    if any {
        log(LogLevel::Info, &format!("items: {} item(s) loaded", names.len()));
    }
    *NAMES.write().unwrap_or_else(|e| e.into_inner()) = names;
    *STRINGS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if any && !INSTALLED.load(Ordering::Relaxed) && !GAVE_UP.load(Ordering::Relaxed) {
        install();
    } else if !any && INSTALLED.load(Ordering::Relaxed) {
        remove();
    }
}

/// The language the player reads, from the device's locale (`en` when unknown).
fn language() -> String {
    let loc = objc::send0(objc::class("NSLocale"), "currentLocale");
    let code = objc::string(objc::send1(loc, "objectForKey:", objc::nsstring("kCFLocaleLanguageCodeKey") as *const _));
    if code.is_empty() {
        "en".to_string()
    } else {
        code
    }
}

fn give_up(why: &str) {
    log(LogLevel::Warn, &format!("items: mod items are off: {why}"));
    GAVE_UP.store(true, Ordering::Relaxed);
}

fn install() {
    let ok = native::with_engine(|engine, host| {
        let Some(base) = host.modules().into_iter().find(|m| m.name.ends_with(MODULE)).map(|m| m.base) else {
            return Err("libApplication.so is not loaded".to_string());
        };
        for f in NAME_FUNCS {
            let words: Option<Vec<u32>> = host
                .read(base + f, 8)
                .map(|b| b.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect());
            if words.as_deref() != Some(&NAME_PROLOGUE[..]) {
                return Err("this is not the game build they know".to_string());
            }
        }
        for (i, f) in NAME_FUNCS.iter().enumerate() {
            match engine.hook_address(host, OWNER, MODULE, *f, NAME_REPLACEMENTS[i] as usize as u32) {
                Ok(h) => ORIGS[i].store(h.original as usize, Ordering::Relaxed),
                Err(e) => {
                    engine.unhook_owner(host, OWNER);
                    return Err(e.message);
                }
            }
        }
        match engine.hook_symbol(
            host,
            OWNER,
            Some(MODULE),
            VALID_FILL,
            item_valid as extern "C" fn(u32) -> u32 as usize as u32,
        ) {
            Ok(h) => VALID_ORIG.store(h.original as usize, Ordering::Relaxed),
            Err(e) => {
                engine.unhook_owner(host, OWNER);
                return Err(e.message);
            }
        }
        Ok(())
    });
    match ok {
        Ok(()) => {
            INSTALLED.store(true, Ordering::Relaxed);
            log(LogLevel::Info, "items: hooks installed (names, item validity)");
        }
        Err(e) => give_up(&e),
    }
}

fn remove() {
    native::with_engine(|engine, host| engine.unhook_owner(host, OWNER));
    INSTALLED.store(false, Ordering::Relaxed);
    log(LogLevel::Info, "items: hooks removed");
}
