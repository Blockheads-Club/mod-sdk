# Content: cosmetics (milestone M8, part 1)

A mod declares new content in `[content]` of `mod.toml` by string key, `<mod-id>:<name>`. The server gives every
key a number for its world ([registry in the 0xBC protocol](protocol-0xbc.md#content-registry)), the mod's client half
ships the textures in `assets/`, and the clients draw them. Any mod with `[content]` is **client-required**: players
install it (the join prompt downloads it), or they cannot join.

This part covers **cosmetics**. Blocks and items are declared today (they get ids) but the clients do not draw them
yet; see the resume note `notes/resume/m8-content.md` of the project for the plan.

## Declaring a cosmetic

```toml
[sides]
server = true      # equips the cosmetic (command, shop, event ...)
client = true      # content needs a client half

[content]
cosmetics = [
  { key = "crowns:gold", slot = "hat", base = "linen-cap", texture = "assets/gold-crown.png", name = { en = "Gold crown" } },
  { key = "crowns:ember", slot = "hair", texture = "assets/ember-hair.png" },
  { key = "crowns:suit", slot = "shirt", texture = "assets/suit.png", texture-arm = "assets/suit-arm.png" },
]
```

| Field | Meaning |
| --- | --- |
| `key` | `<mod-id>:<name>`, unique in the mod. |
| `slot` | `hat`, `shirt`, `pants`, `shoes`, `hair`, `weapon`. |
| `texture` | PNG under `assets/`, up to 1024 x 1024. Laid out like the vanilla texture of the item it re-skins. |
| `texture-arm` | `shirt` only: the arms. Without it the vanilla arms stay. |
| `base` | The vanilla item it re-skins ([`bhmod_manifest::content::BASE_ITEMS`](../crates/bhmod-manifest/src/content.rs): `linen-cap`, `santa-hat`, `leather-boots`, ...). It shows while that item is worn or the slot is empty. Without `base` it shows on every vanilla item worn in the slot and on an empty slot. `hair` has no base. A `weapon` needs one. |
| `name` | Display name per language (used by the mod's own UI and commands). |

## What the player sees

- A cosmetic **re-skins the vanilla item in its slot**: the modded clients draw the mod's texture instead of the
  vanilla one, on the vanilla item's shape. If the slot is empty the blockhead puts on a plain vanilla item for it (the
  cosmetic's `base`, else a linen cap / linen shirt / linen pants / leather boots), so a cosmetic works without owning
  anything; with a vanilla item worn and a `base` that does not match, it stays hidden. That item's type goes out in
  the blockhead's creation data like any worn item, so everybody sees the shape (players without the mod loader see
  the plain item).
- **Textures.** Clothing and hat textures are drawn as they are (the clothing shader does not tint them): paint the
  colours you want, on the layout of the vanilla texture (hats 32x16 / santa hat 32x32 head net, clothing 16x16).
  **Hair is a mask**, not a picture: the game draws the player's own hair colour where the red channel is dark and
  the skin where it is light, so paint it black and white (32x16 head net: top x 8..15 y 0..7; front, right, left,
  back faces at x 8, 0, 16, 24, y 8..15). The hair geometry of the chosen style stays; the mask changes the hair on
  the head cube.
- The cosmetic is chosen by the **server mod**: `bh.content.equip(player, "crowns:gold")` (Luau) or
  `content.equip` (WIT, `blockheads:api` 0.3). The server tells every modded client; all of a player's blockheads
  show it. Vanilla clients cannot join a server with content mods (client-required); on a server where the mod is
  optional on the client side they see the plain vanilla look.
- Cosmetics are not saved by the server. A mod keeps who owns/wears what in its own storage and re-equips on
  `player-join`.
- `weapon` is part of the format but the clients do not draw it yet. Evidence (PC, 1.7.6): the game draws a held
  sprite item with `renderSpriteItem(cache, ...)`, which asks the shared `CPCache` for `Items.png` by name; there is
  no per-blockhead item texture. Swapping the cache entry for a mod atlas while a blockhead is drawn (a hook on
  `-[Blockhead draw:...]`) made the whole blockhead disappear, and a held tool is only drawn while it is used, so it
  could not be checked headlessly. A per-player skin needs a different hook point (the sprite renderer itself).

## API (blockheads:api 0.3)

| WIT `content` | Luau | |
| --- | --- | --- |
| `id-of(kind, key) -> option<u32>` | `bh.content.id_of("cosmetic", key)` | The world's id of a declared key. |
| `key-of(kind, id) -> option<string>` | `bh.content.key_of("cosmetic", id)` | The key of an id. |
| `cosmetics() -> list<cosmetic-info>` | `bh.content.cosmetics()` | The cosmetics of all loaded mods (`key`, `mod-id`, `slot`, `id`). |
| `equip(player, key)` | `bh.content.equip(player, key)` | Puts one of **your mod's** cosmetics on a player (replaces the one in its slot). |
| `unequip(player, slot)` | `bh.content.unequip(player, slot)` | Takes the cosmetic of a slot off. |
| `equipped(player) -> list<tuple<slot, key>>` | `bh.content.equipped(player)` | What the player wears. |

A mod can equip only keys it declares itself (`not-permitted` otherwise); a stale player handle gives `stale-handle`.
On a client `equip` and `unequip` return `unsupported`.
