package fx;

import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;

public final class EnableError implements Mod {
    @Override
    public void onEnable(Api api) {
        throw new StackOverflowError();
    }
}
