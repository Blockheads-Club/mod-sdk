package fx;

import club.blockheads.api.channels.Source;
import club.blockheads.api.channels.Target;
import club.blockheads.api.events.CommandSender;
import club.blockheads.api.players.Player;
import club.blockheads.api.types.ApiErrorException;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;
import club.blockheads.mod.Replies;
import club.blockheads.mod.Senders;
import club.blockheads.mod.Spec;

import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Logs every event it receives and exercises the whole API from a command. */
public final class Sink implements Mod {
    static String who(Player p) {
        return p == null ? "-" : p.key();
    }

    @Override
    public void onEnable(Api api) throws Exception {
        api.onPlayerJoin(e -> api.info("join " + e.player.alias() + " " + e.player.role()));
        api.onPlayerQuit(e -> api.info("quit " + who(e.player) + " " + e.wasKick));
        api.onChat(e -> api.info("chat " + who(e.player) + " " + e.text + " " + e.cancelled));
        api.onCommand(e -> api.info("command " + e.name + " " + e.args + " " + e.raw + " "
                + Senders.name(e.sender) + " " + e.cancelled));
        api.onTick(e -> api.info("tick " + e.tick + " " + e.dt));
        api.onWorldLoaded(e -> api.info("loaded " + e.worldName + " " + e.worldId));
        api.onWorldSave(e -> api.info("save " + e.phase));
        api.onPlayerPreLogin(e -> api.info("prelogin " + e.playerId + " " + e.alias + " " + e.reason));
        api.onBlockPlace(e -> api.info("place " + who(e.player) + " " + e.at.x + "," + e.at.y + " " + e.item + " "
                + e.dataA + " " + e.dataB));
        api.onBlockBreak(e -> api.info("break " + who(e.player) + " " + e.at.x + "," + e.at.y + " " + e.layer + " "
                + e.tile));
        api.onBlockChanged(e -> api.info("changed " + e.at.x + "," + e.at.y + " " + e.tile + " " + who(e.player)));
        api.onObjectPlace(e -> api.info("object " + who(e.player) + " " + e.object + " " + e.item));
        api.onDamage(e -> api.info("damage " + who(e.player) + " " + e.blockhead + " " + e.amount + " "
                + who(e.attacker)));
        api.onDeath(e -> api.info("death " + who(e.player) + " " + e.blockhead));
        api.onPlayerMove(e -> api.info("move " + who(e.player) + " " + e.from.x + "," + e.from.y + "->" + e.to.x + ","
                + e.to.y));
        api.onCustom("sink:evt", e -> api.info("custom " + e.name + " " + e.source + " " + e.payload));
        api.openChannel("sink:chan", (Source origin, byte[] data) -> api.info("channel " + origin.tag() + " "
                + new String(data)));

        api.command(Spec.command("api").build(), (sender, label, args) -> {
            Player me = Senders.player(sender);
            api.info("facts " + api.host().apiVersion() + " " + api.host().modId() + " " + api.host().modVersion() + " "
                    + api.host().currentSide() + " " + api.host().hostName());
            api.warn("warn");
            api.error("error");
            api.debug("debug");
            api.info("get " + who(api.players().get("p1")) + " " + who(api.players().get("zz")));
            api.info("alias " + who(api.players().findByAlias("ALICE")) + " " + who(api.players().findByAlias("x")));
            api.info("online " + api.players().online().size());
            api.info("me " + me.alias() + " " + me.isOnline() + " " + me.role() + " " + me.id());
            api.info("pos " + (me.pos() == null ? "none" : me.pos().x + "," + me.pos().y));
            long h = me.valueHandle();
            api.info("handle " + who(api.players().fromValueHandle(h)) + " " + who(api.players().fromValueHandle(1)));
            me.sendChat("hi from java");
            try {
                api.players().broadcast("denied");
            } catch (ApiErrorException e) {
                api.info("broadcast " + e.error.tag() + " " + e.getMessage());
            }
            api.storage().set("a", new byte[] {1, 2});
            api.storage().set("ab", new byte[] {3});
            api.info("storage " + Arrays.toString(api.storage().get("a")) + " " + api.storage().get("zz") + " "
                    + api.storage().keys("a") + " " + api.storage().delete("a") + " " + api.storage().delete("a"));
            long later = api.runLater(5, () -> api.info("later"));
            long every = api.runEvery(1, 2, () -> api.info("every"));
            api.runNextPoll(() -> api.info("poll"));
            api.info("cancel " + api.cancel(later) + " " + api.cancel(later));
            api.cancel(every);
            int sub = api.onChat(e -> { });
            api.info("off " + api.off(sub) + " " + api.off(sub));
            api.info("fire " + api.fireCustom("sink:evt", mapOf("n", 1L, "p", me)));
            long ch = api.openChannel("sink:other", (o, d) -> { });
            api.send(ch, new Target.Local(), new byte[] {9});
            api.info("close " + api.closeChannel(ch));
            long id = api.command(Spec.command("tmp").build(), (s, l, a) -> null);
            api.info("unregister " + api.unregisterCommand(id) + " " + api.unregisterCommand(id));
            return Replies.reply("done");
        });
    }

    static Map<String, Object> mapOf(Object... kv) {
        Map<String, Object> m = new LinkedHashMap<String, Object>();
        for (int i = 0; i < kv.length; i += 2) {
            m.put((String) kv[i], kv[i + 1]);
        }
        return m;
    }

    static List<Object> list(Object... xs) {
        return Arrays.asList(xs);
    }
}
