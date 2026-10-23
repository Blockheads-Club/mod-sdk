package club.example.hello;

import club.blockheads.api.events.ChatEvent;
import club.blockheads.api.events.PlayerJoinEvent;
import club.blockheads.api.types.Priority;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;
import club.blockheads.mod.Replies;
import club.blockheads.mod.Senders;
import club.blockheads.mod.Spec;

import java.nio.charset.StandardCharsets;
import java.util.Locale;

/**
 * hello-java: the Java twin of hello-lua. It adds a /hello command, greets players who join and counts greetings
 * in mod storage. The SDK's conformance suite checks that both behave the same.
 */
public final class Hello implements Mod {
    private Api api;

    private long greetings() {
        byte[] raw = api.storage().get("greetings");
        return raw == null ? 0 : Long.parseLong(new String(raw, StandardCharsets.UTF_8));
    }

    @Override
    public void onEnable(Api api) {
        this.api = api;

        api.onPlayerJoin((PlayerJoinEvent ev) -> {
            long count = greetings() + 1;
            api.storage().set("greetings", Long.toString(count).getBytes(StandardCharsets.UTF_8));
            ev.player.sendChat("Hello, " + ev.player.alias() + "! You are greeting number " + count + ".");
        });

        // Lowest priority runs first: later handlers may still change their mind.
        api.onChat(Priority.LOWEST, false, (ChatEvent ev) -> {
            if (ev.text.toLowerCase(Locale.ROOT).contains("lua is bad")) {
                ev.cancelled = true;
            }
        });

        api.command(Spec.command("hello").alias("hi").description("Says hello.").build(), (sender, label, args) -> {
            String who = Senders.name(sender);
            api.players().broadcast(who + " says hello to everyone!");
            return Replies.reply("Hello, " + who + "!");
        });

        api.info(api.host().modId() + " enabled");
    }

    @Override
    public void onDisable() {
        api.info(api.host().modId() + " disabled");
    }
}
