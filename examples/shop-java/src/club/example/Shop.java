package club.example;

import club.blockheads.api.players.Role;
import club.blockheads.api.services.CallErrorException;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;
import club.blockheads.mod.Replies;
import club.blockheads.mod.Senders;
import club.blockheads.mod.ServiceClient;
import club.blockheads.mod.Spec;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * shop-java: {@code /buy <item>} takes coins through the {@code economy} service of the Lua economy mod. Shows
 * cross-language interop from the JVM: service arguments and results are plain Java values, players travel as host
 * handles.
 */
public final class Shop implements Mod {
    /** Prices in coins. */
    private static final Map<String, Long> ITEMS = new LinkedHashMap<String, Long>();

    static {
        ITEMS.put("apple", 5L);
        ITEMS.put("sword", 30L);
    }

    @Override
    public void onEnable(Api api) {
        ServiceClient economy = api.service("economy", "^1");
        api.command(Spec.command("buy").description("Buys an item with coins.").usage("/buy <apple|sword>")
                .minRole(Role.PLAYER).console(false).build(), (sender, label, args) -> {
                    club.blockheads.api.players.Player player = Senders.player(sender);
                    if (player == null) {
                        return Replies.reply("Players only.");
                    }
                    String item = args.isEmpty() ? null : args.get(0);
                    Long price = item == null ? null : ITEMS.get(item);
                    if (price == null) {
                        return Replies.usage();
                    }
                    try {
                        // economy.withdraw(player, price) -> bool
                        Object paid = economy.call("withdraw", player, price);
                        return Replies.reply(Boolean.TRUE.equals(paid)
                                ? "You bought a " + item + " for " + price + " coins."
                                : "A " + item + " costs " + price + " coins; you do not have enough.");
                    } catch (CallErrorException e) {
                        return Replies.reply("The shop is closed (" + e.getMessage() + ").");
                    }
                });
    }
}
