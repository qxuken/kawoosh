# Startup memory footprint

Status: alpha.80 baseline measured 2026-11-12, with a reusable CPU-only probe in
[`kawoosh/examples/memory_probe.rs`](../../kawoosh/examples/memory_probe.rs).
This note concerns process memory, separately from the editor's remembered
history described in [memory.md](memory.md).

**Measured finding:** eagerly registering the shipped fonts added about
253 MiB of private commit before the app was constructed. Loading only the
symbols added 2.56 MiB. This was a large startup cost in the measured checkout,
even with an empty directory; the result does not reproduce the reported
190 MB Task Manager reading. Measurement details and limits follow.

The dependency pin moved to alpha.81 on 2026-11-13. Its native OS font
preparation is lazy, so the native-core and final totals below remain
historical alpha.80 results. Its [shipped-font loader](https://github.com/qxuken/kui/blob/c35eeaf8dbd6fbcef4beaecf947e46b51c05cb35/crates/kui-native/src/fonts.rs)
still reads whole files into owned bytes. A fresh probe is needed for
alpha.81 memory totals; the commands below build the current dependency pin.

## Question and measurement boundary

The reported Task Manager observation was approximately 190 MB for a Kawoosh
showing only a directory, compared with Carnet at 82 MB and Zed at 136 MB.
Those observations are the starting question, not a controlled benchmark:
the exact builds, Task Manager column, process grouping, fonts, configuration,
directory contents and prior workloads are not recorded. This investigation
does not reproduce the 190 MB observation or measure a comparable Carnet or
Zed workload. Existing user processes were not used as baselines or changed.

The probe starts fresh child processes, alternating the order of two modes:
all shipped fonts and symbols only. Each child creates the desktop's native
core, registers fonts, constructs Kawoosh, attaches its bundled Lua runtime,
opens a new SQLite state database, loads empty user settings, opens its own
empty directory and builds 101 frames at 1100 × 760 logical pixels, scale 1.
The last 100 frames are separated by 20 ms wall-clock sleeps. Environment
overrides isolate config, user fonts, installed grammars and state; project
config is disabled. Directory I/O and normal app worker threads remain on.
The directory contains no files, VCS checkout, terminal or language-server
workload. See the [probe implementation](../../kawoosh/examples/memory_probe.rs).

There is **no window, GPU device, swapchain, renderer, screenshot device or
command socket**. `Drive` runs the real app view and consumes the core's
output; inspection is disabled after Drive enables it. The executable uses
the repository's dev profile, not the shipped release executable. Therefore
these are controlled startup component measurements, not a desktop app
memory benchmark. See [Drive](https://github.com/qxuken/kui/blob/998f1aa9d30dda682841cab143681e2254ecbc53/crates/kui-native/src/testing.rs),
[main startup](../../kawoosh/src/main.rs) and [build profiles](../../Cargo.toml).

## Counters

The CSV uses [`perf::read_mem`](../../kawoosh/src/perf.rs), which on Windows
reads `PROCESS_MEMORY_COUNTERS_EX`:

| CSV field | Meaning on Windows |
|---|---|
| `resident_bytes` | `WorkingSetSize`: current resident pages, including shared pages |
| `footprint_bytes` | `PrivateUsage`: private committed bytes, including pages not currently resident |
| `peak_resident_bytes` | `PeakWorkingSetSize`: largest total working set seen so far |
| `text_cache_bytes` | kui's estimate of shaped-text cache contents; not an allocator measurement |
| `text_cache_budget_bytes` | cache eviction budget; not an up-front allocation |

Private commit and working set are distinct metrics. Neither field alone is
the private working set a Task Manager memory column can display. The
Windows comment in `perf.rs` calling private commit “what Task Manager calls
memory” is too broad: it should not be used to equate the probe with an
unspecified Task Manager column. Sources: Microsoft's
[counter definitions](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex),
[working set definition](https://learn.microsoft.com/en-us/windows/win32/procthread/process-working-set)
and [counter mapping](https://learn.microsoft.com/en-us/windows/win32/memory/memory-performance-information).

## Source evidence

Desktop startup calls `fonts::load_shipped` on the whole shipped font tree
before constructing Kawoosh. The loader recursively accepts `.ttf`, `.otf`,
`.ttc` and `.otc` files, skips unfilled Git LFS pointers and calls the native
core's file loader for each accepted path. This is independent of the
directory being shown. Sources: [`main::load_fonts`](../../kawoosh/src/main.rs),
[`fonts::font_files` and `fonts::load_shipped`](../../kawoosh/src/fonts.rs).

The measured checkout contained **167 actual font files, 264,094,860 bytes
(251.86 MiB)**, with no LFS placeholders in those files. Counting every file
under `assets/fonts` instead gives 182: that includes licenses, readmes and a
build plan, which the font loader does not load. IosevkaNavcon accounts for
32 TTF files and 216.43 MiB; the symbols-only directory contains one font.
These are file inventories, not direct process-memory measurements.
Source: [the shipped assets](../../assets/fonts), counted by the probe's
`font_inventory` and the PowerShell command below.

At the measured kui-native **0.1.0-alpha.80** pin, `Loader::file` calls
`std::fs::read`, wraps the resulting bytes in an `Arc` and registers them as
`fontdb::Source::SharedFile`. That name does not turn a `Vec<u8>` into an
OS-backed file mapping: this code has read the whole file into owned bytes.
`Core::load_font_file` passes those prepared faces into the session database.
This identifies a concrete private-memory cost to measure, rather than
assuming font bytes stay on disk until first use. Sources: the
[native loader](https://github.com/qxuken/kui/blob/998f1aa9d30dda682841cab143681e2254ecbc53/crates/kui-native/src/system/fonts.rs) and
[core registration](https://github.com/qxuken/kui/blob/998f1aa9d30dda682841cab143681e2254ecbc53/crates/kui-core/src/runtime/resources_api.rs).
The files were inspected in the downloaded crates pinned by
[`Cargo.lock`](../../Cargo.lock). Their `.cargo_vcs_info.json` identifies
commit `998f1aa9d30dda682841cab143681e2254ecbc53`, also the local
`v0.1.0-alpha.80` tag. Source links above and below are pinned to that commit;
the native loader's downloaded source matches it byte-for-byte (SHA-256
`18d8e06b6c858cce30050b5d9ddbcb4397a4138ef3967631446f1c3205ebfb6f`).

kui's initial CPU glyph atlas is 1024 × 1024 × 4 bytes, **4 MiB**. It can
grow, so this is not a bound on later atlas or renderer memory. The shaped
text cache starts with zero entries and zero charged bytes; **64 MiB is its
default eviction budget**, not a fixed 64 MiB allocation. The probe records
both charged bytes and budget to make that difference visible. Sources:
[atlas constructor](https://github.com/qxuken/kui/blob/998f1aa9d30dda682841cab143681e2254ecbc53/crates/kui-core/src/atlas.rs),
[text-cache constructor](https://github.com/qxuken/kui/blob/998f1aa9d30dda682841cab143681e2254ecbc53/crates/kui-core/src/text.rs) and
[cache counters](https://github.com/qxuken/kui/blob/998f1aa9d30dda682841cab143681e2254ecbc53/crates/kui-core/src/runtime.rs), inspected in
the pinned alpha.80 crate sources.

## Reproducing the probe

From the repository root in PowerShell:

```powershell
$probePreviousIncremental = $env:CARGO_INCREMENTAL
try {
    $env:CARGO_INCREMENTAL = '0'
    cargo build -p kawoosh --example memory_probe
} finally {
    $env:CARGO_INCREMENTAL = $probePreviousIncremental
}
& .\target\debug\examples\memory_probe.exe --runs 3 > memory-probe.csv

Get-ChildItem assets/fonts -Recurse -File |
    Where-Object Extension -in '.ttf','.otf','.ttc','.otc' |
    Measure-Object Length -Sum
```

`CARGO_INCREMENTAL=0` was required on this host after existing incremental
artifacts produced access-denied finalization notices and linker errors for
internal Lua/systems symbols. This changes the build procedure, not the
runtime allocator or memory counters. No `cargo clean` was used.

The parent prints a CSV header and one row per stage; each child reports the
font path, file count and byte count on stderr. Every child starts with a new
private temporary directory, which is removed after the child exits. Keep
stderr and the CSV when comparing a later run. Use MiB = bytes / 1,048,576,
and compare the same counter at the same stage.

## Measured results

Host: Windows 11 Pro x64, build 26300; Rust
`1.99.1 (58cf8f9c3 2026-10-13)`, MSVC target. Build: default features,
repository dev profile (workspace opt-level 1, dependency opt-level 2,
line-table debug info), incremental compilation disabled. The executable was
65,816,064 bytes. The working tree was rooted at
`d1217b3f6b19087b927c94214d4bebceda48c092`, with the uncommitted alpha.80 native
constructor migration and runtime-tool changes being verified in this chat.
These are not pristine release-build numbers. Fonts came from this
checkout's `assets/fonts`, not an installed application's resources.

Three fresh processes per mode, in order shipped/symbols, symbols/shipped,
shipped/symbols. Medians below are in MiB, rounded independently to 0.01.
“App” is `Kawoosh::new`; the next stage adds Lua, a new state database and
the empty configuration. Source and measurement: the
[probe](../../kawoosh/examples/memory_probe.rs), run with the command above.

| Stage | Shipped resident | Shipped private commit | Symbols resident | Symbols private commit |
|---|---:|---:|---:|---:|
| Fresh process | 7.18 | 1.73 | 7.18 | 1.74 |
| Native core | 30.63 | 30.30 | 30.65 | 30.32 |
| Fonts registered | 283.10 | 283.06 | 33.61 | 32.88 |
| App constructed | 287.76 | 285.27 | 38.20 | 35.22 |
| Lua, store, configuration | 296.61 | 290.04 | 47.13 | 40.11 |
| Empty directory, first frame | 303.38 | 291.22 | 54.00 | 41.22 |
| Empty directory, frame 101 | 304.61 | 291.51 | 55.25 | 41.56 |

Within each process, font registration increased private commit by
**252.76–252.77 MiB shipped**, versus **2.56 MiB symbols**. This happens before
`Kawoosh::new` and any directory frame, supporting the whole-file loader as
the measured cause of the large difference. The final median difference was
**249.95 MiB private commit** and **249.35 MiB resident**. This difference is
specific to these font modes: the symbols-only mode can choose a different
editor face, so it is not a visual-equivalence benchmark or a proposed
production configuration. Both modes still scan the same OS font catalog.

The text-cache estimate was **95,844 bytes** after the first frame and
**246,675 bytes (0.24 MiB)** after frame 101 in every process. The budget was
67,108,864 bytes in every core. The experiment therefore does not support
counting the 64 MiB text-cache cap as 64 MiB already consumed. The native
catalog reported 80 families before shipped registration, 96 afterward in
shipped mode and 81 in symbols mode. These figures come from the probe's
`text_cache_bytes`, `text_cache_budget_bytes` and `catalog_families` fields.

Raw final rows, retained here in bytes so the six results can be checked
without committing a separate log:

```csv
run,mode,resident_bytes,footprint_bytes,peak_resident_bytes,text_cache_bytes,font_registration_commit_delta_bytes
1,shipped,319324160,305639424,319324160,246675,265039872
1,symbols,57937920,43581440,58028032,246675,2682880
2,symbols,57962496,43532288,58003456,246675,2682880
2,shipped,319401984,305668096,319500288,246675,265052160
3,shipped,319488000,305815552,319500288,246675,265039872
3,symbols,57917440,43597824,57946112,246675,2682880
```

## Follow-up boundary

Reducing eager font loading is the first candidate supported by an explicit
allocation path. Loading the selected family and required symbols first,
then registering another family when chosen, would need to preserve font
family discovery, weight/italic resolution, fallback behavior and the fonts
pane. This investigation does not change production font loading. A reduced
font mode is an experimental comparison, not a ready-to-ship optimization.

GPU/window overhead, the user's configuration and plugins, a populated
state database, real directory size, multiple windows and large or previously
opened files remain separate measurements. The 101-frame run also cannot
establish whether long-lived workloads leak. Compare controlled release
desktop launches and their process groups before drawing conclusions about
the reported Carnet/Zed gap.
