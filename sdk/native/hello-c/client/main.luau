-- The work of this mod is done by client/native/armeabi-v7a/libnative_hello.so (see src/hello.c).
local mod = {}
function mod.on_enable()
    bh.log.info("native-hello: the native library installs the hooks")
end
return mod
