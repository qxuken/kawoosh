/* The door's edges, probed from kw_ext_init and reported as the lines of
 * a scratch named `probe`: a door that does not exist, arguments that
 * are not a list, a door that answers two values, a Lua error, a null
 * context, the protocol; and a command whose body is a handle. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "kawoosh.h"

uint32_t kw_ext_abi(void) { return KW_ABI_VERSION; }

static char report[4096];
static size_t rlen;

static void line(const char *head, KuiStr s) {
    rlen += (size_t)snprintf(report + rlen, sizeof report - rlen, "%s%.*s\n", head, (int)s.len,
                             (const char *)s.ptr);
}

static KuiStr error_of(KwCtx *ctx) {
    KuiStr e = KUI_STR("(no error)");
    kw_error(ctx, &e);
    return e;
}

static KuiValue *one(KuiValue *v) {
    KuiValue *a = kui_value_list();
    kui_value_list_push(a, v);
    return a;
}

static KuiValue *ran(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    /* args[0] is the command's ctx map; its `count` is what `:3probe`
     * gave, 1 without. */
    int64_t count = -1;
    const KuiValue *c = kui_value_at(args, 0);
    kui_value_as_int(kui_value_get(c, KUI_STR("count")), &count);
    char msg[64];
    snprintf(msg, sizeof msg, "probe ran, count %lld", (long long)count);
    kw_do(ctx, "echo", kw_str(msg));
    return kw_int(7); /* the host frees it */
}

void *kw_ext_init(KwCtx *ctx) {
    rlen = 0;
    KuiStr e;
    /* no such door */
    KuiValue *r = kw_call(ctx, KUI_STR("nope"), NULL);
    line(r ? "nope: answered" : "nope: ", r ? KUI_STR("") : error_of(ctx));
    kui_value_free(r);
    /* arguments that are a scalar, then a map */
    KuiValue *scalar = kui_value_int(1);
    r = kw_call(ctx, KUI_STR("echo"), scalar);
    line("scalar: ", r ? KUI_STR("answered") : error_of(ctx));
    kui_value_free(r);
    kui_value_free(scalar);
    KuiValue *map = kui_value_map();
    kui_value_map_set(map, KUI_STR("x"), kui_value_int(1));
    r = kw_call(ctx, KUI_STR("echo"), map);
    line("map: ", r ? KUI_STR("answered") : error_of(ctx));
    kui_value_free(r);
    kui_value_free(map);
    /* a door answering one string */
    KuiValue *a = one(kui_value_str(KUI_STR("/a/b.txt")));
    r = kw_call(ctx, KUI_STR("fs.basename"), a);
    KuiStr s = KUI_STR("(not a string)");
    if (r) kui_value_as_str(r, &s);
    line("basename: ", r ? s : error_of(ctx));
    kui_value_free(r);
    kui_value_free(a);
    /* a door answering `nil, why` has refused: NULL and the reason */
    a = kui_value_list();
    kui_value_list_push(a, kui_value_str(KUI_STR("")));
    kui_value_list_push(a, kui_value_str(KUI_STR("nowhere")));
    r = kw_call(ctx, KUI_STR("_extension"), a);
    line("refused: ", r ? KUI_STR("answered") : error_of(ctx));
    kui_value_free(r);
    kui_value_free(a);
    /* the error is the last failing call's: cleared by a success */
    a = one(kui_value_str(KUI_STR("/a/b.txt")));
    kui_value_free(kw_call(ctx, KUI_STR("fs.basename"), a));
    kui_value_free(a);
    line("after a success: ", kw_error(ctx, &e) ? KUI_STR("stale error") : KUI_STR("no error"));
    /* a Lua error: open_scratch given a number */
    a = one(kui_value_int(1));
    r = kw_call(ctx, KUI_STR("buf.open_scratch"), a);
    e = error_of(ctx);
    line("lua error: ", r ? KUI_STR("answered") : (e.len > 20 ? (KuiStr){e.ptr, 20} : e));
    kui_value_free(r);
    kui_value_free(a);
    /* a null context */
    r = kw_call(NULL, KUI_STR("echo"), NULL);
    line(r ? "null ctx: answered" : "null ctx: NULL", KUI_STR(""));
    kui_value_free(r);
    char p[32];
    snprintf(p, sizeof p, "protocol: %u", kw_protocol(ctx));
    line(p, KUI_STR(""));
    /* a command whose body is a handle */
    line("command: ", kw_do(ctx, "command", kw_str("probe"), kw_fn(ctx, ran, NULL))
                          ? KUI_STR("registered")
                          : error_of(ctx));
    /* a null function is no handle */
    r = kw_fn(ctx, NULL, NULL);
    line("null fn: ", r ? KUI_STR("a handle") : error_of(ctx));
    kui_value_free(r);
    /* the same fn and user twice is one handle */
    KuiValue *h1 = kw_fn(ctx, ran, NULL), *h2 = kw_fn(ctx, ran, NULL), *h3 = kw_fn(ctx, ran, report);
    int64_t i1 = -1, i2 = -2, i3 = -3;
    kui_value_as_int(kui_value_get(h1, KUI_STR("kw_fn")), &i1);
    kui_value_as_int(kui_value_get(h2, KUI_STR("kw_fn")), &i2);
    kui_value_as_int(kui_value_get(h3, KUI_STR("kw_fn")), &i3);
    line(i1 == i2 && i1 != i3 ? "handles: same fn same user is one, another user another"
                              : "handles: not deduplicated", KUI_STR(""));
    kui_value_free(h1);
    kui_value_free(h2);
    kui_value_free(h3);

    if (rlen > 0) rlen--;
    kw_do(ctx, "buf.open_scratch", kw_map("name", kw_str("probe"), "text", kw_strn(report, rlen), NULL));
    return NULL;
}
