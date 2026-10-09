/* A word index, the plugin `tests/lua_costs.rs` (`native_vs_lua`)
 * measures against its Lua twin `tests/ext/words.lua`: the same three
 * jobs, the same output, one in C over kawoosh.h and kui.h, one in Lua.
 *
 *   :cwords          the buffer's identifiers counted, ranked (count
 *                    down, then the word's bytes up), the top TOP in a
 *                    scratch as "count word" lines. The ranking is kept
 *                    for the pane.
 *   :cwords_data     the same, the text read through kw_call("buf.text")
 *                    instead of kw_buf_text.
 *   :cwords_rename OLD NEW
 *                    every whole-word OLD replaced by NEW, as one batch
 *                    of edits (kw_buf_edits).
 *   :cwords_pane     the ranking as a native pane: ROWS rows of count,
 *                    word and a bar, the selected one highlighted.
 *   :cwords_next     the selection one row down (a change the pane must
 *                    draw).
 *
 * Each command echoes the C side's own clock, phase by phase. A word is
 * [A-Za-z_][A-Za-z0-9_]*, ASCII, scanned left to right; anything else is
 * skipped a byte at a time — Lua's "[%a_][%w_]*" under gmatch. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <time.h>
#endif

#include "kawoosh.h"

#define TOP 100
#define ROWS 200

uint32_t kw_ext_abi(void) { return KW_ABI_VERSION; }
const char *kw_ext_name(void) { return "words"; }

static double now_ms(void) {
#ifdef _WIN32
    LARGE_INTEGER f, c;
    QueryPerformanceFrequency(&f);
    QueryPerformanceCounter(&c);
    return (double)c.QuadPart * 1e3 / (double)f.QuadPart;
#else
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
#endif
}

static int head(uint8_t c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_'; }
static int tail(uint8_t c) { return head(c) || (c >= '0' && c <= '9'); }

/* -- the ranking, kept for the pane --------------------------------- */

typedef struct {
    char *word;
    size_t len;
    long count;
} Entry;

static Entry *ranked;   /* the top ROWS, owned */
static size_t n_ranked;
static long most;       /* the first's count, for the bars */
static size_t selected; /* the pane's row */

static void forget(void) {
    for (size_t i = 0; i < n_ranked; i++) free(ranked[i].word);
    free(ranked);
    ranked = NULL;
    n_ranked = 0;
}

/* -- the index ------------------------------------------------------ */

typedef struct {
    const uint8_t *p; /* into the text, which outlives the count */
    size_t n;
    long count; /* 0 = an empty slot */
} Slot;

static uint64_t fnv(const uint8_t *p, size_t n) {
    uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < n; i++) h = (h ^ p[i]) * 1099511628211ull;
    return h;
}

static int by_rank(const void *a, const void *b) {
    const Slot *x = a, *y = b;
    if (x->count != y->count) return x->count > y->count ? -1 : 1;
    size_t n = x->n < y->n ? x->n : y->n;
    int c = memcmp(x->p, y->p, n);
    if (c) return c;
    return x->n < y->n ? -1 : x->n > y->n;
}

/* The command's arguments, as `:cwords_rename OLD NEW` gave them. */
static KuiStr arg(const KuiValue *args, size_t i) {
    const KuiValue *c = kui_value_at(args, 0);
    const KuiValue *list = c ? kui_value_get(c, KUI_STR("args")) : NULL;
    KuiStr s = {0};
    if (list) kui_value_as_str(kui_value_at(list, i), &s);
    return s;
}

static KuiValue *index_words(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)args;
    int data = user != NULL;
    double t0 = now_ms();

    KuiValue *held = NULL;
    KuiStr text;
    if (data) {
        held = kw_call(ctx, KUI_STR("buf.text"), NULL);
        if (!held || !kui_value_as_str(held, &text)) return NULL;
    } else if (!kw_buf_text(ctx, 0, &text)) {
        return NULL;
    }
    double t1 = now_ms();

    size_t cap = 1024, used = 0;
    Slot *tab = calloc(cap, sizeof *tab);
    const uint8_t *s = text.ptr, *end = text.ptr + text.len;
    long words = 0;
    while (s < end) {
        if (!head(*s)) {
            s++;
            continue;
        }
        const uint8_t *w = s++;
        while (s < end && tail(*s)) s++;
        size_t n = (size_t)(s - w);
        words++;
        size_t at = fnv(w, n) & (cap - 1);
        while (tab[at].count && !(tab[at].n == n && memcmp(tab[at].p, w, n) == 0)) at = (at + 1) & (cap - 1);
        if (tab[at].count) {
            tab[at].count++;
            continue;
        }
        tab[at] = (Slot){w, n, 1};
        if (++used * 2 > cap) { /* grow at half full */
            size_t ncap = cap * 2;
            Slot *nt = calloc(ncap, sizeof *nt);
            for (size_t i = 0; i < cap; i++) {
                if (!tab[i].count) continue;
                size_t j = fnv(tab[i].p, tab[i].n) & (ncap - 1);
                while (nt[j].count) j = (j + 1) & (ncap - 1);
                nt[j] = tab[i];
            }
            free(tab);
            tab = nt;
            cap = ncap;
        }
    }
    double t2 = now_ms();

    /* Packed, then sorted whole: the Lua twin sorts every word too. */
    size_t k = 0;
    for (size_t i = 0; i < cap; i++)
        if (tab[i].count) tab[k++] = tab[i];
    qsort(tab, k, sizeof *tab, by_rank);
    double t3 = now_ms();

    forget();
    n_ranked = k < ROWS ? k : ROWS;
    ranked = malloc(n_ranked * sizeof *ranked);
    for (size_t i = 0; i < n_ranked; i++) {
        ranked[i].word = malloc(tab[i].n + 1);
        memcpy(ranked[i].word, tab[i].p, tab[i].n);
        ranked[i].word[tab[i].n] = 0;
        ranked[i].len = tab[i].n;
        ranked[i].count = tab[i].count;
    }
    most = n_ranked ? ranked[0].count : 1;
    selected = 0;

    size_t top = k < TOP ? k : TOP, olen = 0, ocap = 64 * (top + 1);
    char *out = malloc(ocap);
    for (size_t i = 0; i < top; i++) {
        if (olen + tab[i].n + 32 > ocap) out = realloc(out, ocap *= 2);
        olen += (size_t)snprintf(out + olen, ocap - olen, "%ld %.*s\n", tab[i].count, (int)tab[i].n,
                                 (const char *)tab[i].p);
    }
    if (olen) olen--; /* the last newline */
    kw_do(ctx, "buf.open_scratch",
          kw_map("name", kw_str("cwords"), "text", kw_strn(out, olen), "read_only", kw_bool(true), NULL));
    free(out);
    free(tab);
    kui_value_free(held);
    double t4 = now_ms();

    char msg[200];
    snprintf(msg, sizeof msg, "read %.3f count %.3f sort %.3f out %.3f total %.3f ms; %ld words, %zu distinct",
             t1 - t0, t2 - t1, t3 - t2, t4 - t3, t4 - t0, words, k);
    kw_do(ctx, "echo", kw_str(msg));
    return NULL;
}

/* -- the rename ----------------------------------------------------- */

static KuiValue *rename_word(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    KuiStr old = arg(args, 0), new = arg(args, 1);
    if (!old.len) return NULL;
    double t0 = now_ms();
    KuiStr text;
    if (!kw_buf_text(ctx, 0, &text)) return NULL;
    double t1 = now_ms();
    size_t cap = 1024, n = 0;
    KwEdit *edits = malloc(cap * sizeof *edits);
    const uint8_t *p = text.ptr;
    size_t len = text.len;
    for (size_t i = 0; i + old.len <= len;) {
        const uint8_t *hit = memchr(p + i, old.ptr[0], len - i - old.len + 1);
        if (!hit) break;
        size_t at = (size_t)(hit - p);
        if (memcmp(hit, old.ptr, old.len) == 0 && (at == 0 || !tail(p[at - 1])) &&
            (at + old.len == len || !tail(p[at + old.len]))) {
            if (n == cap) edits = realloc(edits, (cap *= 2) * sizeof *edits);
            edits[n++] = (KwEdit){at, at + old.len, new};
            i = at + old.len;
        } else {
            i = at + 1;
        }
    }
    double t2 = now_ms();
    bool ok = kw_buf_edits(ctx, 0, edits, n);
    free(edits);
    double t3 = now_ms();
    char msg[200];
    snprintf(msg, sizeof msg, "read %.3f find %.3f queue %.3f total %.3f ms; %zu edits%s", t1 - t0, t2 - t1,
             t3 - t2, t3 - t0, n, ok ? "" : " (refused)");
    kw_do(ctx, "echo", kw_str(msg));
    return NULL;
}

/* -- the pane, its kawoosh half -------------------------------------- */

static KuiValue *open_pane(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)args;
    kw_do(ctx, "view_open", kw_str("cwords"));
    return NULL;
}

static KuiValue *next_row(void *user, KwCtx *ctx, const KuiValue *args) {
    (void)user;
    (void)ctx;
    (void)args;
    if (n_ranked) selected = (selected + 1) % n_ranked;
    return NULL;
}

static int DATA = 1;

static void command(KwCtx *ctx, const char *name, KwFn fn, void *user) {
    kw_do(ctx, "command", kw_str(name), kw_fn(ctx, fn, user));
}

void *kw_ext_init(KwCtx *ctx) {
    KuiStr ns;
    if (!kw_namespace(ctx, &ns)) return NULL;
    kw_do(ctx, "view", kw_str("cwords"), kw_null(), kw_null(), kw_map("native", kw_strn(ns.ptr, ns.len), NULL));
    command(ctx, "cwords", index_words, NULL);
    command(ctx, "cwords_data", index_words, &DATA);
    kw_do(ctx, "command", kw_str("cwords_rename"), kw_fn(ctx, rename_word, NULL),
          kw_map("args", kw_list(kw_str("text"), kw_str("text")), NULL));
    command(ctx, "cwords_pane", open_pane, NULL);
    command(ctx, "cwords_next", next_row, NULL);
    return NULL;
}

void kw_ext_free(void *user) {
    (void)user;
    forget();
}

/* -- the pane, its kui half ------------------------------------------ */

uint32_t kui_ext_abi(void) { return KUI_ABI_VERSION; }

static const KuiStr SLOTS[] = {{(const uint8_t *)"*", 1}};
const KuiStr *kui_ext_slots(size_t *count) {
    *count = 1;
    return SLOTS;
}

void *kui_ext_init(void) { return NULL; }
void kui_ext_free(void *user) { (void)user; }

/* The Lua twin's tree, node for node: a scrolling column; a head; per
 * row a row of the count (fixed width), the word (grows) and a bar
 * (fixed width, the count's share of the first's). */
void kui_ext_view(void *user, KuiCtx *ui) {
    (void)user;
    KuiTheme t = KUI_THEME_INIT;
    kui_theme(ui, &t);
    KuiSpec body = {
        .dir = KUI_COLUMN,
        .width = {KUI_GROW, 1},
        .height = {KUI_GROW, 1},
        .pad_l = 12, .pad_r = 12, .pad_t = 12, .pad_b = 12,
        .gap = 2,
        .bg = t.bg,
        .overflow = KUI_SCROLL_Y,
    };
    kui_open_keyed(ui, KUI_STR("body"), &body, NULL);
    char line[96];
    snprintf(line, sizeof line, "%zu words ranked, row %zu", n_ranked, selected + 1);
    KuiTextStyle muted = {.size = 12, .color = t.muted};
    kui_text(ui, KUI_STR(line), &muted);
    KuiTextStyle mono = {.size = 13, .color = t.fg, .family = KUI_FONT_MONO, .wrap = KUI_WRAP_NONE};
    KuiTextStyle faint = {.size = 13, .color = t.faint, .family = KUI_FONT_MONO, .wrap = KUI_WRAP_NONE};
    for (size_t i = 0; i < n_ranked; i++) {
        KuiSpec row = {
            .dir = KUI_ROW,
            .width = {KUI_GROW, 1},
            .pad_l = 8, .pad_r = 8, .pad_t = 2, .pad_b = 2,
            .gap = 8,
            .cross_align = KUI_CENTER,
            .radius = 3,
            .bg = i == selected ? t.selection : 0,
        };
        kui_open(ui, &row, NULL);
        {
            KuiSpec cell = {.dir = KUI_ROW, .width = {KUI_FIXED, 64}};
            kui_open(ui, &cell, NULL);
            snprintf(line, sizeof line, "%ld", ranked[i].count);
            kui_text(ui, KUI_STR(line), &faint);
            kui_close(ui);
            KuiSpec word = {.dir = KUI_ROW, .width = {KUI_GROW, 1}};
            kui_open(ui, &word, NULL);
            kui_text(ui, (KuiStr){(const uint8_t *)ranked[i].word, ranked[i].len}, &mono);
            kui_close(ui);
            KuiSpec track = {.dir = KUI_ROW, .width = {KUI_FIXED, 160}, .height = {KUI_FIXED, 8}, .bg = t.sunken,
                             .radius = 2};
            kui_open(ui, &track, NULL);
            KuiSpec bar = {.dir = KUI_ROW,
                           .width = {KUI_FIXED, (float)(160.0 * (double)ranked[i].count / (double)most)},
                           .height = {KUI_GROW, 1},
                           .bg = t.accent,
                           .radius = 2};
            kui_open(ui, &bar, NULL);
            kui_close(ui);
            kui_close(ui);
        }
        kui_close(ui);
    }
    kui_close(ui);
}

void kui_ext_on_event(void *user, const KuiEvent *ev) {
    (void)user;
    (void)ev;
}
