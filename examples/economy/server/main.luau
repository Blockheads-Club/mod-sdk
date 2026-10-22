--!strict
-- economy: per-world coin balances kept in mod storage, offered to other mods as the `economy` service.
-- Fires the custom event `economy:balance-changed` { player, balance } after every change.

local START_BALANCE = 100

local function key(p: Player): string
	return "bal:" .. p:id()
end

local function balance(p: Player): number
	local v = bh.storage.get(key(p))
	return if v then tonumber(v) :: number else START_BALANCE
end

local function set(p: Player, n: number)
	bh.storage.set(key(p), tostring(n))
	bh.events.fire_custom("economy:balance-changed", { player = p, balance = n })
end

local function amount(n: any): number
	if type(n) ~= "number" or n < 0 or n ~= math.floor(n) then
		error("bad amount", 0)
	end
	return n
end

local api = {}

function api.balance(p: Player): number
	return balance(p)
end

function api.deposit(p: Player, n: number): number
	local b = balance(p) + amount(n)
	set(p, b)
	return b
end

function api.withdraw(p: Player, n: number): boolean
	local b = balance(p)
	if b < amount(n) then
		return false
	end
	set(p, b - n)
	return true
end

function api.pay(from: Player, to: Player, n: number): number
	if not api.withdraw(from, n) then
		error("insufficient funds", 0)
	end
	api.deposit(to, n)
	return balance(from)
end

local mod = {}

function mod.on_enable()
	bh.services.provide("economy", "1.0.0", api)

	bh.commands.register({
		name = "balance",
		aliases = { "bal" },
		usage = "/balance [player]",
		description = "Shows coins.",
		console = false,
	}, function(sender, label, args)
		local who = if args[1] then bh.players.find_by_alias(args[1]) else sender.player
		if not who then
			return "No such player online."
		end
		return who:alias() .. " has " .. balance(who) .. " coins."
	end)

	bh.commands.register({
		name = "pay",
		usage = "/pay <player> <amount>",
		description = "Gives coins to another player.",
		console = false,
	}, function(sender, label, args)
		local to = args[1] and bh.players.find_by_alias(args[1])
		local n = tonumber(args[2] or "")
		if not to or not n or n <= 0 or n ~= math.floor(n) then
			return { handled = false, reply = { "Name an online player and a whole amount above 0." } }
		end
		if to == sender.player then
			return "You cannot pay yourself."
		end
		local ok, err = pcall(api.pay, sender.player, to, n)
		if not ok then
			return "Payment failed: " .. tostring(err)
		end
		to:send_chat(sender.player:alias() .. " paid you " .. n .. " coins.")
		return "Paid " .. n .. " coins to " .. to:alias() .. ". You have " .. balance(sender.player) .. " left."
	end)

	bh.commands.register({
		name = "eco-give",
		usage = "/eco-give <player> <amount>",
		description = "Gives coins out of thin air.",
		min_role = "admin",
	}, function(sender, label, args)
		local to = args[1] and bh.players.find_by_alias(args[1])
		local n = tonumber(args[2] or "")
		if not to or not n or n <= 0 then
			return { handled = false }
		end
		return to:alias() .. " now has " .. api.deposit(to, n) .. " coins."
	end)
end

return mod
