//! Cosmetics on the game's blockheads (Android): the server's `COSMETICS` say who wears what
//! ([`crate::loader::Loader::content`]), these Objective-C hooks make the blockheads show the mods' textures. The
//! same hooks and the same rules as the PC port (`bhpc-run/src/cosmetics.rs`); what a blockhead shows is decided by
//! `bhmod_content`.
//!
//! - `-[Blockhead updateClothingCubes]` (the game's own, for this client's blockheads): runs the original, then
//!   replaces `hatTexture`, `shirtBodyTexture`, `shirtArmTexture`, `pantsTexture`, `shoesTexture`. While it runs,
//! - `-[Blockhead clothingItemAtIndex:]` gives an empty slot a plain vanilla item when a cosmetic needs a shape to
//!   draw on (the item type then travels in the blockhead's creation data, so other players see the shape too).
//! - `-[Blockhead updateSkin]`: replaces `headHairTexture`.
//! - `-[Blockhead setupFromNetCreationData:]` (other players' blockheads): replaces clothing and hair textures.
//! - `-[Blockhead updateAnimation]` (every frame): refreshes a blockhead when the cosmetics changed since it was
//!   last looked at.
//!
//! Textures are made with `-[CPTexture2D initWithData:pixelFormat:pixelsWide:pixelsHigh:contentSize:mipmap:]`, the
//! initializer the game's own loader uses (pixel format 0 is RGBA8888, the loader itself passes 1, RGBA4444), from the
//! mod's decoded PNG.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use bhmod_content::{Gpu, Look, Part, CLOTHING};
use bhmod_core::types::LogLevel;

use super::objc::{self, Id, Imp, Sel};
use super::{log, with_loader};

struct Originals {
    clothing_item: Imp,
    update_cubes: Imp,
    update_skin: Imp,
    setup_net: Imp,
    update_animation: Imp,
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
    /// Vanilla `InventoryItem`s given to empty slots, by item type.
    items: HashMap<u16, usize>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// True while `-[Blockhead updateClothingCubes]` runs.
    static IN_UPDATE: Cell<bool> = const { Cell::new(false) };
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

fn store(bh: Id, ivar: &str, tex: Id) {
    if tex.is_null() {
        return;
    }
    let slot = objc::ivar_ptr(bh, "Blockhead", ivar);
    if !slot.is_null() {
        objc::store_strong(slot, tex);
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
        let data = objc::send1(data, "initWithLength:", g.rgba.len() as *const _);
        let ptr = objc::send0(data, "mutableBytes") as *mut u8;
        if ptr.is_null() {
            objc::send0(data, "release");
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(g.rgba.as_ptr(), ptr, g.rgba.len());
        let tex = objc::send0(objc::class("CPTexture2D"), "alloc");
        // pixelFormat 0 (RGBA8888; the game's own loader passes 1, RGBA4444), contentSize as two floats (softfp: the same words as two u32), no mipmaps.
        let init = objc::msg::<unsafe extern "C" fn(Id, Sel, *const u8, i32, u32, u32, u32, u32, i8) -> Id>();
        let tex = init(
            tex,
            objc::sel("initWithData:pixelFormat:pixelsWide:pixelsHigh:contentSize:mipmap:"),
            ptr,
            0,
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

/// A vanilla `InventoryItem` of `item_type` for an empty slot.
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Clothing,
    Hair,
    Both,
}

/// Writes the cosmetics' textures into a blockhead.
fn apply_to(bh: Id, which: Which) {
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

extern "C" fn hook_clothing_item(this: Id, cmd: Sel, idx: i32) -> Id {
    let orig = ORIG.get().expect("hooks").clothing_item;
    let real = unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel, i32) -> Id>(orig)(this, cmd, idx) };
    if !real.is_null() || !IN_UPDATE.with(Cell::get) || !(0..4).contains(&idx) {
        return real;
    }
    let player = player_of(this);
    let wear = with_loader(|l| l.content().resolve(&player, CLOTHING[idx as usize], None).and_then(|r| r.wear)).flatten();
    match wear {
        Some(t) => item(t),
        None => real,
    }
}

extern "C" fn hook_update_cubes(this: Id, cmd: Sel) {
    let orig = ORIG.get().expect("hooks").update_cubes;
    let on = active();
    if on {
        IN_UPDATE.with(|f| f.set(true));
    }
    unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel)>(orig)(this, cmd) };
    if on {
        IN_UPDATE.with(|f| f.set(false));
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

extern "C" fn hook_update_animation(this: Id, cmd: Sel) {
    if active() {
        let now = serial();
        let seen = STATE.with(|s| s.borrow().seen.get(&(this as usize)).copied());
        if seen != Some(now) {
            STATE.with(|s| s.borrow_mut().seen.insert(this as usize, now));
            if is_net(this) {
                apply_to(this, Which::Both);
            } else {
                objc::send0(this, "updateClothingCubes");
                objc::send0(this, "updateSkin");
            }
        }
    }
    let orig = ORIG.get().expect("hooks").update_animation;
    unsafe { std::mem::transmute::<Imp, unsafe extern "C" fn(Id, Sel)>(orig)(this, cmd) };
}

/// Forgets which blockheads were looked at (a world closed: the addresses may be reused).
pub fn reset() {
    STATE.with(|s| s.borrow_mut().seen.clear());
}

/// Installs the hooks. False when a method is missing (cosmetics stay off).
pub fn install() -> bool {
    let clothing_item = objc::swizzle("Blockhead", "clothingItemAtIndex:", hook_clothing_item as Imp);
    let update_cubes = objc::swizzle("Blockhead", "updateClothingCubes", hook_update_cubes as Imp);
    let update_skin = objc::swizzle("Blockhead", "updateSkin", hook_update_skin as Imp);
    let setup_net = objc::swizzle("Blockhead", "setupFromNetCreationData:", hook_setup_net as Imp);
    let update_animation = objc::swizzle("Blockhead", "updateAnimation", hook_update_animation as Imp);
    let ok = [clothing_item, update_cubes, update_skin, setup_net, update_animation].iter().all(|p| !p.is_null());
    let _ = ORIG.set(Originals { clothing_item, update_cubes, update_skin, setup_net, update_animation });
    ok
}
