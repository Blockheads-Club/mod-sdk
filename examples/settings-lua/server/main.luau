--!strict
-- Greets players who join. Everything it says comes from its settings: change them with
-- `/mods set settings-lua greeting Hello {name}`, in mods.toml or in the host app.

local mod = {}

local function is_staff(p: any): boolean
	for _, role in bh.settings.get("staff") do
		if role == p:role() then
			return true
		end
	end
	return false
end

local function greet(p: any)
	local s = bh.settings.all()
	if not s.enabled then
		return
	end
	local text = string.gsub(s.greeting, "{name}", p:alias())
	if s.style == "loud" then
		text = string.upper(text)
	end
	for _ = 1, s["repeat"] do
		p:send_chat(text)
	end
	local staff = is_staff(p)
	for i, rule in s.rules do
		if staff or not rule.staff_only then
			p:send_chat(i .. ". " .. rule.text)
		end
	end
end

function mod.on_enable()
	bh.events.on("player-join", function(ev)
		greet(ev.player)
	end)
	bh.events.on_custom("settings-lua:settings-changed", function(ev)
		bh.log.info("settings changed: " .. table.concat(ev.payload.keys, ", "))
	end)
	-- /greetme shows the greeting again; /quiet switches greetings off (the mod changes its own setting).
	bh.commands.register({ name = "greetme", usage = "/greetme", console = false }, function(sender)
		greet(sender.player)
		return nil
	end)
	bh.commands.register({ name = "quiet", usage = "/quiet", min_role = "admin" }, function()
		bh.settings.set("enabled", false)
		return "Greetings are off. /mods set settings-lua enabled true turns them on."
	end)
end

return mod
