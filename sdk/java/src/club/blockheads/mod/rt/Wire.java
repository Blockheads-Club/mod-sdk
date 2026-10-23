// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod.rt;

import club.blockheads.api.players.Player;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Helpers of the generated codecs: checked conversions between wire objects and Java values. */
public final class Wire {
    /** The map key of a host handle inside a {@code value}. */
    public static final String HANDLE_KEY = "$h";

    private Wire() {
    }

    /** A function used by the generated codecs. */
    public interface Fn<A, B> {
        B apply(A a);
    }

    private static IllegalArgumentException bad(String what, Object o) {
        return new IllegalArgumentException("bad wire value: expected " + what + ", found " + describe(o));
    }

    /** A short text for error messages. */
    public static String describe(Object o) {
        if (o == null) {
            return "null";
        }
        if (o instanceof byte[]) {
            return "bytes(" + ((byte[]) o).length + ")";
        }
        if (o instanceof Map && ((Map<?, ?>) o).size() == 1) {
            Map.Entry<?, ?> e = ((Map<?, ?>) o).entrySet().iterator().next();
            return e.getKey() + (e.getValue() == null ? "" : ": " + describe(e.getValue()));
        }
        return String.valueOf(o);
    }

    public static boolean bool(Object o) {
        if (o instanceof Boolean) {
            return (Boolean) o;
        }
        throw bad("a boolean", o);
    }

    public static long num(Object o) {
        if (o instanceof Long || o instanceof Integer) {
            return ((Number) o).longValue();
        }
        throw bad("an integer", o);
    }

    public static double dbl(Object o) {
        if (o instanceof Number) {
            return ((Number) o).doubleValue();
        }
        throw bad("a number", o);
    }

    public static String text(Object o) {
        if (o instanceof String) {
            return (String) o;
        }
        throw bad("text", o);
    }

    public static byte[] bytes(Object o) {
        if (o instanceof byte[]) {
            return (byte[]) o;
        }
        throw bad("bytes", o);
    }

    @SuppressWarnings("unchecked")
    public static Map<String, Object> map(Object o) {
        if (o instanceof Map) {
            return (Map<String, Object>) o;
        }
        throw bad("a map", o);
    }

    @SuppressWarnings("unchecked")
    public static <T> List<T> list(Object o, Fn<Object, T> f) {
        if (!(o instanceof List)) {
            throw bad("a list", o);
        }
        List<T> out = new ArrayList<T>();
        for (Object x : (List<Object>) o) {
            out.add(f.apply(x));
        }
        return out;
    }

    public static <T> List<Object> listOut(List<T> l, Fn<T, Object> f) {
        List<Object> out = new ArrayList<Object>(l.size());
        for (T x : l) {
            out.add(f.apply(x));
        }
        return out;
    }

    public static <T> T optIn(Object o, Fn<Object, T> f) {
        return o == null ? null : f.apply(o);
    }

    public static <T> Object optOut(T v, Fn<T, Object> f) {
        return v == null ? null : f.apply(v);
    }

    /** Unwraps a WIT {@code result}: the payload of {@code ok}, or throws what {@code mkErr} makes of the error. */
    public static Object unwrap(Object r, Fn<Object, RuntimeException> mkErr) {
        Map<String, Object> m = map(r);
        if (m.containsKey("ok")) {
            return m.get("ok");
        }
        if (m.containsKey("err")) {
            throw mkErr.apply(m.get("err"));
        }
        throw bad("ok or err", r);
    }

    /** Decodes a {@code types.value} (nested canonical CBOR bytes) into plain Java values; handles become players. */
    public static Object value(Object o, Bridge b) {
        return fromCbor(Cbor.decode(bytes(o)), b);
    }

    /** Encodes plain Java values (players become host handles) as a {@code types.value}. */
    public static Object valueOut(Object v, Bridge b) {
        return Cbor.encode(toCbor(v, b));
    }

    private static Object fromCbor(Object v, Bridge b) {
        if (v instanceof List) {
            List<Object> out = new ArrayList<Object>();
            for (Object x : (List<?>) v) {
                out.add(fromCbor(x, b));
            }
            return out;
        }
        if (v instanceof Map) {
            Map<?, ?> m = (Map<?, ?>) v;
            if (m.size() == 1 && m.containsKey(HANDLE_KEY) && m.get(HANDLE_KEY) instanceof Long) {
                Object p = b.call("players.from-value-handle", m.get(HANDLE_KEY));
                if (p == null) {
                    throw new HostException("stale handle");
                }
                return new Player(b, text(p));
            }
            Map<String, Object> out = new LinkedHashMap<String, Object>();
            for (Map.Entry<?, ?> e : m.entrySet()) {
                out.put((String) e.getKey(), fromCbor(e.getValue(), b));
            }
            return out;
        }
        return v;
    }

    private static Object toCbor(Object v, Bridge b) {
        if (v instanceof Player) {
            Map<String, Object> h = new LinkedHashMap<String, Object>();
            h.put(HANDLE_KEY, ((Player) v).valueHandle());
            return h;
        }
        if (v instanceof List) {
            List<Object> out = new ArrayList<Object>();
            for (Object x : (List<?>) v) {
                out.add(toCbor(x, b));
            }
            return out;
        }
        if (v instanceof Object[]) {
            return toCbor(java.util.Arrays.asList((Object[]) v), b);
        }
        if (v instanceof Map) {
            Map<String, Object> out = new LinkedHashMap<String, Object>();
            for (Map.Entry<?, ?> e : ((Map<?, ?>) v).entrySet()) {
                if (!(e.getKey() instanceof String)) {
                    throw new IllegalArgumentException("bad value: map keys must be text");
                }
                out.put((String) e.getKey(), toCbor(e.getValue(), b));
            }
            return out;
        }
        return v;
    }
}
