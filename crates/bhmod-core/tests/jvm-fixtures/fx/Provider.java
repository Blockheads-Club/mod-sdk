package fx;

import club.blockheads.api.players.Player;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;
import club.blockheads.mod.ServiceError;

import java.util.List;

/** Provides the `calc` service. */
public final class Provider implements Mod {
    @Override
    public void onEnable(Api api) {
        api.provide("calc", "1.0.0", (method, args) -> {
            switch (method) {
                case "add":
                    return ((Long) args.get(0)) + ((Long) args.get(1));
                case "name":
                    return ((Player) args.get(0)).alias();
                case "echo":
                    return args;
                case "fail":
                    throw new ServiceError("calc refused");
                case "crash":
                    throw new IllegalStateException("calc crashed");
                case "error":
                    throw new AssertionError("calc fatal");
                case "bad":
                    return new Object();
                default:
                    throw new ServiceError("no method " + method);
            }
        });
    }
}
