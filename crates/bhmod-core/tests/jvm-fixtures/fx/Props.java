package fx;

import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;

/** Reports how the JVM was started and what the raw bridge does with bad calls. */
public final class Props implements Mod {
    @Override
    public void onEnable(Api api) {
        api.info("prop " + System.getProperty("bhmod.test"));
        api.info("heap " + (Runtime.getRuntime().maxMemory() / (1024 * 1024)));
        api.info("classpath " + System.getProperty("java.class.path"));
        api.info("raw " + api.bridge().call("host.mod-id"));
        for (String fn : new String[] {"nope.nothing", "players.get"}) {
            try {
                api.bridge().call(fn, 5L);
            } catch (IllegalStateException e) {
                api.info("bridge " + e.getMessage());
            }
        }
    }
}
