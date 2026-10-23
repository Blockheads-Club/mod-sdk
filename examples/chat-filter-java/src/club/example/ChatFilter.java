package club.example;

import club.blockheads.api.events.ChatEvent;
import club.blockheads.api.players.Role;
import club.blockheads.api.types.Priority;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;
import club.blockheads.mod.Replies;
import club.blockheads.mod.Spec;

import java.nio.charset.StandardCharsets;
import java.util.Locale;

/**
 * chat-filter-java: the Java twin of chat-filter. Cancels chat lines with blocked links, masks rude words and logs
 * what it did. Shows event priorities, cancellation, {@code ignoreCancelled}, text rewriting and a moderator-only
 * command.
 */
public final class ChatFilter implements Mod {
    /** A message containing one of these is cancelled. */
    private static final String[] BLOCKED = {"spam.example", "free-diamonds.example"};
    /** These are replaced by asterisks. */
    private static final String[] MASKED = {"darn", "heck"};

    private Api api;

    private boolean enabled() {
        byte[] v = api.storage().get("enabled");
        return v == null || !new String(v, StandardCharsets.UTF_8).equals("off");
    }

    private static String mask(String text) {
        for (String w : MASKED) {
            StringBuilder stars = new StringBuilder();
            for (int i = 0; i < w.length(); i++) {
                stars.append('*');
            }
            text = text.replace(w, stars);
        }
        return text;
    }

    @Override
    public void onEnable(Api api) {
        this.api = api;

        // Low: decide early whether the message is dropped.
        api.onChat(Priority.LOW, false, (ChatEvent ev) -> {
            if (!enabled()) {
                return;
            }
            String lower = ev.text.toLowerCase(Locale.ROOT);
            for (String b : BLOCKED) {
                if (lower.contains(b)) {
                    ev.cancelled = true;
                    ev.player.sendChat("Your message was blocked: links to " + b + " are not allowed.");
                    return;
                }
            }
        });

        // Normal: rewrite what is left. ignoreCancelled skips messages a lower priority already dropped.
        api.onChat(Priority.NORMAL, true, (ChatEvent ev) -> {
            if (enabled()) {
                ev.text = mask(ev.text);
            }
        });

        // Monitor: sees the final state and only logs.
        api.onChat(Priority.MONITOR, false, (ChatEvent ev) -> {
            if (ev.cancelled) {
                api.info("blocked a message from " + ev.player.alias());
            }
        });

        api.command(Spec.command("chatfilter").description("Turns the chat filter on or off.")
                .usage("/chatfilter on|off").minRole(Role.MODERATOR).build(), (sender, label, args) -> {
            String mode = args.isEmpty() ? null : args.get(0);
            if (!"on".equals(mode) && !"off".equals(mode)) {
                return Replies.usage("The filter is " + (enabled() ? "on" : "off") + ".");
            }
            api.storage().set("enabled", mode.getBytes(StandardCharsets.UTF_8));
            return Replies.reply("Chat filter " + mode + ".");
        });
    }
}
