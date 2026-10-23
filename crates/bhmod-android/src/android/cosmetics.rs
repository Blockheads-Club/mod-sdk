//! Cosmetics on the game's blockheads (Android): the server's `COSMETICS` say who wears what
//! ([`crate::loader::Loader::content`]), these Objective-C hooks make the blockheads show the mods' textures. The
//! same hooks and the same rules as the PC port (`bhpc-run/src/cosmetics.rs`); what a blockhead shows is decided by
//! `bhmod_content`.
//!
//! - `-[Blockhead updateClothingCubes]` (the game's own, for this client's blockheads): lends plain vanilla items to
//!   empty slots a cosmetic needs a shape for (auto-wear), runs the original, then
//!   replaces `hatTexture`, `shirtBodyTexture`, `shirtArmTexture`, `pantsTexture`, `shoesTexture`. While it runs,
//! - `-[Blockhead updateSkin]`: replaces `headHairTexture`.
//! - `-[Blockhead setupFromNetCreationData:]` (other players' blockheads): replaces clothing and hair textures.
//! - `-[Blockhead update:accurateDT:isSimulation:]` (every frame): refreshes a blockhead the first time it is seen and
//!   whenever the cosmetics changed.
//!
//! Textures are made with `-[CPTexture2D initWithData:pixelFormat:pixelsWide:pixelsHigh:contentSize:mipmap:]`, the
//! initializer the game's own loader uses (pixel format 1, premultiplied RGBA bytes; 0 makes it return nil), from the mod's decoded PNG.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use bhmod_content::{Gpu, Look, Part, CLOTHING};
use bhmod_core::types::LogLevel;

use super::objc::{self, Id, Imp, Sel};
use super::{log, with_loader};

struct Originals {
    update_cubes: Imp,
    update_skin: Imp,
    setup_net: Imp,
    update_frame: Imp,
}
// SAFETY: code addresses, written once when the hooks are installed.
unsafe impl Send for Originals {}
unsafe impl Sync for Originals {}
static ORIG: OnceLock<Originals> = OnceLock::new();

#[derive(Default)]
struct State {
    /// The last serial of the cosmetics each blockhead (by address) was refreshed for.
    seen: HashMap<usize, u64>,
    /// `CPTexture2D` objects made from the mods' textures (0 = could not be made).
    textures: HashMap<(u64, u8), usize>,
    /// The game's own texture of every ivar a cosmetic replaced (retained), by blockhead and ivar.
    originals: HashMap<(usize, &'static str), usize>,
    /// Vanilla `InventoryItem`s lent to empty slots, by item type.
    items: HashMap<u16, usize>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn part_key(p: Part) -> u8 {
    match p {
        Part::Main => 0,
        Part::Arm => 1,
    }
}

fn active() -> bool {
    with_loader(|l| l.content().is_active()).unwrap_or(false)
}

fn serial() -> u64 {
    with_loader(|l| l.content().serial()).unwrap_or(0)
}

/// The owner of a blockhead (`clientID`).
fn player_of(bh: Id) -> String {
    objc::string(objc::ivar(bh, "Blockhead", "clientID"))
}

/// The `ItemType` worn in each clothing slot ([`CLOTHING`] order), `None` for empty.
fn worn(bh: Id) -> [Option<u16>; 4] {
    ["shoesItemType", "pantsItemType", "shirtItemType", "hatItemType"].map(|n| {
        let p = objc::ivar_ptr(bh, "Blockhead", n) as *const u16;
        let v = if p.is_null() { 0 } else { unsafe { *p } };
        (v != 0).then_some(v)
    })
}

fn is_net(bh: Id) -> bool {
    let p = objc::ivar_ptr(bh, "DynamicObject", "isNet") as *const u8;
    !p.is_null() && unsafe { *p } != 0
}

/// Replaces the texture in `ivar` with `tex`, keeping the game's own texture to put back later.
fn store(bh: Id, ivar: &'static str, tex: Id) {
    let slot = objc::ivar_ptr(bh, "Blockhead", ivar);
    if tex.is_null() || slot.is_null() {
        return;
    }
    let first = STATE.with(|s| !s.borrow().originals.contains_key(&(bh as usize, ivar)));
    if first {
        let cur = unsafe { *slot };
        if !cur.is_null() {
            objc::send0(cur, "retain");
            STATE.with(|s| s.borrow_mut().originals.insert((bh as usize, ivar), cur as usize));
        }
    }
    objc::store_strong(slot, tex);
}

const CLOTHING_IVARS: [&str; 5] = ["hatTexture", "shirtBodyTexture", "shirtArmTexture", "pantsTexture", "shoesTexture"];

impl Which {
    /// The texture ivars this part covers.
    fn ivars(self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self != Which::Hair {
            v.extend(CLOTHING_IVARS);
        }
        if self != Which::Clothing {
            v.push("headHairTexture");
        }
        v
    }
}

/// Puts the game's own textures back into a blockhead (`keep`: leave the ivars alone and only drop what was saved,
/// because the game just rebuilt them).
fn restore(bh: Id, which: Which, keep: bool) {
    let mine = which.ivars();
    let saved: Vec<(&'static str, usize)> = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let keys: Vec<_> = s.originals.keys().filter(|k| k.0 == bh as usize && mine.contains(&k.1)).copied().collect();
        keys.into_iter().filter_map(|k| s.originals.remove(&k).map(|v| (k.1, v))).collect()
    });
    for (ivar, obj) in saved {
        let slot = objc::ivar_ptr(bh, "Blockhead", ivar);
        if !keep && !slot.is_null() {
            objc::store_strong(slot, obj as Id);
        }
        objc::send0(obj as Id, "release");
    }
}

/// A `CPTexture2D` for a cosmetic's texture (made once; null when it cannot be loaded).
fn texture(id: u64, part: Part) -> Id {
    let key = (id, part_key(part));
    if let Some(t) = STATE.with(|s| s.borrow().textures.get(&key).copied()) {
        return t as Id;
    }
    let gpu: Option<Arc<Gpu>> = with_loader(|l| match l.content_mut().texture(id, part) {
        Ok(g) => g,
        Err(why) => {
            log(LogLevel::Warn, &format!("cosmetics: {why}"));
            None
        }
    })
    .flatten();
    let tex = gpu.map_or(std::ptr::null_mut(), |g| make_texture(&g));
    STATE.with(|s| s.borrow_mut().textures.insert(key, tex as usize));
    tex
}

fn make_texture(g: &Gpu) -> Id {
    unsafe {
        let data = objc::send0(objc::class("NSMutableData"), "alloc");
        let data = objc::send1(data, "initWithLength:", g.pixels.len() as *const _);
        let ptr = objc::send0(data, "mutableBytes") as *mut u8;
        if ptr.is_null() {
            objc::send0(data, "release");
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(g.pixels.as_ptr(), ptr, g.pixels.len());
        let tex = objc::send0(objc::class("CPTexture2D"), "alloc");
        // The game's own pixel format (1: premultiplied RGBA bytes), contentSize as two floats (softfp: the same words as two u32), no mipmaps.
        let init = objc::msg::<unsafe extern "C" fn(Id, Sel, *const u8, i32, u32, u32, u32, u32, i8) -> Id>();
        let tex = init(
            tex,
            objc::sel("initWithData:pixelFormat:pixelsWide:pixelsHigh:contentSize:mipmap:"),
            ptr,
            bhmod_content::PIXEL_FORMAT as i32,
            g.wide,
            g.high,
            (g.content_wide as f32).to_bits(),
            (g.content_high as f32).to_bits(),
            0,
        );
        objc::send0(data, "release");
        tex
    }
}

/// A vanilla `InventoryItem` of `item_type` (made once) to put in an empty clothing slot.
fn item(item_type: u16) -> Id {
    if let Some(i) = STATE.with(|s| s.borrow().items.get(&item_type).copied()) {
        return i as Id;
    }
    let alloc = objc::send0(objc::class("InventoryItem"), "alloc");
    let it = unsafe {
        objc::msg::<unsafe extern "C" fn(Id, Sel, i32, u16, u16, Id, Id) -> Id>()(
            alloc,
            objc::sel("initWithType:dataA:dataB:subItems:dynamicObjectSaveDict:"),
            i32::from(item_type),
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    STATE.with(|s| s.borrow_mut().items.insert(item_type, it as usize));
    it
}

/// Lends plain vanilla items to the empty clothing slots a cosmetic needs a shape for (auto-wear). The game reads
/// `inventoryItems[0]` (the clothing group item) `.subItems[slot][0]`; the item is put into that slot's array for the
/// duration of `updateClothingCubes`, and the item type ivar is set so the cube is built. Returns the arrays to
/// empty again afterwards.
fn lend_items(bh: Id, player: &str) -> Vec<Id> {
    let inv = objc::ivar(bh, "Blockhead", "inventoryItems");
    if inv.is_null() {
        return Vec::new();
    }
    let at = |o: Id, i: usize| objc::send1(o, "objectAtIndex:", i as *const std::ffi::c_void);
    let count = |o: Id| objc::send0(o, "count") as usize;
    let group_slot = at(inv, 0);
    if group_slot.is_null() || count(group_slot) == 0 {
        return Vec::new();
    }
    let subs = objc::send0(at(group_slot, 0), "subItems");
    let n = count(subs);
    let mut lent = Vec::new();
    for (i, slot) in CLOTHING.iter().enumerate().take(n) {
        let Some(wear) = with_loader(|l| l.content().resolve(player, slot, None).and_then(|r| r.wear)).flatten() else {
            continue;
        };
        let arr = at(subs, i);
        if arr.is_null() || count(arr) != 0 {
            continue;
        }
        objc::send1(arr, "addObject:", item(wear) as *const std::ffi::c_void);
        let names = ["shoesItemType", "pantsItemType", "shirtItemType", "hatItemType"];
        let p = objc::ivar_ptr(bh, "Blockhead", names[i]) as *mut u16;
        if !p.is_null() {
            unsafe { *p = wear };
        }
        lent.push(arr);
    }
    lent
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Clothing,
    Hair,
    Both,
}

/// Writes the cosmetics' textures into a blockhead.
fn apply_to(bh: Id, which: Which) {
    // The game just rebuilt (some of) these textures: what was saved of them is stale.
    restore(bh, which, true);
    let player = player_of(bh);
    let w = worn(bh);
    let Some((look, serial)) = with_loader(|l| (l.content().look(&player, w), l.content().serial())) else { return };
    STATE.with(|s| s.borrow_mut().seen.insert(bh as usize, serial));
    let look: Look = look;
    if look.is_empty() {
        return;
    }
    if which != Which::Hair {
        for (i, slot) in CLOTHING.iter().enumerate() {
            let Some((id, _)) = look.clothing[i] else { continue };
            let (main, arm) = match *slot {
                "shirt" => ("shirtBodyTexture", Some("shirtArmTexture")),
                "hat" => ("hatTexture", None),
                "pants" => ("pantsTexture", None),
                _ => ("shoesTexture", None),
            };
            store(bh, main, texture(id, Part::Main));
            if let Some(arm) = arm {
                store(bh, arm, texture(id, Part::Arm));
            }
        }
    }
    if which != Which::Clothing {
        if let Some(id) = look.hair {
            store(bh, "headHairTexture", texture(id, Part::Main));
        }
    }
}

extern "C" fn hook_update_cubes(this: Id, cmd: Sel) {
    let orig = ORIG.get().expect("hooks").update_cubes;
    let on = active();
    let lent = if on { lend_items(this, &player_of(this)) } else { Vec::new() };
    unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel)>(orig)(this, cmd) };
    for arr in lent {
        objc::send0(arr, "removeAllObjects");
    }
    if on {
        apply_to(this, Which::Clothing);
    }
}

extern "C" fn hook_update_skin(this: Id, cmd: Sel) {
    let orig = ORIG.get().expect("hooks").update_skin;
    unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel)>(orig)(this, cmd) };
    if active() {
        apply_to(this, Which::Hair);
    }
}

extern "C" fn hook_setup_net(this: Id, cmd: Sel, data: Id) {
    let orig = ORIG.get().expect("hooks").setup_net;
    unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel, Id)>(orig)(this, cmd, data) };
    if active() {
        apply_to(this, Which::Both);
    }
}

extern "C" fn hook_update_frame(this: Id, cmd: Sel, dt: u32, accurate: u32, simulation: i8) {
    if active() {
        let now = serial();
        let seen = STATE.with(|s| s.borrow().seen.get(&(this as usize)).copied());
        if seen != Some(now) {
            STATE.with(|s| s.borrow_mut().seen.insert(this as usize, now));
            let player = player_of(this);
            let w = worn(this);
            let look = with_loader(|l| l.content().look(&player, w)).unwrap_or_default();
            let had = STATE.with(|s| s.borrow().originals.keys().any(|k| k.0 == this as usize));
            if !look.is_empty() || had {
                if is_net(this) {
                    // Other players' blockheads are never rebuilt by the game: put the old textures back, then
                    // override.
                    restore(this, Which::Both, false);
                    apply_to(this, Which::Both);
                } else {
                    objc::send0(this, "updateClothingCubes");
                    objc::send0(this, "updateSkin");
                }
            }
        }
    }
    let orig = ORIG.get().expect("hooks").update_frame;
    unsafe {
        std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel, u32, u32, i8)>(orig)(
            this, cmd, dt, accurate, simulation,
        )
    };
}

/// Forgets which blockheads were looked at (a world closed: the addresses may be reused).
pub fn reset() {
    STATE.with(|s| s.borrow_mut().seen.clear());
}

/// Installs the hooks. False when a method is missing (cosmetics stay off).
pub fn install() -> bool {
    let update_cubes = objc::swizzle("Blockhead", "updateClothingCubes", hook_update_cubes as Imp);
    let update_skin = objc::swizzle("Blockhead", "updateSkin", hook_update_skin as Imp);
    let setup_net = objc::swizzle("Blockhead", "setupFromNetCreationData:", hook_setup_net as Imp);
    let update_frame = objc::swizzle("Blockhead", "update:accurateDT:isSimulation:", hook_update_frame as Imp);
    let ok = [update_cubes, update_skin, setup_net, update_frame].iter().all(|p| !p.is_null());
    let _ = ORIG.set(Originals { update_cubes, update_skin, setup_net, update_frame });
    ok
}
