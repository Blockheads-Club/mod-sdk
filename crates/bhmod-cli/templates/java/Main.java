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

        api.info("{{id}} enabled");
    }
}
