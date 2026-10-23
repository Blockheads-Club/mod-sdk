-- Price check, server half: a service clients may call (mod.toml: remote = true).
local prices = { apple = 3, bread = 5, sword = 120 }

local mod = {}

function mod.on_enable()
    bh.services.provide("prices", "1.0.0", {
        price = function(item)
            if type(item) ~= "string" then error("item must be text", 0) end
            return prices[item]
        end,
        catalog = function()
            local names = {}
            for name in pairs(prices) do table.insert(names, name) end
            table.sort(names)
            return names
        end,
        -- Not listed in remote-methods: only server mods can call this one.
        update = function(item, coins) prices[item] = coins end,
    })
    -- Inside a provider, bh.services.caller() says who is asking over the network.
    bh.events.on("player-join", function(ev)
        bh.log.info(ev.player:alias() .. " can check prices")
    end)
end

return mod
