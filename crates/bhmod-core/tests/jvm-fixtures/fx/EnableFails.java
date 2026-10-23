package fx;

import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;

public final class EnableFails implements Mod {
    @Override
    public void onEnable(Api api) throws Exception {
        api.info("about to fail");
        throw new Exception("enable failed");
    }
}
