// SPDX-License-Identifier: MIT OR Apache-2.0
package club.blockheads.mod.rt;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.lang.reflect.Constructor;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

/**
 * Loads the classes of a mod jar from memory, each mod in its own class loader (child of the loader of this
 * runtime, so the mod sees the SDK but not other mods).
 *
 * <p>On a desktop JVM the {@code .class} entries are defined directly. On Android (ART) a jar with
 * {@code classes.dex} entries is loaded with {@code dalvik.system.InMemoryDexClassLoader}.
 */
final class ModLoader {
    private ModLoader() {
    }

    /** The entries of a jar, in order. */
    static Map<String, byte[]> entries(byte[] jar) throws IOException {
        Map<String, byte[]> out = new LinkedHashMap<String, byte[]>();
        ZipInputStream in = new ZipInputStream(new ByteArrayInputStream(jar));
        try {
            ZipEntry e;
            byte[] buf = new byte[8192];
            while ((e = in.getNextEntry()) != null) {
                if (e.isDirectory()) {
                    continue;
                }
                ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                int n;
                while ((n = in.read(buf)) > 0) {
                    bytes.write(buf, 0, n);
                }
                out.put(e.getName(), bytes.toByteArray());
            }
        } finally {
            in.close();
        }
        return out;
    }

    /** The main class named in the jar manifest ({@code Bhmod-Main}, else {@code Main-Class}), or null. */
    static String mainClass(Map<String, byte[]> entries) {
        byte[] mf = entries.get("META-INF/MANIFEST.MF");
        if (mf == null) {
            return null;
        }
        String main = null;
        for (String line : new String(mf, StandardCharsets.UTF_8).split("\r?\n")) {
            if (line.startsWith("Bhmod-Main:")) {
                return line.substring("Bhmod-Main:".length()).trim();
            }
            if (line.startsWith("Main-Class:")) {
                main = line.substring("Main-Class:".length()).trim();
            }
        }
        return main;
    }

    /** Creates the class loader of a mod. */
    static ClassLoader loader(Map<String, byte[]> entries, ClassLoader parent) throws Exception {
        List<byte[]> dex = new ArrayList<byte[]>();
        for (Map.Entry<String, byte[]> e : entries.entrySet()) {
            if (e.getKey().matches("classes[0-9]*\\.dex")) {
                dex.add(e.getValue());
            }
        }
        if (!dex.isEmpty()) {
            ClassLoader l = dexLoader(dex, parent);
            if (l != null) {
                return l;
            }
        }
        return new JarLoader(entries, parent);
    }

    /** InMemoryDexClassLoader through reflection (it only exists on Android); null elsewhere. */
    private static ClassLoader dexLoader(List<byte[]> dex, ClassLoader parent) throws Exception {
        Class<?> c;
        try {
            c = Class.forName("dalvik.system.InMemoryDexClassLoader");
        } catch (ClassNotFoundException e) {
            return null;
        }
        ByteBuffer[] buffers = new ByteBuffer[dex.size()];
        for (int i = 0; i < buffers.length; i++) {
            buffers[i] = ByteBuffer.wrap(dex.get(i));
        }
        try {
            Constructor<?> k = c.getConstructor(ByteBuffer[].class, ClassLoader.class);
            return (ClassLoader) k.newInstance(buffers, parent);
        } catch (NoSuchMethodException e) {
            // API 26: a single buffer only.
            Constructor<?> k = c.getConstructor(ByteBuffer.class, ClassLoader.class);
            return (ClassLoader) k.newInstance(buffers[0], parent);
        }
    }

    /** Defines classes from the {@code .class} entries of a jar held in memory. */
    static final class JarLoader extends ClassLoader {
        private final Map<String, byte[]> entries;

        JarLoader(Map<String, byte[]> entries, ClassLoader parent) {
            super(parent);
            this.entries = entries;
        }

        @Override
        protected Class<?> findClass(String name) throws ClassNotFoundException {
            byte[] b = entries.get(name.replace('.', '/') + ".class");
            if (b == null) {
                throw new ClassNotFoundException(name);
            }
            return defineClass(name, b, 0, b.length);
        }

        @Override
        public InputStream getResourceAsStream(String name) {
            byte[] b = entries.get(name);
            return b != null ? new ByteArrayInputStream(b) : super.getResourceAsStream(name);
        }
    }
}
