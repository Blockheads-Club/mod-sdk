--!strict
-- decor-items: hands out the mod's items and turns blocks into its variant block.
--   /decor [lamp|plaque] [count]   gives the item (default: a brass lamp)
--   /warm                          turns the block under your feet into a warm brick (mining it gives a wall plaque)

local ITEMS = {
	lamp = "decor-items:brass-lamp",
	plaque = "decor-items:wall-plaque",
}

local mod = {}

function mod.on_enable()
	bh.commands.register({
		name = "decor",
		usage = "/decor [lamp|plaque] [count]",
		description = "Gives you one of the decor items (needs the mod on your side).",
	}, function(sender, _label, args)
		local who = sender.player
		if not who then
			return { handled = true, reply = { "Only players carry items." } }
		end
		local word = args[1] or "lamp"
		local key = ITEMS[word]
		if not key then
			return { handled = true, reply = { `Unknown item "{word}". Try: lamp, plaque` } }
		end
		local count = math.clamp(math.floor(tonumber(args[2]) or 1), 1, 64)
		bh.content.give_item(who, key, count)
		return { handled = true, reply = { `You got {count} x {word}.` } }
	end)

	bh.commands.register({
		name = "warm",
		usage = "/warm",
		description = "Turns the block under your feet into a warm brick.",
	}, function(sender, _label, _args)
		local who = sender.player
		local pos = if who then who:pos() else nil
		if not who or not pos then
			return { handled = true, reply = { "Stand somewhere first." } }
		end
		bh.content.place_block(pos.x, pos.y - 1, "decor-items:warm-brick")
		return { handled = true, reply = { "The ground under you is a warm brick now." } }
	end)
end

return mod
