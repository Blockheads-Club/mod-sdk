-- Type declarations of the `bh` global for Luau mods (blockheads:api@0.2.0). Hand-written from the WIT in
-- ../wit/blockheads-api; see luau-api.md for the mapping rules. Use with luau-lsp ("definitions" setting).

export type Role = "player" | "moderator" | "admin" | "owner"
export type Priority = "lowest" | "low" | "normal" | "high" | "highest" | "monitor"
export type EventKind =
	"player-join"
	| "player-quit"
	| "chat"
	| "command"
	| "tick"
	| "world-loaded"
	| "world-save"
	-- API 0.2
	| "player-pre-login"
	| "block-place"
	| "block-break"
	| "block-changed"
	| "object-place"
	| "damage"
	| "death"
	| "player-move"
export type Level = "trace" | "debug" | "info" | "warn" | "error"
export type RejectReason =
	"client-too-old"
	| "server-full"
	| "blacklisted"
	| "not-whitelisted"
	| "incorrect-password"
	| "kicked"
	| "banned"
	| "invalid-username"
	| "still-loading"
export type Pos = { x: number, y: number }

declare class Player
	function id(self): string
	function alias(self): string
	function is_online(self): boolean
	function role(self): Role
	function send_chat(self, text: string): ()
	function kick(self, reason: string): ()
	function pos(self): Pos?
	function value_handle(self): number
end

export type CommandSender = { player: Player?, console: boolean? }

export type Event = {
	kind: EventKind | "custom",
	player: Player?,
	was_kick: boolean?,
	text: string?,
	cancelled: boolean?,
	sender: CommandSender?,
	name: string?,
	args: { string }?,
	raw: string?,
	tick: number?,
	dt: number?,
	world_name: string?,
	world_id: string?,
	phase: ("pre" | "post")?,
	-- API 0.2
	player_id: string?,
	alias: string?,
	reason: RejectReason?,
	pos: Pos?,
	item: number?,
	data_a: number?,
	data_b: number?,
	layer: ("block" | "back-wall")?,
	tile: number?,
	object: ("workbench" | "interaction-object")?,
	blockhead: number?,
	amount: number?,
	attacker: Player?,
	from: Pos?,
	to: Pos?,
	source: string?,
	payload: any,
}

export type CommandSpec = {
	name: string,
	aliases: { string }?,
	description: string?,
	usage: string?,
	min_role: Role?,
	console: boolean?,
}

export type CommandOutcome = { handled: boolean?, reply: (string | { string })? }
export type SubscribeOpts = { priority: Priority?, ignore_cancelled: boolean? }
export type ChannelOrigin = { mod: string?, server: boolean?, player: Player? }
export type ContentKind = "block" | "item" | "cosmetic"
export type CosmeticInfo = { key: string, mod_id: string, slot: string, id: number? }

export type ChannelTarget = "local" | "server" | "all-players" | Player

declare bh: {
	host: {
		api_version: () -> string,
		mod_id: () -> string,
		mod_version: () -> string,
		current_side: () -> "server" | "client",
		host_name: () -> string,
	},
	log: {
		log: (level: Level, message: string) -> (),
		trace: (message: string) -> (),
		debug: (message: string) -> (),
		info: (message: string) -> (),
		warn: (message: string) -> (),
		error: (message: string) -> (),
	},
	players: {
		get: (id: string) -> Player?,
		find_by_alias: (alias: string) -> Player?,
		from_value_handle: (handle: number) -> Player?,
		online: () -> { Player },
		broadcast: (text: string) -> (),
	},
	content: {
		id_of: (kind: ContentKind, key: string) -> number?,
		key_of: (kind: ContentKind, id: number) -> string?,
		cosmetics: () -> { CosmeticInfo },
		equipped: (player: Player) -> { [string]: string },
		equip: (player: Player, key: string) -> (),
		unequip: (player: Player, slot: string) -> (),
	},
	events: {
		on: (kind: EventKind, handler: (ev: Event) -> (), opts: SubscribeOpts?) -> number,
		off: (id: number) -> boolean,
		on_custom: (name: string, handler: (ev: Event) -> (), opts: SubscribeOpts?) -> number,
		fire_custom: (name: string, payload: any) -> boolean,
	},
	commands: {
		register: (
			spec: CommandSpec,
			handler: (sender: CommandSender, label: string, args: { string }) -> (CommandOutcome | string)?
		) -> number,
		unregister: (id: number) -> boolean,
	},
	scheduler: {
		run_later: (delay: number, task: () -> ()) -> number,
		run_every: (delay: number, period: number, task: () -> ()) -> number,
		run_next_poll: (task: () -> ()) -> number,
		cancel: (id: number) -> boolean,
	},
	storage: {
		get: (key: string) -> string?,
		set: (key: string, value: string) -> (),
		delete: (key: string) -> boolean,
		keys: (prefix: string?) -> { string },
	},
	services: {
		provide: (name: string, version: string, methods: { [string]: (...any) -> any }) -> (),
		call: (name: string, version_req: string, method: string, ...any) -> any,
		get: (name: string, version_req: string) -> { [string]: (...any) -> any },
	},
	channels: {
		open: (name: string, handler: ((origin: ChannelOrigin, data: string) -> ())?) -> number,
		send: (id: number, to: ChannelTarget, data: string) -> (),
		close: (id: number) -> boolean,
	},
	value: {
		encode: (value: any) -> string,
		decode: (bytes: string) -> any,
	},
}
