--!strict
-- landclaim: players buy 16x16 tile plots with coins from the economy service. Blocks and objects in a plot can only
-- be placed or removed by its owner (admins, and players with the `landclaim.bypass` node of the optional perms
-- mod, may too). Cancelled edits go through the same path as a vanilla protected tile, so the client is resynced.
--
-- /claim (buys the plot you stand in), /unclaim (gives it up, half the price back), /claiminfo.

local SIZE = 16
local PRICE = 50

local eco = bh.services.get("economy", "^1")

local function plot_key(x: number, y: number): string
	return "claim:" .. math.floor(x / SIZE) .. ":" .. math.floor(y / SIZE)
end

type Claim = { id: string, alias: string }

local function claim_at(x: number, y: number): Claim?
	local raw = bh.storage.get(plot_key(x, y))
	return if raw then bh.value.decode(raw) else nil
end

local function may_bypass(p: Player): boolean
	local role = p:role()
	if role == "admin" or role == "owner" then
		return true
	end
	local ok, allowed = pcall(bh.services.call, "permissions", "^1", "has", p, "landclaim.bypass")
	return ok and allowed == true
end

local function guard(ev)
	local c = claim_at(ev.pos.x, ev.pos.y)
	if c and c.id ~= ev.player:id() and not may_bypass(ev.player) then
		ev.cancelled = true
		ev.player:send_chat("This land belongs to " .. c.alias .. ".")
	end
end

local mod = {}

function mod.on_enable()
	bh.events.on("block-break", guard, { priority = "low" })
	bh.events.on("block-place", guard, { priority = "low" })
	bh.events.on("object-place", guard, { priority = "low" })

	bh.commands.register({ name = "claim", description = "Buys the plot you stand in.", console = false }, function(sender)
		local p = sender.player
		local at = p:pos()
		if not at then
			return "You need to be in the world."
		end
		local c = claim_at(at.x, at.y)
		if c then
			return if c.id == p:id() then "You already own this plot." else "This plot belongs to " .. c.alias .. "."
		end
		if not eco.withdraw(p, PRICE) then
			return "A plot costs " .. PRICE .. " coins; you have " .. eco.balance(p) .. "."
		end
		bh.storage.set(plot_key(at.x, at.y), bh.value.encode({ id = p:id(), alias = p:alias() }))
		return "You bought this plot for " .. PRICE .. " coins."
	end)

	bh.commands.register({ name = "unclaim", description = "Gives up the plot you stand in.", console = false }, function(sender)
		local p = sender.player
		local at = p:pos()
		local c = at and claim_at(at.x, at.y)
		if not at or not c or c.id ~= p:id() then
			return "You do not own this plot."
		end
		bh.storage.delete(plot_key(at.x, at.y))
		eco.deposit(p, PRICE // 2)
		return "Plot released; " .. PRICE // 2 .. " coins refunded."
	end)

	bh.commands.register({ name = "claiminfo", description = "Who owns the plot you stand in.", console = false }, function(sender)
		local at = sender.player:pos()
		local c = at and claim_at(at.x, at.y)
		return if c then "Owned by " .. c.alias .. "." else "Nobody owns this plot."
	end)
end

return mod
