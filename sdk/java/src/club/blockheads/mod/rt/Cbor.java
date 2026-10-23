// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod.rt;

import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * Canonical CBOR (RFC 8949 section 4.2.1) for the dynamic {@code types.value}: the same subset the hosts accept.
 *
 * <p>Java side: {@code null}, {@link Boolean}, {@link Long} (or any integral number), {@link Double} (or
 * {@link Float}), {@link String}, {@code byte[]}, {@link List} and {@link Map} with text keys.
 */
public final class Cbor {
    /** Deepest allowed nesting. */
    public static final int MAX_DEPTH = 32;

    private Cbor() {
    }

    /** Encodes a value as canonical CBOR. */
    public static byte[] encode(Object v) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        write(out, v, 0);
        return out.toByteArray();
    }

    /** Decodes one canonical CBOR value that spans the whole input. */
    public static Object decode(byte[] b) {
        Reader r = new Reader(b);
        Object v = r.value(0);
        if (r.i != b.length) {
            throw new IllegalArgumentException("bad value: trailing bytes after the value");
        }
        return v;
    }

    private static void head(ByteArrayOutputStream out, int major, long n) {
        int m = major << 5;
        if (n >= 0 && n <= 23) {
            out.write(m | (int) n);
        } else if (n >= 0 && n <= 0xff) {
            out.write(m | 24);
            out.write((int) n);
        } else if (n >= 0 && n <= 0xffff) {
            out.write(m | 25);
            out.write((int) (n >> 8));
            out.write((int) n);
        } else if (n >= 0 && n <= 0xffffffffL) {
            out.write(m | 26);
            for (int s = 24; s >= 0; s -= 8) {
                out.write((int) (n >> s));
            }
        } else {
            out.write(m | 27);
            for (int s = 56; s >= 0; s -= 8) {
                out.write((int) (n >> s));
            }
        }
    }

    private static Integer toHalf(double v) {
        float f = (float) v;
        if ((double) f != v) {
            return null;
        }
        int bits = Float.floatToRawIntBits(f);
        int sign = (bits >>> 16) & 0x8000;
        int exp = (bits >>> 23) & 0xff;
        int man = bits & 0x7fffff;
        if (exp == 0xff) {
            return sign | 0x7c00;
        }
        if (exp == 0 && man == 0) {
            return sign;
        }
        int e = exp - 127;
        if (e >= -14 && e <= 15) {
            if ((man & 0x1fff) != 0) {
                return null;
            }
            return sign | ((e + 15) << 10) | (man >>> 13);
        }
        if (e >= -24 && e < -14) {
            int full = man | 0x800000;
            int shift = (-14 - e) + 13;
            if ((full & ((1 << shift) - 1)) != 0) {
                return null;
            }
            return sign | (full >>> shift);
        }
        return null;
    }

    private static double fromHalf(int h) {
        double sign = (h & 0x8000) != 0 ? -1.0 : 1.0;
        int exp = (h >> 10) & 0x1f;
        double man = h & 0x3ff;
        if (exp == 0) {
            return sign * man * Math.pow(2, -24);
        }
        if (exp == 0x1f) {
            return man == 0 ? sign * Double.POSITIVE_INFINITY : Double.NaN;
        }
        return sign * (1.0 + man / 1024.0) * Math.pow(2, exp - 15);
    }

    private static void writeFloat(ByteArrayOutputStream out, double v) {
        if (Double.isNaN(v)) {
            out.write(0xf9);
            out.write(0x7e);
            out.write(0x00);
            return;
        }
        Integer h = toHalf(v);
        if (h != null) {
            out.write(0xf9);
            out.write(h >> 8);
            out.write(h);
        } else if ((double) (float) v == v) {
            out.write(0xfa);
            int bits = Float.floatToRawIntBits((float) v);
            for (int s = 24; s >= 0; s -= 8) {
                out.write(bits >> s);
            }
        } else {
            out.write(0xfb);
            long bits = Double.doubleToRawLongBits(v);
            for (int s = 56; s >= 0; s -= 8) {
                out.write((int) (bits >> s));
            }
        }
    }

    private static void write(ByteArrayOutputStream out, Object v, int depth) {
        if (depth > MAX_DEPTH) {
            throw new IllegalArgumentException("bad value: nested too deeply");
        }
        if (v == null) {
            out.write(0xf6);
        } else if (v instanceof Boolean) {
            out.write((Boolean) v ? 0xf5 : 0xf4);
        } else if (v instanceof Long || v instanceof Integer || v instanceof Short || v instanceof Byte) {
            long n = ((Number) v).longValue();
            if (n >= 0) {
                head(out, 0, n);
            } else {
                head(out, 1, ~n);
            }
        } else if (v instanceof Double || v instanceof Float) {
            writeFloat(out, ((Number) v).doubleValue());
        } else if (v instanceof String) {
            byte[] s = ((String) v).getBytes(StandardCharsets.UTF_8);
            head(out, 3, s.length);
            out.write(s, 0, s.length);
        } else if (v instanceof byte[]) {
            byte[] s = (byte[]) v;
            head(out, 2, s.length);
            out.write(s, 0, s.length);
        } else if (v instanceof List) {
            List<?> l = (List<?>) v;
            head(out, 4, l.size());
            for (Object o : l) {
                write(out, o, depth + 1);
            }
        } else if (v instanceof Map) {
            writeMap(out, (Map<?, ?>) v, depth);
        } else {
            throw new IllegalArgumentException("bad value: cannot convert a " + v.getClass().getName());
        }
    }

    private static final java.util.Comparator<byte[]> BYTEWISE = new java.util.Comparator<byte[]>() {
        @Override
        public int compare(byte[] a, byte[] b) {
            int n = Math.min(a.length, b.length);
            for (int i = 0; i < n; i++) {
                int d = (a[i] & 0xff) - (b[i] & 0xff);
                if (d != 0) {
                    return d;
                }
            }
            return a.length - b.length;
        }
    };

    private static void writeMap(ByteArrayOutputStream out, Map<?, ?> m, int depth) {
        TreeMap<byte[], Object> sorted = new TreeMap<byte[], Object>(BYTEWISE);
        for (Map.Entry<?, ?> e : m.entrySet()) {
            if (!(e.getKey() instanceof String)) {
                throw new IllegalArgumentException("bad value: map keys must be text");
            }
            ByteArrayOutputStream k = new ByteArrayOutputStream();
            write(k, e.getKey(), depth + 1);
            sorted.put(k.toByteArray(), e.getValue());
        }
        head(out, 5, sorted.size());
        for (Map.Entry<byte[], Object> e : sorted.entrySet()) {
            out.write(e.getKey(), 0, e.getKey().length);
            write(out, e.getValue(), depth + 1);
        }
    }

    private static final class Reader {
        final byte[] b;
        int i;

        Reader(byte[] b) {
            this.b = b;
        }

        private IllegalArgumentException bad(String msg) {
            return new IllegalArgumentException("bad value: " + msg);
        }

        int take1() {
            if (i >= b.length) {
                throw bad("truncated value");
            }
            return b[i++] & 0xff;
        }

        byte[] take(int n) {
            if (n < 0 || b.length - i < n) {
                throw bad("truncated value");
            }
            byte[] s = Arrays.copyOfRange(b, i, i + n);
            i += n;
            return s;
        }

        long arg(int info) {
            long n;
            long min;
            switch (info) {
                case 24:
                    n = take1();
                    min = 24;
                    break;
                case 25:
                    n = ((long) take1() << 8) | take1();
                    min = 0x100;
                    break;
                case 26:
                    n = 0;
                    for (int k = 0; k < 4; k++) {
                        n = (n << 8) | take1();
                    }
                    min = 0x10000;
                    break;
                case 27:
                    n = 0;
                    for (int k = 0; k < 8; k++) {
                        n = (n << 8) | take1();
                    }
                    min = 0x100000000L;
                    if (n < 0) {
                        throw bad("integer out of range");
                    }
                    break;
                default:
                    if (info <= 23) {
                        return info;
                    }
                    throw bad("indefinite lengths and reserved arguments are not allowed");
            }
            if (n < min) {
                throw bad("argument not in its shortest form");
            }
            return n;
        }

        int len(int info) {
            long n = arg(info);
            if (n > b.length - i) {
                throw bad("length larger than the input");
            }
            return (int) n;
        }

        String text(int info) {
            return new String(take(len(info)), StandardCharsets.UTF_8);
        }

        Object value(int depth) {
            if (depth > MAX_DEPTH) {
                throw bad("nested deeper than " + MAX_DEPTH);
            }
            int ib = take1();
            int major = ib >> 5;
            int info = ib & 0x1f;
            switch (major) {
                case 0:
                    return arg(info);
                case 1:
                    return ~arg(info);
                case 2:
                    return take(len(info));
                case 3:
                    return text(info);
                case 4: {
                    int n = len(info);
                    List<Object> l = new ArrayList<Object>(n);
                    for (int k = 0; k < n; k++) {
                        l.add(value(depth + 1));
                    }
                    return l;
                }
                case 5: {
                    int n = len(info);
                    Map<String, Object> m = new java.util.LinkedHashMap<String, Object>();
                    byte[] last = null;
                    for (int k = 0; k < n; k++) {
                        int start = i;
                        int kb = take1();
                        if (kb >> 5 != 3) {
                            throw bad("map keys must be text");
                        }
                        String key = text(kb & 0x1f);
                        byte[] enc = Arrays.copyOfRange(b, start, i);
                        if (last != null && BYTEWISE.compare(last, enc) >= 0) {
                            throw bad("map keys duplicated or not in canonical order");
                        }
                        last = enc;
                        m.put(key, value(depth + 1));
                    }
                    return m;
                }
                case 6:
                    throw bad("tags are not allowed");
                default:
                    return simple(info);
            }
        }

        Object simple(int info) {
            switch (info) {
                case 20:
                    return Boolean.FALSE;
                case 21:
                    return Boolean.TRUE;
                case 22:
                    return null;
                case 25: {
                    int h = (take1() << 8) | take1();
                    double f = fromHalf(h);
                    if (Double.isNaN(f) && h != 0x7e00) {
                        throw bad("NaN must be encoded as f97e00");
                    }
                    return f;
                }
                case 26: {
                    int bits = 0;
                    for (int k = 0; k < 4; k++) {
                        bits = (bits << 8) | take1();
                    }
                    double f = Float.intBitsToFloat(bits);
                    if (Double.isNaN(f) || toHalf(f) != null) {
                        throw bad("float not in its shortest form");
                    }
                    return f;
                }
                case 27: {
                    long bits = 0;
                    for (int k = 0; k < 8; k++) {
                        bits = (bits << 8) | take1();
                    }
                    double f = Double.longBitsToDouble(bits);
                    if (Double.isNaN(f) || (double) (float) f == f) {
                        throw bad("float not in its shortest form");
                    }
                    return f;
                }
                default:
                    throw bad("unsupported simple value");
            }
        }
    }
}
