--!strict
-- scheduler-lua: delayed and repeating tasks. One tick is one world step (0.2 s on the server), so 5 ticks = 1 s.

local TICKS_PER_SECOND = 5
local MAX_REMIND_SECONDS = 86400

local mod = {}
local countdown: number? = nil

function mod.on_enable()
	-- A repeating tip every 5 minutes.
	bh.scheduler.run_every(300 * TICKS_PER_SECOND, 300 * TICKS_PER_SECOND, function()
		bh.players.broadcast("Tip: type /remind 30 drink water")
	end)

	-- /countdown [seconds]: counts down in chat once per second.
	bh.commands.register({
		name = "countdown",
		usage = "/countdown [seconds]",
		description = "Counts down in chat.",
	}, function(sender, label, args)
		if countdown then
			return { handled = true, reply = { "A countdown is already running." } }
		end
		-- A whole number of seconds: a fraction such as 2.5 would never reach 0 and the countdown would run forever.
		local left = math.floor(math.clamp(tonumber(args[1]) or 3, 1, 10))
		local id: number
		id = bh.scheduler.run_every(0, TICKS_PER_SECOND, function()
			if left == 0 then
				bh.players.broadcast("Go!")
				bh.scheduler.cancel(id)
				countdown = nil
			else
				bh.players.broadcast(tostring(left))
				left -= 1
			end
		end)
		countdown = id
		return { handled = true, reply = {} }
	end)

	-- /remind <seconds> <text>: a one-shot task, answered right away from the next network poll.
	bh.commands.register({
		name = "remind",
		usage = "/remind <seconds> <text>",
		description = "Reminds you later.",
	}, function(sender, label, args)
		local seconds = tonumber(args[1])
		local who = sender.player
		if not seconds or #args < 2 or not who then
			return { handled = false, reply = {} }
		end
		-- Negative or huge delays are not valid ticks (a Luau error counts as a fault), so keep them in range.
		seconds = math.clamp(seconds, 0, MAX_REMIND_SECONDS)
		local text = table.concat(args, " ", 2)
		local ok = pcall(bh.scheduler.run_later, math.floor(seconds * TICKS_PER_SECOND), function()
			if who:is_online() then
				who:send_chat("Reminder: " .. text)
			end
		end)
		if not ok then
			return { handled = true, reply = { "Too many reminders are waiting; try again later." } }
		end
		bh.scheduler.run_next_poll(function()
			if who:is_online() then
				who:send_chat(`OK, I will remind you in {seconds} s.`)
			end
		end)
		return { handled = true, reply = {} }
	end)
end

return mod
