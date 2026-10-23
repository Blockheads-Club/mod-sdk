package club.example;

import club.blockheads.api.players.Player;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;
import club.blockheads.mod.Replies;
import club.blockheads.mod.Senders;
import club.blockheads.mod.Spec;

import java.util.List;

/**
 * scheduler-java: the Java twin of scheduler-lua. Delayed and repeating tasks. One tick is one world step (0.2 s on
 * the server), so 5 ticks = 1 s.
 */
public final class Scheduler implements Mod {
    private static final int TICKS_PER_SECOND = 5;

    private Api api;
    private Long countdown;
    private int left;

    /** Lua's {@code tonumber}: a number, or null when the text is not one. */
    private static Double number(List<String> args, int i) {
        if (i >= args.size()) {
            return null;
        }
        try {
            return Double.valueOf(args.get(i).trim());
        } catch (NumberFormatException e) {
            return null;
        }
    }

    @Override
    public void onEnable(Api api) {
        this.api = api;

        // A repeating tip every 5 minutes.
        api.runEvery(300 * TICKS_PER_SECOND, 300 * TICKS_PER_SECOND,
                () -> api.players().broadcast("Tip: type /remind 30 drink water"));

        // /countdown [seconds]: counts down in chat once per second.
        api.command(Spec.command("countdown").usage("/countdown [seconds]").description("Counts down in chat.").build(),
                (sender, label, args) -> {
                    if (countdown != null) {
                        return Replies.reply("A countdown is already running.");
                    }
                    Double requested = number(args, 0);
                    left = (int) Math.max(1, Math.min(10, requested == null ? 3 : requested));
                    countdown = api.runEvery(0, TICKS_PER_SECOND, () -> {
                        if (left == 0) {
                            api.players().broadcast("Go!");
                            api.cancel(countdown);
                            countdown = null;
                        } else {
                            api.players().broadcast(Integer.toString(left));
                            left -= 1;
                        }
                    });
                    return Replies.reply();
                });

        // /remind <seconds> <text>: a one-shot task, answered right away from the next network poll.
        api.command(Spec.command("remind").usage("/remind <seconds> <text>").description("Reminds you later.").build(),
                (sender, label, args) -> {
                    Double seconds = number(args, 0);
                    Player who = Senders.player(sender);
                    if (seconds == null || args.size() < 2 || who == null) {
                        return Replies.usage();
                    }
                    String text = String.join(" ", args.subList(1, args.size()));
                    api.runLater((int) Math.floor(seconds * TICKS_PER_SECOND), () -> {
                        if (who.isOnline()) {
                            who.sendChat("Reminder: " + text);
                        }
                    });
                    api.runNextPoll(() -> {
                        if (who.isOnline()) {
                            who.sendChat("OK, I will remind you in " + format(seconds) + " s.");
                        }
                    });
                    return Replies.reply();
                });
    }

    /** Lua prints whole numbers without a fraction. */
    private static String format(double seconds) {
        return seconds == Math.floor(seconds) ? Long.toString((long) seconds) : Double.toString(seconds);
    }
}
