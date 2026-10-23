--!strict
-- cosmetics-hats: puts the mod's cosmetics on players. /hat <crown|party|none> and /hair <ember|none>.
-- The server does not save who wears what, so the choice is kept in mod storage and put back when a player joins.

local PREFIX = "cosmetics-hats:"

-- command word -> cosmetic key, per slot
local CHOICES = {
	hat = { crown = PREFIX .. "crown", party = PREFIX .. "party" },
	hair = { ember = PREFIX .. "ember" },
}

local function storage_key(player: Player, slot: string): string
	return `wear:{player:id()}:{slot}`
end

local function names(slot: string): string
	local list = {}
	for name in CHOICES[slot] do
		table.insert(list, name)
	end
	table.sort(list)
	table.insert(list, "none")
	return table.concat(list, ", ")
end

local function wear(player: Player, slot: string, word: string?): string
	local options = CHOICES[slot]
	if word == nil then
		return `Usage: /{slot} <{names(slot)}>`
	elseif word == "none" then
		bh.content.unequip(player, slot)
		bh.storage.delete(storage_key(player, slot))
		return `Your {slot} is back to normal.`
	end
	local key = options[word]
	if not key then
		return `Unknown {slot} "{word}". Try: {names(slot)}`
	end
	bh.content.equip(player, key)
	bh.storage.set(storage_key(player, slot), key)
	return `Your {slot} is now {word}.`
end

local mod = {}

function mod.on_enable()
	for slot in CHOICES do
		bh.commands.register({
			name = slot,
			usage = `/{slot} <{names(slot)}>`,
			description = `Changes your {slot} cosmetic (players with the mod loader see it).`,
		}, function(sender, _label, args)
			local who = sender.player
			if not who then
				return { handled = true, reply = { "Only players wear cosmetics." } }
			end
			return { handled = true, reply = { wear(who, slot, args[1]) } }
		end)
	end

	-- Put back what a player chose in an earlier session.
	bh.events.on("player-join", function(ev)
		for slot in CHOICES do
			local key = bh.storage.get(storage_key(ev.player, slot))
			if key then
				bh.content.equip(ev.player, key)
			end
		end
	end)
end

return mod
