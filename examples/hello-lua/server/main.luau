--!strict
-- hello-lua: a minimal Blockheads server mod in Luau.
-- It adds a /hello command, greets players who join and counts greetings in mod storage.
-- The `bh` global mirrors the blockheads:api WIT (see docs/luau-api.md).

local function greetings(): number
	local raw = bh.storage.get("greetings")
	return if raw then tonumber(raw) or 0 else 0
end

local mod = {}

function mod.on_enable()
	bh.events.on("player-join", function(ev)
		local count = greetings() + 1
		bh.storage.set("greetings", tostring(count))
		ev.player:send_chat(`Hello, {ev.player:alias()}! You are greeting number {count}.`)
	end)

	bh.events.on("chat", function(ev)
		-- Lowest priority runs first: later handlers may still change their mind.
		if string.find(string.lower(ev.text), "lua is bad", 1, true) then
			ev.cancelled = true
		end
	end, { priority = "lowest" })

	bh.commands.register({
		name = "hello",
		aliases = { "hi" },
		description = "Says hello.",
		usage = "/hello",
	}, function(sender, label, args)
		local who = if sender.player then sender.player:alias() else "console"
		bh.players.broadcast(`{who} says hello to everyone!`)
		return { handled = true, reply = { `Hello, {who}!` } }
	end)

	bh.log.info("hello-lua enabled")
end

function mod.on_disable()
	bh.log.info("hello-lua disabled")
end

return mod
