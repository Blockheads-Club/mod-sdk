//! The Apportable Objective-C runtime (`libSystem.so`), reached with `dlsym`, and the game's exported ivar offsets
//! (`OBJC_IVAR_$_Class.ivar` in `libApplication.so`). 32-bit ARM, soft-float ABI: every argument used here is a
//! pointer or an integer, so `objc_msgSend` is called through plain `extern "C"` function types.

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::OnceLock;

/// An object.
pub type Id = *mut c_void;
/// A selector.
pub type Sel = *const c_void;
/// A method implementation.
pub type Imp = *const c_void;

extern "C" {
    fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: i32 = 2;
const RTLD_NOLOAD: i32 = 4;

struct Rt {
    get_class: unsafe extern "C" fn(*const c_char) -> Id,
    sel_register: unsafe extern "C" fn(*const c_char) -> Sel,
    instance_method: unsafe extern "C" fn(Id, Sel) -> *mut c_void,
    set_imp: unsafe extern "C" fn(*mut c_void, Imp) -> Imp,
    store_strong: unsafe extern "C" fn(*mut Id, Id),
    msg_send: *const c_void,
    app: usize,
}
// SAFETY: plain function pointers and a library handle, never mutated after initialisation.
unsafe impl Send for Rt {}
unsafe impl Sync for Rt {}

static RT: OnceLock<Option<Rt>> = OnceLock::new();

unsafe fn sym(h: *mut c_void, name: &str) -> *mut c_void {
    let c = CString::new(name).expect("no NUL");
    dlsym(h, c.as_ptr())
}

fn rt() -> Option<&'static Rt> {
    RT.get_or_init(|| unsafe {
        let sys = dlopen(c"libSystem.so".as_ptr(), RTLD_NOW | RTLD_NOLOAD);
        let app = dlopen(c"libApplication.so".as_ptr(), RTLD_NOW | RTLD_NOLOAD);
        if sys.is_null() || app.is_null() {
            return None;
        }
        let f = |n| sym(sys, n);
        let (a, b, c, d, e, g) = (
            f("objc_getClass"),
            f("sel_registerName"),
            f("class_getInstanceMethod"),
            f("method_setImplementation"),
            f("objc_msgSend"),
            f("objc_storeStrong"),
        );
        if [a, b, c, d, e, g].iter().any(|p| p.is_null()) {
            return None;
        }
        Some(Rt {
            get_class: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_char) -> Id>(a),
            sel_register: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_char) -> Sel>(b),
            instance_method: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(Id, Sel) -> *mut c_void>(c),
            set_imp: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void, Imp) -> Imp>(d),
            store_strong: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut Id, Id)>(g),
            msg_send: e,
            app: app as usize,
        })
    })
    .as_ref()
}

/// True when the runtime and the game library are loaded.
pub fn available() -> bool {
    rt().is_some()
}

/// A class by name (null when missing).
pub fn class(name: &str) -> Id {
    let Some(r) = rt() else { return std::ptr::null_mut() };
    let c = CString::new(name).expect("no NUL");
    unsafe { (r.get_class)(c.as_ptr()) }
}

/// A selector by name.
pub fn sel(name: &str) -> Sel {
    let r = rt().expect("runtime");
    let c = CString::new(name).expect("no NUL");
    unsafe { (r.sel_register)(c.as_ptr()) }
}

/// Replaces `-[class selector]` and returns the original implementation (null when the method is missing).
pub fn swizzle(class_name: &str, selector: &str, imp: Imp) -> Imp {
    let Some(r) = rt() else { return std::ptr::null() };
    let cls = class(class_name);
    if cls.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let m = (r.instance_method)(cls, sel(selector));
        if m.is_null() {
            return std::ptr::null();
        }
        (r.set_imp)(m, imp)
    }
}

/// `objc_msgSend` as a function of the given type.
///
/// # Safety
/// `F` must be an `extern "C"` function pointer type matching the method's arguments.
pub unsafe fn msg<F: Copy>() -> F {
    let r = rt().expect("runtime");
    debug_assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*const c_void>());
    std::mem::transmute_copy(&r.msg_send)
}

/// `[obj sel]` returning an object or integer-sized value.
pub fn send0(obj: Id, selector: &str) -> Id {
    unsafe { msg::<unsafe extern "C" fn(Id, Sel) -> Id>()(obj, sel(selector)) }
}

/// `[obj sel:a]`.
pub fn send1(obj: Id, selector: &str, a: *const c_void) -> Id {
    unsafe { msg::<unsafe extern "C" fn(Id, Sel, *const c_void) -> Id>()(obj, sel(selector), a) }
}

/// `[obj sel:a b:b]`.
pub fn send2(obj: Id, selector: &str, a: *const c_void, b: *const c_void) -> Id {
    unsafe { msg::<unsafe extern "C" fn(Id, Sel, *const c_void, *const c_void) -> Id>()(obj, sel(selector), a, b) }
}

/// The address of `obj.ivar` (`OBJC_IVAR_$_Class.ivar`), null when the symbol is missing or `obj` is nil.
pub fn ivar_ptr(obj: Id, class_name: &str, ivar: &str) -> *mut Id {
    let Some(r) = rt() else { return std::ptr::null_mut() };
    if obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let off = sym(r.app as *mut c_void, &format!("OBJC_IVAR_$_{class_name}.{ivar}")) as *const u32;
        if off.is_null() {
            return std::ptr::null_mut();
        }
        (obj as *mut u8).add(*off as usize) as *mut Id
    }
}

/// `obj.ivar` as an object (nil when missing).
pub fn ivar(obj: Id, class_name: &str, ivar: &str) -> Id {
    let p = ivar_ptr(obj, class_name, ivar);
    if p.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { *p }
}

/// An autoreleased `NSData` with these bytes.
pub fn nsdata(bytes: &[u8]) -> Id {
    unsafe {
        msg::<unsafe extern "C" fn(Id, Sel, *const u8, u32) -> Id>()(
            class("NSData"),
            sel("dataWithBytes:length:"),
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    }
}

/// The bytes of an `NSData` (copied).
pub fn data_bytes(data: Id) -> Vec<u8> {
    if data.is_null() {
        return Vec::new();
    }
    let len = send0(data, "length") as usize;
    let ptr = send0(data, "bytes") as *const u8;
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, len).to_vec() }
}

/// An autoreleased `NSString`.
pub fn nsstring(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).expect("no NUL");
    send1(class("NSString"), "stringWithUTF8String:", c.as_ptr() as *const c_void)
}

/// The text of an `NSString` (empty for nil).
pub fn string(s: Id) -> String {
    if s.is_null() {
        return String::new();
    }
    let p = send0(s, "UTF8String") as *const c_char;
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

/// An autoreleased `NSNumber` holding an int.
pub fn nsnumber(v: i32) -> Id {
    unsafe { msg::<unsafe extern "C" fn(Id, Sel, i32) -> Id>()(class("NSNumber"), sel("numberWithInt:"), v) }
}

/// `objc_storeStrong(slot, obj)`: retains `obj`, releases what `*slot` held and stores it (an ARC assignment to an
/// ivar). Does nothing without the runtime.
pub fn store_strong(slot: *mut Id, obj: Id) {
    if let Some(r) = rt() {
        unsafe { (r.store_strong)(slot, obj) };
    }
}
