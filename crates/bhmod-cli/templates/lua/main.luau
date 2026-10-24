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

	-- Offer a service to client mods (needs [provides] ... remote = true in mod.toml, see there):
	-- bh.services.provide("{{id}}", "1.0.0", {
	-- 	hello = function(name)
	-- 		local c = bh.services.caller() -- { mod = "<calling mod>", player = <player> }
	-- 		return "Hello, " .. tostring(name) .. "!"
	-- 	end,
	-- })
	-- Call a service a client mod provides (answer comes later, in the callback):
	-- bh.services.call_remote(player, "their-service", "^1", "method", { "arg" }, function(ok, v) end, 3000)

	bh.log.info("{{id}} enabled")
end

function mod.on_disable() end

return mod
