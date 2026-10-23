// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod.rt;

import club.blockheads.api.guest.GuestDispatch;
import club.blockheads.mod.Mod;
import club.blockheads.mod.ModGuest;

import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * The entry points the Rust runtime calls. Every function returns canonical CBOR {@code [status, payload]}:
 * status 0 is success, 1 an {@link Exception} (text), 2 a fatal {@link Throwable} such as an {@code Error}
 * (text). Nothing is thrown to the caller.
 */
public final class Glue {
    private static final Map<Integer, ModGuest> GUESTS = new HashMap<Integer, ModGuest>();
    private static final Map<Integer, Bridge> BRIDGES = new HashMap<Integer, Bridge>();
    private static final Map<Integer, Thread> RUNNING = new HashMap<Integer, Thread>();

    private Glue() {
    }

    /** Called by the runtime: a call into the host. {@code args} and the result are CBOR. */
    static native byte[] host(int inst, String fn, byte[] args);

    private static byte[] result(int status, Object payload) {
        return Cbor.encode(Arrays.asList((Object) Long.valueOf(status), payload));
    }

    /** Text for logs: the throwable and its first frames. */
    static String describe(Throwable t) {
        StringBuilder s = new StringBuilder(String.valueOf(t));
        StackTraceElement[] st = t.getStackTrace();
        for (int i = 0; i < st.length && i < 4; i++) {
            s.append("\n    at ").append(st[i]);
        }
        Throwable c = t.getCause();
        if (c != null && c != t) {
            s.append("\ncaused by ").append(c);
        }
        return s.toString();
    }

    /**
     * Prepares a mod: opens its jar and finds its main class. Runs no mod code.
     *
     * @return {@code [0, main-class]}
     */
    public static byte[] create(final int inst, String modId, byte[] jar) {
        try {
            final Map<String, byte[]> entries = ModLoader.entries(jar);
            final String main = ModLoader.mainClass(entries);
            if (main == null) {
                return result(1, modId + ": the jar manifest has no Bhmod-Main (or Main-Class) entry");
            }
            final ClassLoader loader = ModLoader.loader(entries, Glue.class.getClassLoader());
            Bridge bridge = new NativeBridge(inst);
            ModGuest guest = new ModGuest(bridge, new ModGuest.Factory() {
                @Override
                public Mod create() throws Exception {
                    Class<?> c = Class.forName(main, true, loader);
                    return (Mod) c.getDeclaredConstructor().newInstance();
                }
            });
            synchronized (GUESTS) {
                GUESTS.put(inst, guest);
                BRIDGES.put(inst, bridge);
            }
            return result(0, main);
        } catch (Exception e) {
            return result(1, describe(e));
        } catch (Throwable t) {
            return result(2, describe(t));
        }
    }

    /**
     * Calls a WIT {@code guest} function: {@code fn} is its name, {@code args} the CBOR list of its arguments.
     *
     * @return {@code [0, result]}
     */
    public static byte[] invoke(int inst, String fn, byte[] args) {
        ModGuest guest;
        Bridge bridge;
        synchronized (GUESTS) {
            guest = GUESTS.get(inst);
            bridge = BRIDGES.get(inst);
            RUNNING.put(inst, Thread.currentThread());
        }
        try {
            if (guest == null) {
                return result(1, "no such mod instance " + inst);
            }
            @SuppressWarnings("unchecked")
            List<Object> list = (List<Object>) Cbor.decode(args);
            return result(0, GuestDispatch.invoke(guest, bridge, fn, list));
        } catch (ModGuest.ProviderError e) {
            return result(1, e.getMessage());
        } catch (Exception e) {
            return result(1, describe(e));
        } catch (Throwable t) {
            return result(2, describe(t));
        } finally {
            synchronized (GUESTS) {
                RUNNING.remove(inst);
                // Clear a pending interrupt so it cannot leak into the next call.
                Thread.interrupted();
            }
        }
    }

    /** Interrupts the thread that runs a call of {@code inst} (the runtime's watchdog at the hard deadline). */
    public static void interrupt(int inst) {
        synchronized (GUESTS) {
            Thread t = RUNNING.get(inst);
            if (t != null) {
                t.interrupt();
            }
        }
    }

    /** Forgets a mod instance. */
    public static void destroy(int inst) {
        synchronized (GUESTS) {
            GUESTS.remove(inst);
            BRIDGES.remove(inst);
        }
    }
}
