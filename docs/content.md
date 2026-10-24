# Content: cosmetics, items and variant blocks (milestone M8)

A mod declares new content in `[content]` of `mod.toml` by string key, `<mod-id>:<name>`. The server gives every
key a number for its world ([registry in the 0xBC protocol](protocol-0xbc.md#content-registry)), the mod's client half
ships the textures in `assets/`, and the clients draw them. Any mod with `[content]` is **client-required**: players
install it (the join prompt downloads it), or they cannot join.

This page covers **cosmetics** (re-skins of what a player wears), **items** (real new `ItemType`s) and **variant
blocks** (a vanilla block carrying a variant id). Items and blocks are described at the end.

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
- `weapon` re-skins the **held tool or item** (the item in the blockhead's hand while it is used, and the portrait in
  the HUD) when the player wears it and holds the cosmetic's `base`. The PC client draws it with an inline hook on the
  game's sprite code ([native.md](native.md#weapon-skins)); the texture is the whole picture of the item (any size up
  to 1024 x 1024), not a cell of the vanilla atlas. The Android client does not draw weapons yet.

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

## Items (API 0.3.2)

```toml
[content]
items = [
  { key = "decor:lamp", texture = "assets/lamp.png", name = { en = "Brass lamp", tr = "Pirinç lamba" } },
]
```

An item is a **real `ItemType`**: the world gives the key an id from 1107 up (`bh.content.id_of("item", key)`), the id
is stored in inventories, chests and the save like any vanilla item, and stacks like one. What makes it work:

- **Only on sessions where every peer runs the mod.** Mods with `[content]` are client-required, so a vanilla
  client cannot join the server at all, and the server never hands out a mod item id to anybody else.
- **Names.** Both clients ask the mod table for the name wherever the game builds an item's title (inventory, chests,
  crafting, trade, tooltips). `name` is shown in capitals like the game's own names, in the player's language, else
  `en`; an item without `name` shows its key's name part (`decor:brass-lamp` shows `BRASS LAMP`).
- **Icons.** `texture` is a square PNG (up to 256 x 256). The clients copy it into a free cell of the game's item atlas
  (`Items.png`, about 167 cells are free), scaled to the atlas's cell size.
- **Behaviour on the server** is the mod's: it can `bh.content.give_item(player, key, count)` (a command, a shop, a
  reward), and a mod may react to inventory events. A mod item is not placeable, not wearable, has no recipe and does
  nothing when used by itself: the vanilla rules do not know it. A variant block's `drops` is the way to get one by
  mining.

## Variant blocks (API 0.3.2)

```toml
[content]
blocks = [
  { key = "decor:warm-brick", texture = "assets/warm-brick.png", base = "red-bricks", drops = "decor:wall-plaque",
    name = { en = "Warm brick" } },
]
```

A variant block is a **vanilla block of its `base` type that also carries the variant id** in the 16 spare bytes of
its tile (`Tile.padding`, see [`bhmod_content::variant`](../crates/bhmod-content/src/variant.rs)): a magic, the id, the
base `typeIndex`, a layer, 2 bytes of mod data and a checksum over the bytes and the tile's coordinates. Physics,
light, fluids and everything else are the base block's, so a world with variant blocks loads in vanilla (they show as
their base block). The tile bytes travel in saves and in the block packet like every other tile byte.

- **The server owns the record.** It clears it on every change of a tile's block type (mining, placing, the generator),
  and a record is valid only while the tile still has the base type and the coordinates it was written for.
- `bh.content.place_block(x, y, key)` puts the variant at a tile; `bh.content.block_at(x, y)` reads it. Mining a
  variant gives `drops` (an item of the same mod) instead of the base block's item.
- **Clients** draw the variant's `texture` (a square PNG, one cell of the tile atlas, scaled to its cell size) where
  the record is valid.
- **Off by default.** The server writes variant records only when `mods.variant-blocks = true` is set in its
  configuration, until the check that no vanilla client path reads or clears the tile bytes has been done on a device
  (see `notes/mod-research/content-ids.md` section 5.3).

`base` is one of: `wood`, `cobblestone`, `red-bricks`, `limestone-block`, `marble-block`, `sandstone-block`,
`red-marble-block`, `glass`, `gold-block`, `lapis-block`, `basalt-block`, `copper-block`, `tin-block`, `bronze-block`,
`iron-block`, `steel-block`, `black-glass`, `plaster` ([`BASE_TILES`](../crates/bhmod-manifest/src/content.rs)).

Example: [`examples/decor-items`](../examples/decor-items).

### API (blockheads:api 0.3.2)

| WIT `content` | Luau | |
| --- | --- | --- |
| `give-item(who, key, count)` | `bh.content.give_item(player, key, count?)` | Gives a player one of **your mod's** items (1 to 64). |
| `place-block(x, y, key)` | `bh.content.place_block(x, y, key)` | Puts **your mod's** variant block at a loaded tile. |
| `block-at(x, y) -> option<string>` | `bh.content.block_at(x, y)` | The variant block key at a tile. |

Clients get `unsupported` from `give-item` and `place-block`.
