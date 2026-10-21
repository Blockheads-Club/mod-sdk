--!strict
-- {{id}}: a Blockheads server mod in Luau. See https://github.com/Blockheads-Club/mod-sdk/blob/dev/docs/luau-api.md

local mod = {}

function mod.on_enable()
	bh.events.on("player-join", function(ev)
		ev.player:send_chat(`Welcome, {ev.player:alias()}!`)
	end)

	bh.commands.register({ name = "{{id}}", description = "Says hello.", usage = "/{{id}}" }, function(sender, label, args)
		return { handled = true, reply = { "Hello from {{id}}!" } }
	end)

	bh.log.info("{{id}} enabled")
end

function mod.on_disable() end

return mod
