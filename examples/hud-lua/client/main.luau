-- HUD demo: widgets in two slots, a notice, a dialog and what happens when it is answered.
local mod = {}

local coins = 1250

local function show_coins()
    bh.ui.set_widget("coins", { slot = "top-right", kind = "counter", icon = "coin", text = tostring(coins) })
end

function mod.on_enable()
    show_coins()
    bh.ui.set_widget("where", { slot = "top-left", text = "X 1042 Y 88" })
    bh.ui.notice("HUD demo is on.", "success")

    -- The position text follows the world step.
    bh.events.on("tick", function(ev)
        if ev.tick % 5 == 0 then
            bh.ui.set_widget("where", { slot = "top-left", text = "tick " .. ev.tick })
        end
    end)

    -- Answers to dialogs of this mod arrive as "<mod-id>:ui-action".
    bh.events.on_custom("hud-lua:ui-action", function(ev)
        local answer = ev.payload
        if answer.button == 1 then
            local n = tonumber(answer.text) or 0
            coins = coins - n
            show_coins()
            bh.ui.toast("You sent " .. n .. " coins.")
            -- A dialog can also be closed from the mod's side.
            local done = bh.ui.show_dialog({ title = "Done", buttons = { { label = "OK" } } })
            bh.ui.close_dialog(done)
        else
            bh.ui.toast("Nothing was sent.")
            bh.ui.remove_widget("where")
        end
    end)

    bh.ui.show_dialog({
        title = "Send coins",
        text = "How many coins do you want to send?",
        field = { placeholder = "50", initial = "10", max_length = 6 },
        buttons = { { label = "Cancel" }, { label = "Send", kind = "primary" } },
    })
end

return mod
