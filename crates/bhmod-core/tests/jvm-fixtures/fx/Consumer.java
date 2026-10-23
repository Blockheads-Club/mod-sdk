package fx;

import club.blockheads.api.services.CallErrorException;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;
import club.blockheads.mod.Replies;
import club.blockheads.mod.Senders;
import club.blockheads.mod.ServiceClient;
import club.blockheads.mod.Spec;

/** Calls the `calc` service from a command: `/calc <method> [args...]`. */
public final class Consumer implements Mod {
    @Override
    public void onEnable(Api api) {
        ServiceClient calc = api.service("calc", "^1");
        api.command(Spec.command("calc").build(), (sender, label, args) -> {
            String method = args.get(0);
            try {
                Object r;
                if (method.equals("add")) {
                    r = calc.call("add", Long.parseLong(args.get(1)), Long.parseLong(args.get(2)));
                } else if (method.equals("name")) {
                    r = calc.call("name", Senders.player(sender));
                } else if (method.equals("echo")) {
                    r = calc.call("echo", 1L, "two", Senders.player(sender), Sink.list(3L, null));
                } else {
                    r = calc.call(method);
                }
                return Replies.reply("ok " + r);
            } catch (CallErrorException e) {
                return Replies.reply("err " + e.getMessage());
            }
        });
    }
}
