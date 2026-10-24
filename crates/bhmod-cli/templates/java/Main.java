package {{javapkg}};

import club.blockheads.api.events.PlayerJoinEvent;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;
import club.blockheads.mod.Replies;
import club.blockheads.mod.Senders;

/** {{id}}: greets players who join and adds /hello. */
public final class Main implements Mod {
    @Override
    public void onEnable(Api api) {
        api.onPlayerJoin((PlayerJoinEvent ev) -> ev.player.sendChat("Hello, " + ev.player.alias() + "!"));

        api.command("hello", "Says hello.", (sender, label, args) -> Replies.reply("Hello, " + Senders.name(sender) + "!"));

        // Offer a service to client mods (also needs `[provides] ... remote = true` in mod.toml, see there):
        // api.provide("{{id}}", "1.0.0", (method, args) -> "Hello!");
        // Call a service a client mod provides (the callback runs later):
        // api.callRemote(new RemoteTarget.Player(player), "their-service", "^1", "method", Arrays.asList("arg"), 3000L,
        //         new RemoteCallback() { ... });

        api.info("{{id}} enabled");
    }
}
