/* SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * native-hello: an example native client mod. It installs one hook of each kind through libbhmod and logs a summary
 * every ten seconds:
 *
 *   - an Objective-C swizzle on -[World update:accurateDT:pinchScale:dragInProgress:] (counts world updates),
 *   - an inline hook on the exported game function imageTypeForItemType(ItemType, bool) (counts lookups and calls
 *     the original through the trampoline),
 *   - an import hook on the game's __wrap_glBindTexture (counts texture binds).
 *
 * Build: scripts/build-native-example.sh (needs the Android NDK). No C runtime is used: the library is built with
 * -nostdlib, so it loads in the Android process and in the PC runner's guest alike.
 */
#include "bhmod.h"

static const bhmod_api_t *api;
static void (*orig_update)(void *self, void *cmd, int dt, int accurate_dt, int scale, int drag);
static int (*orig_image_type)(unsigned type, int flag);
static void (*orig_bind)(unsigned target, unsigned name);
static unsigned updates, lookups, binds;

static char *put_uint(char *p, unsigned v) {
  char tmp[11];
  int n = 0;
  do {
    tmp[n++] = (char)('0' + v % 10);
    v /= 10;
  } while (v);
  while (n) *p++ = tmp[--n];
  return p;
}

static char *put_str(char *p, const char *s) {
  while (*s) *p++ = *s++;
  return p;
}

static void summary(void) {
  char line[96];
  char *p = put_str(line, "world updates ");
  p = put_uint(p, updates);
  p = put_str(p, ", item lookups ");
  p = put_uint(p, lookups);
  p = put_str(p, ", texture binds ");
  p = put_uint(p, binds);
  *p = 0;
  api->log(api, BHMOD_LOG_INFO, line);
}

/* The arguments are passed through unchanged (floats travel in core registers: the game is soft-float ARM). */
static void my_update(void *self, void *cmd, int dt, int accurate_dt, int scale, int drag) {
  if (++updates == 1) api->log(api, BHMOD_LOG_INFO, "first world update through the hook");
  if (updates % 600 == 0) summary();
  orig_update(self, cmd, dt, accurate_dt, scale, drag);
}

static int my_image_type(unsigned type, int flag) {
  if (++lookups == 1) api->log(api, BHMOD_LOG_INFO, "first item lookup through the hook");
  return orig_image_type(type, flag);
}

static void my_bind(unsigned target, unsigned name) {
  if (++binds == 1) api->log(api, BHMOD_LOG_INFO, "first texture bind through the hook");
  orig_bind(target, name);
}

static void report(const char *what, int result) {
  char line[96];
  char *p = put_str(line, what);
  p = put_str(p, result > 0 ? ": hooked" : ": failed - ");
  if (result <= 0) p = put_str(p, api->last_error(api));
  *p = 0;
  api->log(api, result > 0 ? BHMOD_LOG_INFO : BHMOD_LOG_WARN, line);
}

int bhmod_native_init(const bhmod_api_t *a) {
  api = a;
  if (a->abi != BHMOD_ABI_VERSION) return 1;
  api->log(api, BHMOD_LOG_INFO, "hello from native code");
  report("world update", api->hook_objc(api, "World", "update:accurateDT:pinchScale:dragInProgress:", 0,
                                         (void *)my_update, (void **)&orig_update));
  report("imageTypeForItemType",
         api->hook_symbol(api, "libApplication.so", "_Z20imageTypeForItemType8ItemTypea", (void *)my_image_type,
                          (void **)&orig_image_type));
  report("glBindTexture", api->hook_import(api, "libApplication.so", "__wrap_glBindTexture", (void *)my_bind,
                                           (void **)&orig_bind));
  return 0;
}

void bhmod_native_shutdown(void) { summary(); }
