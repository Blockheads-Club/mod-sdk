#!/usr/bin/env python3
"""Generates the Java bindings of the `blockheads:api` WIT package (the JVM SDK, milestone M9).

    scripts/gen-java.py [WIT_DIR] [OUT_DIR]          write the bindings (defaults: wit/blockheads-api, sdk/java/src)
    scripts/gen-java.py --check [WIT_DIR] [OUT_DIR]  fail if the committed bindings differ from the WIT

Every interface `foo-bar` becomes the Java package `club.blockheads.api.foobar`:

  * enums, records and variants become Java enums and classes with `fromWire` / `toWire` codecs,
  * resources become classes whose methods call the host,
  * the functions of an interface become the Java interface `FooBar` and its host-backed class `FooBarImpl`,
  * the `guest` interface becomes `Guest` plus the dispatcher `GuestDispatch` the runtime uses to call the mod,
  * `events` additionally gets `EventHandlers`, one `onXxx` helper per event kind.

The wire form is the CBOR convention of `types.value` (records are maps with kebab-case keys, variants one-entry
maps, enums text, `option` null or the value, `result` is `{"ok": v}` / `{"err": e}`, resources are the text key of
the host object, `types.value` itself is a nested canonical CBOR byte string that the SDK exposes as a plain Java
object). Java is generated for `--release 8` so it also runs on ART.

Only the Python standard library is used. Rerun after every change to the WIT; CI runs `--check`.
"""
import os
import re
import sys

PKG = "club.blockheads.api"
RT = "club.blockheads.mod.rt"

JAVA_RESERVED = set(
    """abstract assert boolean break byte case catch char class const continue default do double else enum extends
    final finally float for goto if implements import instanceof int interface long native new package private
    protected public return short static strictfp super switch synchronized this throw throws transient try void
    volatile while true false null var record""".split()
)

PRIMS = {
    # wit: (java primitive, java boxed, wire decoder, wire encoder)
    "bool": ("boolean", "Boolean", "Wire.bool(%s)", "%s"),
    "u8": ("int", "Integer", "(int) Wire.num(%s)", "(long) %s"),
    "u16": ("int", "Integer", "(int) Wire.num(%s)", "(long) %s"),
    "s8": ("int", "Integer", "(int) Wire.num(%s)", "(long) %s"),
    "s16": ("int", "Integer", "(int) Wire.num(%s)", "(long) %s"),
    "s32": ("int", "Integer", "(int) Wire.num(%s)", "(long) %s"),
    "u32": ("long", "Long", "Wire.num(%s)", "%s"),
    "u64": ("long", "Long", "Wire.num(%s)", "%s"),
    "s64": ("long", "Long", "Wire.num(%s)", "%s"),
    "f32": ("float", "Float", "(float) Wire.dbl(%s)", "(double) %s"),
    "f64": ("double", "Double", "Wire.dbl(%s)", "%s"),
    "char": ("String", "String", "Wire.text(%s)", "%s"),
    "string": ("String", "String", "Wire.text(%s)", "%s"),
}


# ---------------------------------------------------------------------------------------------------------------
# WIT parsing (the subset the package uses, plus resources, tuples and annotations)

class WitError(Exception):
    pass


def tokenize(src):
    """Returns (kind, text, doc) tokens; `doc` is the `///` block right before an item's first token."""
    toks = []
    i, n = 0, len(src)
    doc = []
    while i < n:
        c = src[i]
        if c in " \t\r":
            i += 1
        elif c == "\n":
            i += 1
        elif src.startswith("///", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            doc.append(src[i + 3 : j].strip())
            i = j
        elif src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
        elif src.startswith("/*", i):
            j = src.find("*/", i)
            if j < 0:
                raise WitError("unterminated block comment")
            i = j + 2
        elif src.startswith("->", i):
            toks.append(("sym", "->", doc))
            doc = []
            i += 2
        elif c in "{}()<>,:;=.@*":
            toks.append(("sym", c, doc))
            doc = []
            i += 1
        elif c.isalnum() or c in "%_/-+":
            j = i
            while j < n and (src[j].isalnum() or src[j] in "%_/-+"):
                j += 1
            toks.append(("id", src[i:j].lstrip("%"), doc))
            doc = []
            i = j
        else:
            raise WitError("unexpected character %r" % c)
    return toks


class Parser:
    def __init__(self, src):
        self.t = tokenize(src)
        self.i = 0

    def peek(self, k=0):
        return self.t[self.i + k] if self.i + k < len(self.t) else ("eof", "", [])

    def next(self):
        tok = self.peek()
        self.i += 1
        return tok

    def eat(self, text):
        if self.peek()[1] == text and self.peek()[0] != "eof":
            self.i += 1
            return True
        return False

    def expect(self, text):
        tok = self.next()
        if tok[1] != text:
            raise WitError("expected %r, found %r" % (text, tok[1]))
        return tok

    def ident(self):
        tok = self.next()
        if tok[0] != "id":
            raise WitError("expected an identifier, found %r" % tok[1])
        return tok[1]

    def skip_annotations(self):
        while self.peek()[1] == "@":
            self.next()
            self.ident()
            if self.eat("("):
                depth = 1
                while depth:
                    t = self.next()
                    if t[0] == "eof":
                        raise WitError("unterminated annotation")
                    depth += (t[1] == "(") - (t[1] == ")")

    def type(self):
        name = self.ident()
        if name in ("list", "option", "borrow", "own"):
            self.expect("<")
            inner = self.type()
            self.expect(">")
            return (name if name in ("list", "option") else "ref", inner)
        if name == "tuple":
            self.expect("<")
            items = [self.type()]
            while self.eat(","):
                items.append(self.type())
            self.expect(">")
            return ("tuple", items)
        if name == "result":
            if not self.eat("<"):
                return ("result", None, None)
            ok = None if self.eat("_") else self.type()
            err = None
            if self.eat(","):
                err = None if self.eat("_") else self.type()
            self.expect(">")
            return ("result", ok, err)
        if name in PRIMS:
            return ("prim", name)
        return ("named", name)

    def params(self):
        self.expect("(")
        out = []
        while not self.eat(")"):
            name = self.ident()
            self.expect(":")
            out.append((name, self.type()))
            self.eat(",")
        return out

    def func(self, name, doc, static=False):
        self.expect("func")
        params = self.params()
        ret = self.type() if self.eat("->") else None
        self.expect(";")
        return {"name": name, "doc": doc, "params": params, "ret": ret, "static": static}

    def interface(self):
        doc = self.peek()[2]
        self.expect("interface")
        name = self.ident()
        self.expect("{")
        iface = {"name": name, "doc": doc, "uses": [], "types": [], "resources": [], "funcs": []}
        while not self.eat("}"):
            doc = self.peek()[2]
            self.skip_annotations()
            tok = self.peek()
            doc = doc or tok[2]
            kw = tok[1]
            if kw == "use":
                self.next()
                src = self.ident()
                self.expect(".")
                self.expect("{")
                names = []
                while not self.eat("}"):
                    n = self.ident()
                    alias = self.ident() if self.peek()[1] == "as" and self.next() else n
                    names.append((n, alias))
                    self.eat(",")
                self.expect(";")
                iface["uses"].append((src, names))
            elif kw == "type":
                self.next()
                n = self.ident()
                self.expect("=")
                t = self.type()
                self.expect(";")
                iface["types"].append({"kind": "alias", "name": n, "doc": doc, "type": t})
            elif kw == "record":
                self.next()
                n = self.ident()
                self.expect("{")
                fields = []
                while not self.eat("}"):
                    fdoc = self.peek()[2]
                    fname = self.ident()
                    self.expect(":")
                    fields.append({"name": fname, "type": self.type(), "doc": fdoc})
                    self.eat(",")
                iface["types"].append({"kind": "record", "name": n, "doc": doc, "fields": fields})
            elif kw == "enum":
                self.next()
                n = self.ident()
                self.expect("{")
                cases = []
                while not self.eat("}"):
                    cdoc = self.peek()[2]
                    cases.append({"name": self.ident(), "doc": cdoc})
                    self.eat(",")
                iface["types"].append({"kind": "enum", "name": n, "doc": doc, "cases": cases})
            elif kw == "variant":
                self.next()
                n = self.ident()
                self.expect("{")
                cases = []
                while not self.eat("}"):
                    cdoc = self.peek()[2]
                    cname = self.ident()
                    ty = None
                    if self.eat("("):
                        ty = self.type()
                        self.expect(")")
                    cases.append({"name": cname, "type": ty, "doc": cdoc})
                    self.eat(",")
                iface["types"].append({"kind": "variant", "name": n, "doc": doc, "cases": cases})
            elif kw == "resource":
                self.next()
                n = self.ident()
                res = {"name": n, "doc": doc, "methods": []}
                if self.eat("{"):
                    while not self.eat("}"):
                        self.skip_annotations()
                        mdoc = self.peek()[2]
                        if self.peek()[1] == "constructor":
                            raise WitError("resource constructors are not supported")
                        mname = self.ident()
                        self.expect(":")
                        static = self.eat("static")
                        res["methods"].append(self.func(mname, mdoc, static))
                else:
                    self.expect(";")
                iface["resources"].append(res)
            elif kw in ("flags", "tuple", "type-alias"):
                raise WitError("unsupported WIT construct %r in interface %s" % (kw, name))
            else:
                fname = self.ident()
                self.expect(":")
                iface["funcs"].append(self.func(fname, doc))
        return iface


def parse_dir(wit_dir):
    interfaces = []
    package = None
    for fn in sorted(os.listdir(wit_dir)):
        if not fn.endswith(".wit"):
            continue
        with open(os.path.join(wit_dir, fn), encoding="utf-8") as f:
            p = Parser(f.read())
        while p.peek()[0] != "eof":
            p.skip_annotations()
            kw = p.peek()[1]
            if kw == "package":
                p.next()
                parts = []
                while p.peek()[1] != ";":
                    parts.append(p.next()[1])
                p.expect(";")
                package = "".join(parts)
            elif kw == "interface":
                interfaces.append(p.interface())
            elif kw == "world":
                p.next()
                p.ident()
                p.expect("{")
                depth = 1
                while depth:
                    t = p.next()
                    depth += (t[1] == "{") - (t[1] == "}")
            else:
                raise WitError("%s: unexpected %r" % (fn, kw))
    return package, interfaces


# ---------------------------------------------------------------------------------------------------------------
# Naming

def camel(name):
    return "".join(p[:1].upper() + p[1:] for p in re.split(r"[-_]", name))


def lower_camel(name):
    c = camel(name)
    return c[:1].lower() + c[1:]


def const(name):
    return re.sub(r"[-]", "_", name).upper()


def field_name(name):
    n = lower_camel(name)
    return n + "_" if n in JAVA_RESERVED else n


def pkg_of(iface):
    return PKG + "." + iface.replace("-", "")


def javadoc(doc, indent=""):
    lines = [d for d in doc]
    if not lines:
        return ""
    body = "\n".join(indent + " * " + l.replace("*/", "* /").replace("<", "&lt;").replace(">", "&gt;").replace("@", "{@literal @}").rstrip() for l in lines)
    return indent + "/**\n" + body + "\n" + indent + " */\n"


HEADER = """// Generated by scripts/gen-java.py from the blockheads:api WIT package. Do not edit.
// SPDX-License-Identifier: MIT OR Apache-2.0
"""


# ---------------------------------------------------------------------------------------------------------------
# Model

class Model:
    def __init__(self, interfaces):
        self.ifaces = {i["name"]: i for i in interfaces}
        # (iface, name) -> definition
        self.defs = {}
        for i in interfaces:
            for t in i["types"]:
                self.defs[(i["name"], t["name"])] = t
            for r in i["resources"]:
                self.defs[(i["name"], r["name"])] = dict(r, kind="resource")
        self.scope = {}
        for i in interfaces:
            sc = {}
            for key in self.defs:
                if key[0] == i["name"]:
                    sc[key[1]] = key
            for src, names in i["uses"]:
                for n, alias in names:
                    if (src, n) not in self.defs:
                        raise WitError("%s uses unknown %s.%s" % (i["name"], src, n))
                    sc[alias] = (src, n)
            self.scope[i["name"]] = sc

    def lookup(self, iface, name):
        key = self.scope[iface].get(name)
        if key is None:
            raise WitError("%s: unknown type %s" % (iface, name))
        return key

    def resolve(self, iface, t):
        """Resolves aliases: returns a type tree whose 'named' leaves point at record/enum/variant/resource defs."""
        k = t[0]
        if k == "prim":
            return t
        if k == "list":
            return ("list", self.resolve(iface, t[1]))
        if k == "option":
            return ("option", self.resolve(iface, t[1]))
        if k == "ref":
            return self.resolve(iface, t[1])
        if k == "tuple":
            return ("tuple", [self.resolve(iface, x) for x in t[1]])
        if k == "result":
            return ("result", None if t[1] is None else self.resolve(iface, t[1]),
                    None if t[2] is None else self.resolve(iface, t[2]))
        key = self.lookup(iface, t[1])
        d = self.defs[key]
        if key == ("types", "value"):
            return ("value",)
        if d["kind"] == "alias":
            return self.resolve(key[0], d["type"])
        return ("def", key)


# ---------------------------------------------------------------------------------------------------------------
# Java type and codec expressions

class Gen:
    def __init__(self, model):
        self.m = model
        self.files = {}

    def jclass(self, key):
        return "%s.%s" % (pkg_of(key[0]), camel(key[1]))

    def jtype(self, t, boxed=False):
        k = t[0]
        if k == "prim":
            p = PRIMS[t[1]]
            return p[1] if boxed else p[0]
        if k == "value":
            return "Object"
        if k == "list":
            if t[1] == ("prim", "u8"):
                return "byte[]"
            return "java.util.List<%s>" % self.jtype(t[1], True)
        if k == "option":
            return self.jtype(t[1], True)
        if k == "tuple":
            return "java.util.List<Object>"
        if k == "def":
            return self.jclass(t[1])
        raise WitError("no Java type for %r" % (t,))

    def dec(self, t, e):
        """Java expression decoding the wire object `e` into the Java type."""
        k = t[0]
        if k == "prim":
            return PRIMS[t[1]][2] % e
        if k == "value":
            return "Wire.value(%s, b)" % e
        if k == "list":
            if t[1] == ("prim", "u8"):
                return "Wire.bytes(%s)" % e
            return "Wire.list(%s, x -> %s)" % (e, self.dec(t[1], "x"))
        if k == "option":
            return "Wire.optIn(%s, x -> %s)" % (e, self.dec(t[1], "x"))
        if k == "tuple":
            return "Wire.list(%s, x -> x)" % e
        if k == "def":
            d = self.m.defs[t[1]]
            if d["kind"] == "resource":
                return "new %s(b, Wire.text(%s))" % (self.jclass(t[1]), e)
            return "%s.fromWire(%s, b)" % (self.jclass(t[1]), e)
        raise WitError("cannot decode %r" % (t,))

    def enc(self, t, e):
        """Java expression encoding the Java value `e` as a wire object."""
        k = t[0]
        if k == "prim":
            return PRIMS[t[1]][3] % e
        if k == "value":
            return "Wire.valueOut(%s, b)" % e
        if k == "list":
            if t[1] == ("prim", "u8"):
                return e
            return "Wire.listOut(%s, x -> %s)" % (e, self.enc(t[1], "x"))
        if k == "option":
            return "Wire.optOut(%s, x -> %s)" % (e, self.enc(t[1], "x"))
        if k == "tuple":
            return e
        if k == "def":
            d = self.m.defs[t[1]]
            if d["kind"] == "resource":
                return "%s.key()" % e
            return "%s.toWire(b)" % e
        raise WitError("cannot encode %r" % (t,))

    # -- files -------------------------------------------------------------------------------------------------

    def emit(self, iface, cls, body):
        imports = "import %s.Wire;\n\n" % RT if "Wire." in body else ""
        path = os.path.join(*(pkg_of(iface).split(".") + [cls + ".java"]))
        self.files[path] = HEADER + "package %s;\n\n%s" % (pkg_of(iface), imports) + body

    # -- types -------------------------------------------------------------------------------------------------

    def gen_enum(self, iface, d):
        cls = camel(d["name"])
        cases = ",\n".join(
            javadoc(c["doc"], "    ") + '    %s("%s")' % (const(c["name"]), c["name"]) for c in d["cases"])
        body = javadoc(d["doc"]) + """public enum %(cls)s {
%(cases)s;

    /** The WIT name of the case. */
    public final String wire;

    %(cls)s(String wire) {
        this.wire = wire;
    }

    public static %(cls)s fromWire(Object o, %(rt)s.Bridge b) {
        String s = Wire.text(o);
        for (%(cls)s c : values()) {
            if (c.wire.equals(s)) {
                return c;
            }
        }
        throw new IllegalArgumentException("unknown %(name)s: " + s);
    }

    public Object toWire(%(rt)s.Bridge b) {
        return wire;
    }
}
""" % {"cls": cls, "cases": cases, "rt": RT, "name": d["name"]}
        self.emit(iface, cls, body)

    def gen_record(self, iface, d):
        cls = camel(d["name"])
        fields = []
        for f in d["fields"]:
            ty = self.m.resolve(iface, f["type"])
            fields.append((field_name(f["name"]), f["name"], ty, f["doc"]))
        decl = "".join(javadoc(doc, "    ") + "    public %s %s;\n" % (self.jtype(ty), jn) for jn, _, ty, doc in fields)
        args = ", ".join("%s %s" % (self.jtype(ty), jn) for jn, _, ty, _ in fields)
        assign = "".join("        this.%s = %s;\n" % (jn, jn) for jn, _, _, _ in fields)
        decode = ",\n                ".join(self.dec(ty, 'm.get("%s")' % wn) for _, wn, ty, _ in fields)
        encode = "".join('        m.put("%s", %s);\n' % (wn, self.enc(ty, "this." + jn)) for jn, wn, ty, _ in fields)
        body = javadoc(d["doc"]) + """public final class %(cls)s {
%(decl)s
    public %(cls)s(%(args)s) {
%(assign)s    }

    public static %(cls)s fromWire(Object o, %(rt)s.Bridge b) {
        java.util.Map<String, Object> m = Wire.map(o);
        return new %(cls)s(%(decode)s);
    }

    public Object toWire(%(rt)s.Bridge b) {
        java.util.Map<String, Object> m = new java.util.LinkedHashMap<String, Object>();
%(encode)s        return m;
    }
}
""" % {"cls": cls, "decl": decl, "args": args, "assign": assign, "decode": decode, "encode": encode, "rt": RT}
        self.emit(iface, cls, body)

    def gen_variant(self, iface, d):
        cls = camel(d["name"])
        parts, dec_cases, enc_cases = [], [], []
        for c in d["cases"]:
            ccls = camel(c["name"])
            ty = self.m.resolve(iface, c["type"]) if c["type"] else None
            doc = javadoc(c["doc"], "    ")
            if ty is None:
                parts.append(doc + """    public static final class %(c)s extends %(cls)s {
        @Override
        public String tag() {
            return "%(w)s";
        }

        @Override
        public Object payload() {
            return null;
        }

        @Override
        public String toString() {
            return "%(w)s";
        }
    }
""" % {"c": ccls, "cls": cls, "w": c["name"]})
                dec_cases.append('        if (tag.equals("%s")) {\n            return new %s();\n        }\n' % (c["name"], ccls))
                enc_cases.append('        if (this instanceof %s) {\n            m.put("%s", null);\n        }\n' % (ccls, c["name"]))
            else:
                jt = self.jtype(ty)
                parts.append(doc + """    public static final class %(c)s extends %(cls)s {
        public %(jt)s value;

        public %(c)s(%(jt)s value) {
            this.value = value;
        }

        @Override
        public String tag() {
            return "%(w)s";
        }

        @Override
        public Object payload() {
            return value;
        }

        @Override
        public String toString() {
            return "%(w)s: " + value;
        }
    }
""" % {"c": ccls, "cls": cls, "jt": jt, "w": c["name"]})
                dec_cases.append('        if (tag.equals("%s")) {\n            return new %s(%s);\n        }\n' % (
                    c["name"], ccls, self.dec(ty, "payload")))
                enc_cases.append('        if (this instanceof %s) {\n            m.put("%s", %s);\n        }\n' % (
                    ccls, c["name"], self.enc(ty, "((%s) this).value" % ccls)))
        body = javadoc(d["doc"]) + """public abstract class %(cls)s {
    %(cls)s() {
    }

    /** The WIT name of the case. */
    public abstract String tag();

    /** The payload of the case, or null when it has none. */
    public abstract Object payload();

%(parts)s
    public static %(cls)s fromWire(Object o, %(rt)s.Bridge b) {
        java.util.Map<String, Object> m = Wire.map(o);
        if (m.size() != 1) {
            throw new IllegalArgumentException("a %(name)s is a one-entry map");
        }
        java.util.Map.Entry<String, Object> e = m.entrySet().iterator().next();
        String tag = e.getKey();
        Object payload = e.getValue();
%(dec)s        throw new IllegalArgumentException("unknown %(name)s case: " + tag);
    }

    public Object toWire(%(rt)s.Bridge b) {
        java.util.Map<String, Object> m = new java.util.LinkedHashMap<String, Object>();
%(enc)s        return m;
    }
}
""" % {"cls": cls, "parts": "\n".join(parts), "dec": "".join(dec_cases), "enc": "".join(enc_cases),
       "rt": RT, "name": d["name"]}
        self.emit(iface, cls, body)

    def gen_resource(self, iface, d):
        cls = camel(d["name"])
        methods = [self.method(iface, f, "%s.%s.%s" % (iface, d["name"], f["name"]), self_key=True)
                   for f in d["methods"]]
        body = javadoc(d["doc"]) + """public class %(cls)s {
    private final %(rt)s.Bridge b;
    private final String key;

    public %(cls)s(%(rt)s.Bridge b, String key) {
        this.b = b;
        this.key = key;
    }

    /** The key that identifies the host object. */
    public String key() {
        return key;
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof %(cls)s && ((%(cls)s) o).key.equals(key);
    }

    @Override
    public int hashCode() {
        return key.hashCode();
    }

    @Override
    public String toString() {
        return "%(name)s(" + key + ")";
    }

%(methods)s}
""" % {"cls": cls, "rt": RT, "name": d["name"], "methods": "\n".join(methods)}
        self.emit(iface, cls, body)

    # -- functions ---------------------------------------------------------------------------------------------

    def err_class(self, t):
        """The exception class for a result's error type, generated on first use."""
        if t is None or t[0] != "def":
            return RT + ".HostException"
        key = t[1]
        cls = camel(key[1]) + "Exception"
        path = os.path.join(*(pkg_of(key[0]).split(".") + [cls + ".java"]))
        if path not in self.files:
            self.files[path] = HEADER + "package %s;\n\nimport %s.Wire;\n\n" % (pkg_of(key[0]), RT) + """/** Thrown when a host function returns {@code err(%(name)s)}. */
public final class %(cls)s extends RuntimeException {
    private static final long serialVersionUID = 1L;

    /** The error value. */
    public final transient %(etype)s error;

    public %(cls)s(%(etype)s error) {
        super(Wire.describe(error));
        this.error = error;
    }
}
""" % {"cls": cls, "etype": self.jclass(key), "name": key[1]}
        return "%s.%s" % (pkg_of(key[0]), cls)

    def return_type(self, ret):
        if ret is None:
            return "void"
        if ret[0] == "result":
            return self.jtype(ret[1]) if ret[1] else "void"
        return self.jtype(ret)

    def method(self, iface, f, wire_name, self_key=False, declare_only=False, override=False):
        params = [(field_name(n), self.m.resolve(iface, t)) for n, t in f["params"]]
        ret = self.m.resolve(iface, f["ret"]) if f["ret"] else None
        jret = self.return_type(ret)
        sig = "%s %s(%s)" % (jret, field_name(f["name"]), ", ".join("%s %s" % (self.jtype(t), n) for n, t in params))
        doc = javadoc(f["doc"], "    ")
        if declare_only:
            return doc + "    %s;\n" % sig
        enc_args = (["key"] if self_key else []) + [self.enc(t, n) for n, t in params]
        call = 'b.call("%s"%s)' % (wire_name, "".join(", " + a for a in enc_args))
        if ret is None:
            body = "        %s;\n" % call
        elif ret[0] == "result":
            ok, err = ret[1], ret[2]
            ecls = self.err_class(err)
            if err is not None and err[0] == "def":
                mk = "new %s(%s)" % (ecls, self.dec(err, "x"))
            else:
                mk = "new %s(Wire.describe(x))" % ecls
            unwrap = "Wire.unwrap(%s, x -> %s)" % (call, mk)
            body = ("        %s;\n" % unwrap) if ok is None else \
                "        Object r = %s;\n        return %s;\n" % (unwrap, self.dec(ok, "r"))
        else:
            body = "        Object r = %s;\n        return %s;\n" % (call, self.dec(ret, "r"))
        return doc + ("    @Override\n" if override else "") + "    public %s {\n%s    }\n" % (sig, body)

    def gen_interface(self, i):
        name = i["name"]
        cls = camel(name)
        if name == "guest":
            return self.gen_guest(i)
        if i["funcs"]:
            decls = [self.method(name, f, "", declare_only=True) for f in i["funcs"]]
            self.emit(name, cls, javadoc(i["doc"]) + "public interface %s {\n%s}\n" % (cls, "\n".join(decls)))
            impls = [self.method(name, f, "%s.%s" % (name, f["name"]), override=True) for f in i["funcs"]]
            self.emit(name, cls + "Impl", """/** Calls the host to implement {@link %(cls)s}. */
public final class %(cls)sImpl implements %(cls)s {
    private final %(rt)s.Bridge b;

    public %(cls)sImpl(%(rt)s.Bridge b) {
        this.b = b;
    }

%(methods)s}
""" % {"cls": cls, "rt": RT, "methods": "\n".join(impls)})
        self.gen_types(i)

    def gen_types(self, i):
        name = i["name"]
        for d in i["types"]:
            if d["kind"] == "enum":
                self.gen_enum(name, d)
            elif d["kind"] == "record":
                self.gen_record(name, d)
            elif d["kind"] == "variant":
                self.gen_variant(name, d)
        for r in i["resources"]:
            self.gen_resource(name, r)

    def gen_guest(self, i):
        name = "guest"
        sigs, cases = [], []
        for f in i["funcs"]:
            params = [(field_name(n), self.m.resolve(name, t)) for n, t in f["params"]]
            ret = self.m.resolve(name, f["ret"]) if f["ret"] else None
            throws = " throws Exception" if ret is not None and ret[0] == "result" else ""
            mname = field_name(f["name"])
            sigs.append(javadoc(f["doc"], "    ") + "    %s %s(%s)%s;\n" % (
                self.return_type(ret), mname, ", ".join("%s %s" % (self.jtype(t), n) for n, t in params), throws))
            argdec = ", ".join(self.dec(t, "args.get(%d)" % k) for k, (n, t) in enumerate(params))
            call = "g.%s(%s)" % (mname, argdec)
            if ret is None or (ret[0] == "result" and ret[1] is None):
                body = "            %s;\n            return null;" % call
            else:
                body = "            return %s;" % self.enc(ret[1] if ret[0] == "result" else ret, call)
            cases.append('        if (fn.equals("%s")) {\n%s\n        }\n' % (f["name"], body))
        self.emit(name, "Guest", javadoc(i["doc"]) + "public interface Guest {\n%s}\n" % "\n".join(sigs))
        self.emit(name, "GuestDispatch", """/** Decodes a call of the host into a {@link Guest} method and encodes its result. */
public final class GuestDispatch {
    private GuestDispatch() {
    }

    /** Calls the WIT export {@code fn}; {@code args} is the decoded argument list. Exceptions propagate. */
    public static Object invoke(Guest g, %(rt)s.Bridge b, String fn, java.util.List<Object> args) throws Exception {
%(cases)s        throw new IllegalArgumentException("unknown guest function: " + fn);
    }
}
""" % {"rt": RT, "cases": "\n".join(cases)})
        self.gen_types(i)

    def gen_event_handlers(self):
        """`EventHandlers`: `onXxx(Handler<XxxEvent>)` for every case of the `event` variant that carries a record."""
        ev = self.m.defs.get(("events", "event"))
        kinds = self.m.defs.get(("events", "event-kind"))
        if not ev or not kinds or ("types", "priority") not in self.m.defs:
            return
        kind_names = {c["name"] for c in kinds["cases"]}
        pk = pkg_of("events")
        pr = self.jclass(("types", "priority"))
        methods = []
        for c in ev["cases"]:
            if c["type"] is None or c["name"] not in kind_names:
                continue
            rec = self.m.resolve("events", c["type"])
            if rec[0] != "def":
                continue
            args = {"l": "on" + camel(c["name"]), "rcls": self.jclass(rec[1]), "pk": pk, "pr": pr,
                    "k": const(c["name"]), "w": c["name"]}
            methods.append("""    /** Subscribes to {@code %(w)s} events with the default priority. */
    default int %(l)s(club.blockheads.mod.Handler<%(rcls)s> h) {
        return onEvent(%(pk)s.EventKind.%(k)s, %(pr)s.NORMAL, false, h);
    }

    /** Subscribes to {@code %(w)s} events. */
    default int %(l)s(%(pr)s priority, boolean ignoreCancelled, club.blockheads.mod.Handler<%(rcls)s> h) {
        return onEvent(%(pk)s.EventKind.%(k)s, priority, ignoreCancelled, h);
    }
""" % args)
        self.emit("events", "EventHandlers", """/** Typed shortcuts to subscribe to each built-in event. */
public interface EventHandlers {
    /** Subscribes {@code h} to an event kind; the handler receives the event record of that kind. */
    int onEvent(%(pk)s.EventKind kind, %(pr)s priority, boolean ignoreCancelled, club.blockheads.mod.Handler<?> h);

%(methods)s}
""" % {"pk": pk, "pr": pr, "methods": "\n".join(methods)})

    def run(self):
        for i in self.m.ifaces.values():
            self.gen_interface(i)
        self.gen_event_handlers()
        return self.files


def main(argv):
    check = "--check" in argv
    args = [a for a in argv if not a.startswith("--")]
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    wit_dir = args[0] if args else os.path.join(root, "wit", "blockheads-api")
    out_dir = args[1] if len(args) > 1 else os.path.join(root, "sdk", "java", "src")
    package, interfaces = parse_dir(wit_dir)
    files = Gen(Model(interfaces)).run()
    gen_root = os.path.join(out_dir, *PKG.split("."))
    existing = set()
    if os.path.isdir(gen_root):
        for dp, _, fns in os.walk(gen_root):
            for fn in fns:
                if fn.endswith(".java"):
                    existing.add(os.path.relpath(os.path.join(dp, fn), out_dir))
    problems = []
    for rel, text in sorted(files.items()):
        path = os.path.join(out_dir, rel)
        old = None
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                old = f.read()
        if old != text:
            problems.append(rel)
            if not check:
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "w", encoding="utf-8") as f:
                    f.write(text)
    stale = sorted(existing - set(files))
    for rel in stale:
        problems.append(rel + " (stale)")
        if not check:
            os.remove(os.path.join(out_dir, rel))
    if check and problems:
        print("generated Java is out of date (run scripts/gen-java.py): " + ", ".join(problems), file=sys.stderr)
        return 1
    print("%s: %d files for %s" % ("checked" if check else "wrote", len(files), package))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
