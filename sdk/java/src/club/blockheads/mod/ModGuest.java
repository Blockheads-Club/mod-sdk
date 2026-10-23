// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod;

import club.blockheads.api.channels.Source;
import club.blockheads.api.commands.CommandOutcome;
import club.blockheads.api.events.CommandSender;
import club.blockheads.api.events.Event;
import club.blockheads.api.events.Mutation;
import club.blockheads.api.events.Outcome;
import club.blockheads.api.guest.Guest;
import club.blockheads.mod.rt.Bridge;
import club.blockheads.mod.rt.Wire;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;

/** The WIT {@code guest} export of a JVM mod: routes the host's callbacks to the handlers registered on the {@link Api}. */
public final class ModGuest implements Guest {
    /** Creates the {@link Mod} when the mod is enabled (never earlier: its constructor is mod code). */
    public interface Factory {
        Mod create() throws Exception;
    }

    private final Bridge bridge;
    private final Factory factory;
    private final Api api;
    private Mod mod;

    /** Used by the runtime. */
    public ModGuest(Bridge bridge, Factory factory) {
        this.bridge = bridge;
        this.factory = factory;
        this.api = new Api(bridge);
    }

    @SuppressWarnings("unchecked")
    private static <T extends Throwable> RuntimeException sneak(Throwable t) throws T {
        throw (T) t;
    }

    @Override
    public void onEnable() throws Exception {
        mod = factory.create();
        mod.onEnable(api);
    }

    @Override
    public void onDisable() {
        try {
            if (mod != null) {
                mod.onDisable();
            }
        } catch (Exception e) {
            throw sneak(e);
        }
    }

    @Override
    public Outcome handleEvent(long sub, Event ev) {
        Handler<Object> h = api.handlers.get(sub);
        Object record = ev.payload();
        if (h == null || record == null) {
            return new Outcome(cancelled(ev), null);
        }
        Event before = Event.fromWire(ev.toWire(bridge), bridge);
        try {
            h.handle(record);
        } catch (Exception e) {
            throw sneak(e);
        }
        return new Outcome(cancelled(ev), mutation(before, ev));
    }

    /** The `cancelled` field of the event record, false when it has none. */
    @SuppressWarnings("unchecked")
    private boolean cancelled(Event ev) {
        Object rec = ((Map<String, Object>) ev.toWire(bridge)).get(ev.tag());
        return rec instanceof Map && Boolean.TRUE.equals(((Map<String, Object>) rec).get("cancelled"));
    }

    /** The change a handler made to the mutable parts of an event (chat text, custom payload, reject reason, damage). */
    private Mutation mutation(Event before, Event after) {
        Object b = before.payload();
        Object a = after.payload();
        if (a instanceof club.blockheads.api.events.ChatEvent) {
            String text = ((club.blockheads.api.events.ChatEvent) a).text;
            if (!text.equals(((club.blockheads.api.events.ChatEvent) b).text)) {
                return new Mutation.ChatText(text);
            }
        } else if (a instanceof club.blockheads.api.events.CustomEvent) {
            Object now = ((club.blockheads.api.events.CustomEvent) a).payload;
            if (!Arrays.equals((byte[]) Wire.valueOut(now, bridge),
                    (byte[]) Wire.valueOut(((club.blockheads.api.events.CustomEvent) b).payload, bridge))) {
                return new Mutation.CustomPayload(now);
            }
        } else if (a instanceof club.blockheads.api.events.PlayerPreLoginEvent) {
            club.blockheads.api.events.RejectReason r = ((club.blockheads.api.events.PlayerPreLoginEvent) a).reason;
            if (r != ((club.blockheads.api.events.PlayerPreLoginEvent) b).reason) {
                return new Mutation.RejectReason(r);
            }
        } else if (a instanceof club.blockheads.api.events.DamageEvent) {
            float amount = ((club.blockheads.api.events.DamageEvent) a).amount;
            if (amount != ((club.blockheads.api.events.DamageEvent) b).amount) {
                return new Mutation.DamageAmount(amount);
            }
        }
        return null;
    }

    @Override
    public CommandOutcome runCommand(long id, CommandSender sender, String label, List<String> args) {
        CommandHandler h = api.commandHandlers.get(id);
        if (h == null) {
            throw new IllegalStateException("command " + id + " has no handler");
        }
        try {
            CommandOutcome o = h.run(sender, label, args);
            return o != null ? o : new CommandOutcome(true, new ArrayList<String>());
        } catch (Exception e) {
            throw sneak(e);
        }
    }

    @Override
    public void runTask(long id) {
        Object[] t = api.tasks.get(id);
        if (t == null) {
            return;
        }
        if (!(Boolean) t[1]) {
            api.tasks.remove(id);
        }
        try {
            ((Task) t[0]).run();
        } catch (Exception e) {
            throw sneak(e);
        }
    }

    @Override
    @SuppressWarnings("unchecked")
    public Object callService(String name, String method, Object args) throws Exception {
        Service s = api.providers.get(name);
        if (s == null) {
            throw new IllegalStateException("this mod does not provide " + name);
        }
        List<Object> list = args instanceof List ? (List<Object>) args : Arrays.asList(args);
        try {
            return s.call(method, list);
        } catch (ServiceError e) {
            throw new ProviderError(e.getMessage());
        }
    }

    @Override
    public void onChannelMessage(long channel, Source origin, byte[] data) {
        ChannelHandler h = api.channelHandlers.get(channel);
        if (h == null) {
            return;
        }
        try {
            h.onMessage(origin, data);
        } catch (Exception e) {
            throw sneak(e);
        }
    }

    /** A deliberate service error; the runtime reports it as `failed` (not a fault of the provider). */
    public static final class ProviderError extends Exception {
        private static final long serialVersionUID = 1L;

        ProviderError(String message) {
            super(message);
        }
    }
}
