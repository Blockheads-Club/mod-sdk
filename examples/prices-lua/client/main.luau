-- Price check, client half: /price <item> asks the server and shows the answer as a toast.
local mod = {}

local function show(item, ok, result)
    if ok then
        if result == nil then
            bh.ui.toast(item .. " is not for sale.")
        else
            bh.ui.toast(item .. " costs " .. result .. " coins.")
        end
    elseif result.kind == "timeout" then
        bh.ui.toast("The server did not answer.")
    else
        bh.ui.toast("Price check failed: " .. result.kind)
    end
end

function mod.on_enable()
    bh.commands.register({ name = "price", usage = "/price <item>" }, function(sender, label, args)
        local item = args[1]
        if item == nil then
            return { handled = false }
        end
        -- Returns at once; the callback runs when the server has answered (or after 3 s).
        bh.services.call_remote("server", "prices", "^1", "price", { item }, function(ok, result)
            show(item, ok, result)
        end, 3000)
        return { handled = true }
    end)
end

return mod
