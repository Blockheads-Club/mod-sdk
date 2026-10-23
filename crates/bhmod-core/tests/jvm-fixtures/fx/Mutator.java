package fx;

import club.blockheads.api.events.RejectReason;
import club.blockheads.api.types.Priority;
import club.blockheads.mod.Api;
import club.blockheads.mod.Mod;

import java.util.LinkedHashMap;
import java.util.Map;

/** Changes the mutable parts of events. */
public final class Mutator implements Mod {
    @Override
    public void onEnable(Api api) {
        api.onChat(Priority.NORMAL, false, e -> e.text = e.text.toUpperCase());
        api.onPlayerPreLogin(Priority.NORMAL, false, e -> {
            e.cancelled = true;
            e.reason = RejectReason.BANNED;
        });
        api.onDamage(Priority.NORMAL, false, e -> e.amount = e.amount * 2);
        api.onCustom("mut:evt", Priority.NORMAL, false, e -> {
            Map<String, Object> m = new LinkedHashMap<String, Object>();
            m.put("seen", e.payload);
            e.payload = m;
            e.cancelled = true;
        });
        api.onBlockPlace(Priority.NORMAL, false, e -> e.cancelled = true);
        api.onTick(e -> { });
    }
}
