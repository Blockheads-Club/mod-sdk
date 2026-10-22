--!strict
-- perms: named permission nodes. `/perm grant <player> <node>`, `/perm revoke <player> <node>`, `/perm list <player>`.
-- A grant of `a.*` covers every node that starts with `a.`. Admins and the owner have every node.

local function nodes(id: string): { [string]: boolean }
	local raw = bh.storage.get("p:" .. id)
	return if raw then bh.value.decode(raw) else {}
end

local function has(p: Player, node: string): boolean
	local role = p:role()
	if role == "admin" or role == "owner" then
		return true
	end
	local set = nodes(p:id())
	if set[node] then
		return true
	end
	for granted in pairs(set) do
		if string.sub(granted, -2) == ".*" and string.sub(node, 1, #granted - 1) == string.sub(granted, 1, -2) then
			return true
		end
	end
	return false
end

local mod = {}

function mod.on_enable()
	bh.services.provide("permissions", "1.0.0", { has = has })

	bh.commands.register({
		name = "perm",
		usage = "/perm <grant|revoke|list> <player> [node]",
		description = "Manages permission nodes.",
		min_role = "admin",
	}, function(sender, label, args)
		local action, who, node = args[1], args[2] and bh.players.find_by_alias(args[2]), args[3]
		if not who then
			return { handled = false, reply = { "Name an online player." } }
		end
		local set = nodes(who:id())
		if action == "list" then
			local list = {}
			for n in pairs(set) do
				table.insert(list, n)
			end
			table.sort(list)
			return who:alias() .. ": " .. (if #list > 0 then table.concat(list, ", ") else "(none)")
		elseif (action == "grant" or action == "revoke") and node then
			set[node] = if action == "grant" then true else nil
			bh.storage.set("p:" .. who:id(), bh.value.encode(set))
			return (if action == "grant" then "Granted " else "Revoked ") .. node .. " for " .. who:alias() .. "."
		end
		return { handled = false }
	end)
end

return mod
