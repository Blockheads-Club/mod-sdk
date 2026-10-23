package fx;

import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;

/** Fails in every way a mod can: the command name picks the failure. */
public final class Faulty implements Mod {
    @Override
    public void onEnable(Api api) {
        api.onChat(e -> {
            if (e.text.equals("boom")) {
                throw new IllegalStateException("handler boom");
            }
            if (e.text.equals("error")) {
                throw new AssertionError("fatal");
            }
            if (e.text.equals("sleep")) {
                try {
                    Thread.sleep(10000);
                } catch (InterruptedException ex) {
                    throw new IllegalStateException("interrupted");
                }
            }
            if (e.text.equals("slow")) {
                try {
                    Thread.sleep(30);
                } catch (InterruptedException ex) {
                    throw new IllegalStateException("interrupted");
                }
            }
        });
        api.onTick(e -> {
            throw new IllegalStateException("tick boom");
        });
        api.command("nohandler", "x", null);
    }

    @Override
    public void onDisable() throws Exception {
        throw new Exception("disable boom");
    }
}
