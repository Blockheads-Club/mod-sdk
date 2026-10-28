-- Type declarations of the `bh` global for Luau mods (blockheads:api@0.3.4). Hand-written from the WIT in
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
	-- API 0.3.4 (server; false elsewhere)
	function sleeping(self): boolean
	function alive(self): boolean
end

-- API 0.3.4
export type VoiceMode = "proximity" | "global" | "channel" | "broadcast"
export type VoiceActivation = "ptt" | "vad" | "toggle"
export type VoiceState = "none" | "ready" | "talking" | "self-muted" | "deafened" | "admin-muted"
export type VoiceRoute = { listener: Player, modes: { VoiceMode }?, gain: number?, pan: number?, muffle: number? }
export type VoiceIcons = { speaking: boolean?, no_voice: boolean?, self_mute: boolean?, deafen: boolean?, admin_mute: boolean? }
export type UiButton = { icon: string, label: string?, x: number?, y: number?, size: number?, opacity: number? }

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

export type UiWidget = {
	slot: ("top-left" | "top-right")?,
	kind: ("text" | "counter" | "panel")?,
	icon: string?,
	title: string?,
	text: string?,
}
export type UiDialogButton = { label: string, kind: ("primary" | "secondary" | "danger")? }
export type UiDialog = {
	title: string,
	text: string?,
	field: { placeholder: string?, initial: string?, max_length: number? }?,
	buttons: { UiDialogButton },
}
-- The payload of the custom event "<mod-id>:ui-action" (button is 4294967295 when the dialog went away).
export type UiAnswer = { dialog: number, button: number, text: string? }

declare bh: {
	host: {
		api_version: () -> string,
		mod_id: () -> string,
		mod_version: () -> string,
		current_side: () -> "server" | "client",
		host_name: () -> string,
		platform: () -> string, -- 0.3.4
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
		local_player: () -> Player?, -- 0.3.4, clients
		has_mod: (player: Player, mod_id: string) -> boolean, -- 0.3.4, server
	},
	content: {
		id_of: (kind: ContentKind, key: string) -> number?,
		key_of: (kind: ContentKind, id: number) -> string?,
		cosmetics: () -> { CosmeticInfo },
		equipped: (player: Player) -> { [string]: string },
		equip: (player: Player, key: string) -> (),
		unequip: (player: Player, slot: string) -> (),
		-- API 0.3.2
		give_item: (player: Player, key: string, count: number?) -> (),
		place_block: (x: number, y: number, key: string) -> (),
		block_at: (x: number, y: number) -> string?,
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
	-- API 0.3.3: the mod's own [settings]; changes fire the custom event "<mod-id>:settings-changed" { keys = { string } }.
	settings: {
		get: (key: string) -> any,
		all: () -> { [string]: any },
		set: (key: string, value: any) -> (),
	},
	services: {
		provide: (name: string, version: string, methods: { [string]: (...any) -> any }) -> (),
		call: (name: string, version_req: string, method: string, ...any) -> any,
		get: (name: string, version_req: string) -> { [string]: (...any) -> any },
		-- API 0.3.1: calls a service on the other side; the callback gets (true, value) or (false, { kind, message }) later.
		call_remote: (to: "server" | Player, name: string, version_req: string, method: string, args: { any }?, callback: (ok: boolean, result: any) -> (), timeout_ms: number?) -> number,
		cancel_remote: (id: number) -> boolean,
		caller: () -> { mod: string, player: Player? }?,
	},
	channels: {
		open: (name: string, handler: ((origin: ChannelOrigin, data: string) -> ())?) -> number,
		send: (id: number, to: ChannelTarget, data: string) -> (),
		close: (id: number) -> boolean,
	},
	ui: {
		set_widget: (id: string, widget: UiWidget) -> (),
		remove_widget: (id: string) -> boolean,
		toast: (text: string) -> (),
		notice: (text: string, level: ("info" | "warning" | "success" | "error")?) -> (),
		show_dialog: (dialog: UiDialog) -> number,
		close_dialog: (id: number) -> boolean,
		-- API 0.3.4
		set_overhead: (player: Player, icon: "speaking" | "no-voice" | "self-mute" | "deafen" | "admin-mute") -> (),
		clear_overhead: (player: Player) -> boolean,
		set_button: (id: string, button: UiButton) -> (),
		remove_button: (id: string) -> boolean,
		open_settings: (mod_id: string) -> (),
		has_touch: () -> boolean,
	},
	-- API 0.3.4, permission `voice` (see voice.md); events voice:talk-start|talk-stop|state|mic|allowed
	voice: {
		state: (player: Player) -> VoiceState,
		icons: () -> VoiceIcons,
		set_routes: (speaker: Player, routes: { VoiceRoute }) -> (),
		mute: (player: Player, until_ms: number?) -> (),
		unmute: (player: Player) -> boolean,
		set_speaker_gain: (player: Player, percent: number) -> (),
		set_caps: (caps: { max_bitrate: number?, max_speakers: number?, max_frames_per_sec: number? }) -> (),
		-- Only `udp_port` matters (nil = the host's default, game port + 1); `mode` is ignored (frames always use UDP).
		set_transport: (t: { mode: ("auto" | "inband" | "udp")?, udp_port: number? }) -> (),
		set_allowed: (player: Player, modes: { VoiceMode }, activations: { VoiceActivation }) -> (),
		set_icons: (icons: VoiceIcons) -> (),
		set_broadcast_overrides_deafen: (on: boolean) -> (),
		set_default_mode: (mode: VoiceMode) -> (),
		set_activation: (a: VoiceActivation) -> (),
		talk: (mode: VoiceMode?, pressed: boolean) -> (),
		set_mic: (on: boolean) -> (),
		set_self_mute: (on: boolean) -> (),
		set_deafen: (on: boolean) -> (),
		set_vad_threshold: (db: number) -> (),
		set_master: (percent: number) -> (),
		master: () -> number,
		set_player_volume: (player: Player, percent: number) -> (),
		player_volume: (player: Player) -> number,
		mute_player: (player: Player, muted: boolean) -> (),
		player_muted: (player: Player) -> boolean,
		allowed: () -> { modes: { VoiceMode }, activations: { VoiceActivation }, max_bitrate: number },
		self_muted: () -> boolean,
		deafened: () -> boolean,
		mic: () -> "off" | "on" | "denied" | "unavailable",
	},
	terrain: {
		solid_count: (x1: number, y1: number, x2: number, y2: number) -> number,
		world_width: () -> number?,
	},
	input: {
		bind: (action: string, default_key: string) -> (),
		unbind: (action: string) -> boolean,
		key_of: (action: string) -> string?,
	},
	value: {
		encode: (value: any) -> string,
		decode: (bytes: string) -> any,
	},
}
