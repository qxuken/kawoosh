/* A native pane and a thread (docs/design/native.md Decisions 4 and 5):
 * both families of entry points in one library. `kw_ext_init` registers
 * the view `cpanel` as this extension's own and two commands - `cpanel`
 * opens it, `cwake` starts a thread that comes back through kw_wake;
 * kui's `kui_ext_view` draws the pane, a row whose clicks come back to
 * `kui_ext_on_event` and are counted on the next frame. */
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "kawoosh.h"

/* -- the kawoosh half ------------------------------------------------- */

uint32_t kw_ext_abi(void) { return KW_ABI_VERSION; }
const char *kw_ext_name(void) { return "panel"; }

static KuiValue *one_str(const char *s) {
    KuiValue *a = kui_value_list();
    kui_value_list_push(a, kui_value_str(KUI_STR(s)));
    return a;
}

static KuiValue *open_pane(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)args;
    KuiValue *a = one_str("cpanel");
    kui_value_free(kw_call(ctx, KUI_STR("view_open"), a));
    kui_value_free(a);
    return NULL;
}

/* The thread's end: back on the UI thread, with a context of its own. */
static KuiValue *woke(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)args;
    char msg[64];
    snprintf(msg, sizeof msg, "woke from a thread, namespace %s",
             kw_namespace(ctx, &(KuiStr){0}) ? "known" : "none");
    KuiValue *a = one_str(msg);
    kui_value_free(kw_call(ctx, KUI_STR("echo"), a));
    kui_value_free(a);
    free(user);
    return NULL;
}

static void *work(void *arg) {
    usleep(20 * 1000);
    kw_wake(woke, arg);
    return NULL;
}

/* A thread using the call's context while the call waits: refused,
 * with nothing touched. */
static void *misuse(void *arg) {
    KwCtx *ctx = arg;
    KuiValue *r = kw_call(ctx, KUI_STR("echo"), NULL);
    KuiStr e;
    bool had_error = kw_error(ctx, &e);
    return (void *)(intptr_t)(r == NULL && !had_error);
}

static KuiValue *thread_misuse(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)args;
    pthread_t t;
    void *out = NULL;
    pthread_create(&t, NULL, misuse, ctx);
    pthread_join(t, &out);
    KuiValue *a = one_str(out ? "off-thread call refused" : "off-thread call answered");
    kui_value_free(kw_call(ctx, KUI_STR("echo"), a));
    kui_value_free(a);
    return NULL;
}

static KuiValue *start_thread(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)ctx;
    (void)args;
    pthread_t t;
    pthread_create(&t, NULL, work, malloc(1));
    pthread_detach(t);
    return NULL;
}

static void command(KwCtx *ctx, const char *name, KwFn fn) {
    KuiValue *a = one_str(name);
    kui_value_list_push(a, kw_fn(ctx, fn, NULL));
    kui_value_free(kw_call(ctx, KUI_STR("command"), a));
    kui_value_free(a);
}

void *kw_ext_init(KwCtx *ctx) {
    KuiStr ns;
    if (!kw_namespace(ctx, &ns)) return NULL;
    /* kawoosh.view("cpanel", nil, nil, { native = NS }) */
    KuiValue *a = one_str("cpanel");
    kui_value_list_push(a, kui_value_null());
    kui_value_list_push(a, kui_value_null());
    KuiValue *opts = kui_value_map();
    kui_value_map_set(opts, KUI_STR("native"), kui_value_str(ns));
    kui_value_list_push(a, opts);
    kui_value_free(kw_call(ctx, KUI_STR("view"), a));
    kui_value_free(a);
    command(ctx, "cpanel", open_pane);
    command(ctx, "cwake", start_thread);
    command(ctx, "cmisuse", thread_misuse);
    return NULL;
}

void kw_ext_free(void *user) { (void)user; }

/* -- the kui half ----------------------------------------------------- */

uint32_t kui_ext_abi(void) { return KUI_ABI_VERSION; }

static const KuiStr SLOTS[] = {{(const uint8_t *)"*", 1}};
const KuiStr *kui_ext_slots(size_t *count) {
    *count = 1;
    return SLOTS;
}

typedef struct {
    int clicks;
} Panel;

void *kui_ext_init(void) { return calloc(1, sizeof(Panel)); }
void kui_ext_free(void *user) { free(user); }

void kui_ext_view(void *user, KuiCtx *ui) {
    Panel *p = user;
    const KuiValue *params = kui_slot_params(ui);
    int64_t pane = 0;
    if (params) kui_value_as_int(kui_value_get(params, KUI_STR("pane")), &pane);
    KuiTheme t = KUI_THEME_INIT;
    kui_theme(ui, &t);
    KuiSpec column = {
        .dir = KUI_COLUMN,
        .width = {KUI_GROW, 1},
        .height = {KUI_GROW, 1},
        .pad_l = 12, .pad_r = 12, .pad_t = 12, .pad_b = 12,
        .gap = 8,
    };
    kui_open(ui, &column, NULL);
    {
        KuiValue *tag = kui_value_map();
        kui_value_map_set(tag, KUI_STR("kind"), kui_value_str(KUI_STR("bump")));
        KuiSpec row = {.dir = KUI_ROW, .pad_t = 4, .pad_b = 4};
        kui_open(ui, &row, tag);
        char line[64];
        snprintf(line, sizeof line, "native pane %lld, clicks %d", (long long)pane, p->clicks);
        KuiTextStyle style = {.size = 14, .color = t.fg};
        kui_text(ui, KUI_STR(line), &style);
        kui_close(ui);
    }
    kui_close(ui);
}

void kui_ext_on_event(void *user, const KuiEvent *ev) {
    Panel *p = user;
    if (!ev->payload) return;
    KuiStr kind;
    const KuiValue *k = kui_value_get(ev->payload, KUI_STR("kind"));
    if (k && kui_value_as_str(k, &kind) && kind.len == 4 && memcmp(kind.ptr, "bump", 4) == 0)
        p->clicks++;
}
