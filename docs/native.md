# Native mods (M10)

A native mod hooks the game itself, behind the red warning. It is a normal package (`mod.toml` with
`permissions.native = true`, a client entry point of any runtime, and the library) plus an armv7 shared library:

```
client/native/armeabi-v7a/lib<name>.so      required for the game (32-bit ARM only; other ABIs are ignored)
```

The library exports two functions (header: [`crates/bhmod-native/include/bhmod.h`](../crates/bhmod-native/include/bhmod.h)):

```c
int  bhmod_native_init(const bhmod_api_t *api);   /* required; install hooks, return 0 */
void bhmod_native_shutdown(void);                 /* optional; runs when the mod is turned off */
```

There is nothing to link against: everything the host offers is in the `bhmod_api_t` table. Build for ARM mode with the NDK
(`-marm -shared -fPIC`, `-nostdlib` if you can). The PC runner's guest provides bionic's libc, libm and liblog only.
A complete example (C): [`sdk/native/hello-c`](../sdk/native/hello-c), built and packed with
`scripts/build-native-example.sh`.

## Who may load it

Native code runs with the game's full rights, so the loaders are strict, on both clients:

1. **The package says so.** `permissions.native = true` (the index CI labels the entry `needs-security-review`: a
   maintainer reads the code before it is listed; see the [mods repo](https://github.com/Blockheads-Club/mods)).
2. **It is on the public index with this exact file.** Android: the player installs it from the *Mod list* (the only
   native install path: links and zips are refused); the installed file's `sha256` is stored. PC: the join dialog and
   the launcher check the index entry's `sha256`.
3. **The player confirmed the red warning** (Android: "Danger - native code" and the button "I understand, install";
   PC: both native boxes of the install dialog / the join prompt's danger zone) and did not turn on "Never allow native
   mods" (PC setting).
4. **A server can never push native code.** A server may list a native mod; a client takes it only when the file with
   the same `sha256` is installed and confirmed already (Android `Options::native_trusted`), otherwise the join stops
   with "native mods come from the public mod list". The bytes that run are the installed ones, never the downloaded
   ones.

A replaced package file is not loaded: the confirmation is tied to the `sha256` it was given for.

## The table (`bhmod_api_t`, ABI 1)

Every function takes the table as its first argument (the host knows which mod calls). Fields are only ever appended;
check `api->abi` and `api->size`.

| Member | |
| --- | --- |
| `mod_id`, `data_dir` | The mod's id and a private writable folder. |
| `log(api, level, msg)` | To logcat (Android) or the runner's log (PC), tagged with the mod id. |
| `hook_objc(api, cls, selector, class_method, replacement, &original)` | Swizzle `-[cls selector]` (`+[...]` with `class_method`). `original` receives the previous IMP: call it as `ret (*)(id, SEL, ...)`. |
| `hook_symbol(api, module, symbol, replacement, &original)` | Inline-hook an **exported** function of `module` (a file name such as `libApplication.so`; `NULL` = any module). |
| `hook_address(api, module, offset, replacement, &original)` | Inline-hook ARM code at `offset` bytes into `module` (a virtual address of the library file, as a disassembler shows it). |
| `hook_import(api, module, symbol, replacement, &original)` | Redirect the PLT/GOT slots that import `symbol` in `module` (`NULL` = every loaded module). `original` is what the import resolved to. |
| `unhook(api, handle)` | Undo one hook. A mod's hooks are undone when it is turned off. |
| `find_symbol`, `module_base` | `dlsym` and the load address. |
| `last_error(api)` | A sentence about the last failed call. |

`hook_*` return a handle (> 0) or a negative `BHMOD_E_*` (`ARG`, `NOT_FOUND`, `THUMB`, `RELOC`, `EXISTS`, `MEMORY`,
`HANDLE`). A failed hook does not stop the mod; `bhmod_native_init` decides what to do.

### What the hooks do, and what they cannot

- **Objective-C swizzle** (`method_setImplementation` of the game's runtime): data only, no code is patched. Works on
  ARM and Thumb methods. Only methods that are dispatched through the runtime at the time of the call: `-[EvolutionViewController
  drawFrame]` is not (the game keeps its IMP), `-[World update:...]` is.
- **Inline hook**: the first two instructions (8 bytes) of an **ARM-mode** function are replaced with
  `ldr pc, [pc, #-4]; .word replacement`. The two instructions move into a *trampoline* (`original`) that runs them and
  jumps back to `function + 8`. The relocator ([`arm.rs`](../crates/bhmod-native/src/arm.rs)) handles `b`/`bl` (any
  condition), `ldr`/`ldrb`/`vldr` from `pc` (the literal is copied), `add`/`sub`/`mov Rd, pc...` (corrected), and copies
  everything that does not name `pc`. It refuses, with a reason: Thumb targets (odd address; every exported function
  and ObjC IMP of the game is ARM), a function that ends in its first instruction, `pc` as destination or base in other
  forms, the unconditional (`0b1111`) space. One inline hook per function (`BHMOD_E_EXISTS`).
  Install hooks in `bhmod_native_init`, before the game uses the code: patching is not atomic against a thread that
  executes the first instructions.
- **Import hook**: writes the GOT slot, so only the named module calls the replacement; the library's own definition and
  other modules keep the original. Slots are found from the module's dynamic section in memory.
- Replacements run on whatever game thread calls the hooked code: keep them short and thread-safe, never throw. Floats
  travel in core registers (the game is soft-float ARM): declare them as 32-bit integers when you only pass them on, and
  forward every argument you do not understand, stack words included (the example does for `drawShaderQuad`).

## Where it runs

| | Android (the Patcher's loader) | PC (`bhpc-run`) |
| --- | --- | --- |
| Library loaded by | `dlopen` in the game process | the runner's linker into the **guest** (imports bind to the host's bionic) |
| Engine host | `android/native.rs`: `mprotect` + `cacheflush`, an RWX arena, `dl_iterate_phdr`, the process's Objective-C runtime | `bhpc-run/src/native.rs`: guest page patch + JIT invalidation, a guest arena, the linker's modules, the game's own runtime called in the guest |
| API functions | C functions of the loader | host thunks in the guest |
| Hook handlers | the mod's functions | the mod's functions (guest code, no host code in between) |

The engine ([`bhmod-native`](../crates/bhmod-native)) is the same crate on both: hooks are installed by *name* and behave
the same. It is tested on the host with captured instruction bytes (`tests/arm.rs`), a fake process (`tests/engine.rs`),
the header against the Rust layout (`tests/header.rs`, compiled for armv7), and on PC with real mod libraries run by the
JIT (`bhpc-run/src/tests_native.rs`, fixtures in `fixtures/native`) and the real game (`scripts/e2e-native.sh`).

### Known limits

- Under the Android emulator's ARM translation the game's code pages show as `r--p`; restoring that protection after a
  code patch crashes the translator, so code pages stay RWX there (`patch` in `android/native.rs`). Verified on the
  emulator (API 30). On a real phone the pages are executable, the protection is restored, and the open question is
  SELinux on runtime code patching: **needs a device check** (fallback: static trampolines added by the Patcher).
- The game is armeabi-v7a only; hooks need the 1.7.6 build's symbols (exports are stable across 1.7.4 to 1.7.6; offsets
  are not).

## Weapon skins

`weapon` cosmetics ([content.md](content.md)) are the first user of the primitive. The game draws a held tool or item
as a sprite inside its blockhead drawing function (`libApplication.so` + `0xc00780..0xc01a98` in 1.7.6): it binds the
shared `Items` atlas (`__wrap_glBindTexture`), asks the **exported** `texCoordsForItemType(ItemType)` for the item's
rectangle in the atlas (a `Vector4` returned through `r0`) and draws a quad. There is no per-blockhead item texture
(and no function `renderSpriteItem`: the sprite code is inline in that function), and the atlas is shared with every
inventory icon. The PC runner therefore

1. remembers which blockhead is drawing, with before hooks on the three draw entry points (`draw:...`,
   `drawTransparentInventoryItem:...` which draws the item in the hand while it is used, `drawForButton...`);
2. inline-hooks `texCoordsForItemType` (through the same engine, `Natives::hook_internal`). When the caller (`lr`) is
   inside that function and the blockhead's player wears a weapon cosmetic whose `base` is the item, it binds the mod's
   texture with the game's own `glBindTexture` and answers with the mod texture's rectangle; every other call tail-calls
   the trampoline.

The swap lasts for the held item's quad only: the host closes the blockhead's draw window after the call and binds the
game's own texture back, so UI sprites drawn through the same function (the dig-target indicator, HUD icons) are never
skinned. Pictures to start from: [`examples/weapon-skins`](../examples/weapon-skins) (32x32, transparent, drawn diagonally).

The hook exists only while a loaded mod declares a weapon cosmetic and only on the build whose function prologue
matches. Dev aid: `BHPC_DEV_COSMETICS=weapon=<key>` with `BHPC_DEV_WEAPON_ITEM=<item type>` shows the cosmetic on that
item whatever its `base` (single-player testing). Android is not wired yet (the loader has the primitive; the cosmetics
code of the other branch owns the blockhead hooks).
