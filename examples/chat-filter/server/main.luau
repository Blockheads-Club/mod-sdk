--!strict
-- chat-filter: cancels chat lines with blocked links, masks rude words, and logs what it did.
-- Shows event priorities, cancellation, `ignore_cancelled`, text rewriting and a moderator-only command.

local words = require("./words")

local function enabled(): boolean
	return bh.storage.get("enabled") ~= "off"
end

local function mask(text: string): string
	for _, w in words.masked do
		text = string.gsub(text, w, string.rep("*", #w))
	end
	return text
end

local mod = {}

function mod.on_enable()
	-- Low: decide early whether the message is dropped.
	bh.events.on("chat", function(ev)
		if not enabled() then
			return
		end
		local lower = string.lower(ev.text)
		for _, b in words.blocked do
			if string.find(lower, b, 1, true) then
				ev.cancelled = true
				ev.player:send_chat("Your message was blocked: links to " .. b .. " are not allowed.")
				return
			end
		end
	end, { priority = "low" })

	-- Normal: rewrite what is left. `ignore_cancelled` skips messages a lower priority already dropped.
	bh.events.on("chat", function(ev)
		if enabled() then
			ev.text = mask(ev.text)
		end
	end, { ignore_cancelled = true })

	-- Monitor: sees the final state and only logs.
	bh.events.on("chat", function(ev)
		if ev.cancelled then
			bh.log.info(`blocked a message from {ev.player:alias()}`)
		end
	end, { priority = "monitor" })

	bh.commands.register({
		name = "chatfilter",
		description = "Turns the chat filter on or off.",
		usage = "/chatfilter on|off",
		min_role = "moderator",
	}, function(sender, label, args)
		local mode = args[1]
		if mode ~= "on" and mode ~= "off" then
			return { handled = false, reply = { `The filter is {if enabled() then "on" else "off"}.` } }
		end
		bh.storage.set("enabled", mode)
		return { handled = true, reply = { `Chat filter {mode}.` } }
	end)
end

return mod
