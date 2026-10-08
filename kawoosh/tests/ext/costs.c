/* The cost rows of docs/design/native.md Decision 6 (`tests/lua_costs.rs`,
 * `native_buffer_access`): the buffer's text read N times and N edits
 * applied, each through the data route (kw_call) and the typed one
 * (kw_buf_text, kw_buf_edits); the C side's own clock echoed. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "kawoosh.h"

uint32_t kw_ext_abi(void) { return KW_ABI_VERSION; }

static double now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static void echo(KwCtx *ctx, const char *s) { kw_do(ctx, "echo", kw_str(s)); }

/* The command's count: its first argument, else 1. */
static long count_of(const KuiValue *args) {
    const KuiValue *c = kui_value_at(args, 0);
    const KuiValue *list = c ? kui_value_get(c, KUI_STR("args")) : NULL;
    KuiStr s;
    if (list && kui_value_as_str(kui_value_at(list, 0), &s)) return strtol((const char *)s.ptr, NULL, 10);
    return 1;
}

static KuiValue *text_data(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    long n = count_of(args);
    size_t len = 0;
    double t = now_ms();
    for (long i = 0; i < n; i++) {
        KuiValue *v = kw_call(ctx, KUI_STR("buf.text"), NULL);
        KuiStr s;
        if (v && kui_value_as_str(v, &s)) len = s.len;
        kui_value_free(v);
    }
    char msg[96];
    snprintf(msg, sizeof msg, "%.3f ms a read of %zu bytes", (now_ms() - t) / n, len);
    echo(ctx, msg);
    return NULL;
}

static KuiValue *text_typed(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    long n = count_of(args);
    size_t len = 0;
    double t = now_ms();
    for (long i = 0; i < n; i++) {
        KuiStr s;
        if (kw_buf_text(ctx, 0, &s)) len = s.len;
    }
    char msg[96];
    snprintf(msg, sizeof msg, "%.3f ms a read of %zu bytes", (now_ms() - t) / n, len);
    echo(ctx, msg);
    return NULL;
}

static KuiValue *edits_data(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    long n = count_of(args);
    double t = now_ms();
    KuiValue *list = kui_value_list();
    for (long i = 0; i < n; i++)
        kui_value_list_push(list, kw_list(kw_int(i * 64), kw_int(i * 64), kw_str("x")));
    bool ok = kw_do(ctx, "buf.edits", list);
    char msg[96];
    snprintf(msg, sizeof msg, "%.3f ms to queue %ld edits%s", now_ms() - t, n, ok ? "" : " (refused)");
    echo(ctx, msg);
    return NULL;
}

static KuiValue *edits_typed(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    long n = count_of(args);
    double t = now_ms();
    KwEdit *edits = malloc((size_t)n * sizeof *edits);
    for (long i = 0; i < n; i++) edits[i] = (KwEdit){(uint64_t)(i * 64), (uint64_t)(i * 64), KUI_STR("x")};
    bool ok = kw_buf_edits(ctx, 0, edits, (size_t)n);
    free(edits);
    char msg[96];
    snprintf(msg, sizeof msg, "%.3f ms to queue %ld edits%s", now_ms() - t, n, ok ? "" : " (refused)");
    echo(ctx, msg);
    return NULL;
}

static void command(KwCtx *ctx, const char *name, KwFn fn) {
    kw_do(ctx, "command", kw_str(name), kw_fn(ctx, fn, NULL),
          kw_map("args", kw_list(kw_str("text")), NULL));
}

void *kw_ext_init(KwCtx *ctx) {
    command(ctx, "ctext_data", text_data);
    command(ctx, "ctext_typed", text_typed);
    command(ctx, "cedits_data", edits_data);
    command(ctx, "cedits_typed", edits_typed);
    return NULL;
}
