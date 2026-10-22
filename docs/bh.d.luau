-- Type declarations of the `bh` global for Luau mods (blockheads:api@0.1.0). Hand-written in M1 from the WIT in
-- ../wit/blockheads-api; see luau-api.md for the mapping rules. Use with luau-lsp ("definitions" setting).

export type Role = "player" | "moderator" | "admin" | "owner"
export type Priority = "lowest" | "low" | "normal" | "high" | "highest" | "monitor"
export type EventKind = "player-join" | "player-quit" | "chat" | "command" | "tick" | "world-loaded" | "world-save"
export type Level = "trace" | "debug" | "info" | "warn" | "error"

declare class Player
	function id(self): string
	function alias(self): string
	function is_online(self): boolean
	function role(self): Role
	function send_chat(self, text: string): ()
	function kick(self, reason: string): ()
end

export type CommandSender = { player: Player?, console: boolean? }

export type Event = {
	kind: EventKind,
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
		online: () -> { Player },
		broadcast: (text: string) -> (),
	},
	events: {
		on: (kind: EventKind, handler: (ev: Event) -> (), opts: { priority: Priority?, ignore_cancelled: boolean? }?) -> number,
		off: (id: number) -> boolean,
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
}
