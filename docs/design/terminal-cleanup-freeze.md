# Windows terminal cleanup freeze investigation

Status: not reproduced, 2026-11-12. No production terminal change is
justified by the evidence collected so far.

The reported symptom is the entire Kawoosh window becoming unresponsive
while `scoop cleanup --all` runs on Windows. The user later clarified
that the shell was Nushell and the work mainly removed roughly 30 GB of
old LLVM versions. The investigation must not run cleanup against the
user's installed applications.

## Safe feedback loop

`kawoosh/tests/terminal_cleanup.rs` is an explicit, ignored Windows
diagnostic. From PowerShell at the repository root:

```powershell
$env:CARGO_INCREMENTAL = '0'
cargo test -p kawoosh --test terminal_cleanup -- --ignored --nocapture --test-threads=1
```

Disabling incremental compilation here avoids the unrelated LLVM linker
symbol errors observed with this workspace's existing incremental build
artifacts. It is a command-session build setting, not an application fix.

The probe creates fresh temporary directories and starts only disposable
shell subprocesses with profiles disabled. The output probes print text
and read their own generated output fixtures. The metadata probes delete
only generated fixture versions, after resolving and checking every
absolute target against the owned fixture root. They never invoke Scoop
or discover installed applications.
Successful probes remove only their own verified temporary directory;
failed probes retain artifacts and print their location.

Each workload runs in its own test subprocess. The parent watches a
heartbeat around frames, kills only that probe process if frames stall
for three seconds, and bounds an output workload to 60 seconds. Metadata
workloads get 180 seconds including fixture creation, whose heartbeat
advances per directory. Startup and teardown have separate 20-second
stall allowances. The terminal's
existing Windows job handling ends its disposable child when the probe
exits. A blocked frame therefore cannot leave the test runner waiting
indefinitely.

The exercised path is:

```text
PowerShell -> native ConPTY -> real io reader -> PtyOutput
  -> pump_ptys (existing frame budget) -> Terminal::feed
  -> terminal rendering in the headless Kawoosh/KUI frame harness
```

Metadata probes additionally launch through Nushell and a generated
`.cmd` shim using the installed Scoop shim's host selection: `pwsh.exe`
when available, otherwise Windows PowerShell. The generated shim points
only to the disposable probe script. The installed Scoop shim is read
for its launch shape and is never executed. Nu's external invocation
uses its documented [`run-external`](https://www.nushell.sh/commands/docs/run-external.html).

An editor pane remains visible beside the terminal. The probe inserts an
editor character every ten frames and checks that each insertion changes
the buffer. It also requires the terminal's final marker and successful
process exit. Frame times, editor input latency, app process CPU, and
frames leaving pending output are printed.

## Workloads and observations

[Scoop's official cleanup implementation](https://github.com/ScoopInstaller/Scoop/blob/master/libexec/scoop-cleanup.ps1)
was inspected on 2026-11-12. Its output uses a colored application label,
version strings written without a newline, and a line ending per changed
application. The all-applications mode suppresses already-clean entries
and prints a final success message. This source does not explicitly emit
terminal protocol queries. PowerShell and the console host can still
produce their own terminal traffic.

The first probe mimics that output API and shape. The larger two are
deliberate stress workloads, rather than claims about Scoop's volume or
actual emitted bytes.

The initial five complete output-replay runs passed on this workspace with
KUI alpha.80. Measurements from the first run:

| Workload | Frames | Frames leaving output pending | Median frame | Worst frame | Exit |
| --- | ---: | ---: | ---: | ---: | --- |
| 1,000 synthetic apps using PowerShell `Write-Host`, colors and `-NoNewline` | 179 | 0 | 0.280 ms | 1.985 ms | 0 |
| 400,000 colored lines, about 18 MB | 300 | 37 | 0.233 ms | 7.840 ms | 0 |
| 40,000 lines containing color, cursor, device and size queries | 74 | 7 | 0.305 ms | 37.044 ms | 0 |

Across those five runs the largest observed frame was 37.044 ms. All
fifteen workload processes exited successfully and showed their output
tail; no watchdog fired. The colored flood repeatedly left output for
subsequent frames, so the existing parse budget was exercised.

## Nushell and filesystem metadata extension

The additional probe can be run independently:

```powershell
$env:CARGO_INCREMENTAL = '0'
cargo test -p kawoosh --test terminal_cleanup nushell_metadata_cleanup_keeps_frames_and_editor_input_responsive -- --ignored --nocapture --test-threads=1
```

Nushell 0.116.0 was present in this environment. Each workload creates
two synthetic LLVM version directories, each containing 48 leaf header
directories with 256 files: 24,576 files and 384 KiB of logical payload.
Deletion uses PowerShell `Remove-Item -LiteralPath -Recurse -Force` in the
disposable subprocess, interleaved with Scoop-shaped status output.
It exercises file creation/removal and directory metadata without writing
30 GB. It does not model 30 GB of storage traffic or cache pressure.

One variant runs with the editor and terminal; another attaches the real
Lua runtime and shows the fixture in a third, actual directory pane. The
watched variant verifies that the directory is subscribed to the app's
watcher and requires the directory buffer to automatically stop listing
the deleted versions before the test finishes. The current watcher
(`systems/src/watch.rs`) polls watched paths every 500 ms; it is not a
recursive native filesystem event queue.

Two initial Nu-to-Windows-PowerShell runs passed in both variants. First
run measurements, before adding the extra `.cmd` shim:

| Variant | Deletion wall | PowerShell CPU | App CPU | Median frame | Worst frame | Editor insertions | Worst insertion |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Editor and terminal | 2.927 s | 2.641 s | 0.484 s | 0.314 ms | 2.121 ms | 44 | 1.376 ms |
| Directory pane and watcher too | 2.394 s | 2.063 s | 0.531 s | 0.415 ms | 3.950 ms | 49 | 1.858 ms |

Both processes exited with code 0, the watched listing refreshed, and no
watchdog fired. App CPU is the sum across the probe process's threads;
it excludes Nu and PowerShell and is distinct from elapsed wall time.

The final launch path adds the generated `.cmd` shim and its actual
PowerShell selection. First complete measurements with that path:

| Variant | PowerShell | Deletion wall | PowerShell CPU | App CPU | Median frame | Worst frame | Editor insertions | Worst insertion |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Editor and terminal | 7.6.10 | 2.361 s | 2.141 s | 0.391 s | 0.240 ms | 2.014 ms | 54 | 1.275 ms |
| Directory pane and watcher too | 7.6.9 | 2.203 s | 2.219 s | 0.531 s | 0.381 ms | 29.454 ms | 52 | 1.817 ms |

Both exited 0, all editor insertions were handled, and the directory pane
refreshed automatically. Host versions differ because the app's shell
PATH and the test runner's PATH can select different installed pwsh
executables; each script reports its actual version. Neither variant
reproduced a freeze.

One development run exposed a diagnostic harness error rather than a
freeze: changing pane focus directly in the model and sending a key
before framing could route that first key to the previous KUI key sink.
The probe now frames after its direct focus change before asserting input.
That failed assertion was not counted as a reproduction of the reported
window freeze and did not justify a production change.

The final full diagnostic run passed all three tests in 28.58 seconds,
covering the three output workloads and both Nu/shim deletion variants.
In that repeat, the watched deletion variant handled 59 editor insertions,
refreshed its listing, and recorded a 0.383 ms median frame, 23.872 ms
worst frame, and 2.027 ms worst insertion. No three-second watchdog fired.

These observations do **not** establish a cause or a fix for the reported
freeze. In particular:

- This is the real ConPTY, reader, parser and app frame path, but uses the
  headless renderer rather than the native Windows event loop and GPU.
- The query workload goes through ConPTY, which may consume or rewrite
  terminal queries. It does not establish that every injected query
  reached `Terminal::send`, or prove that synchronous writer calls cannot
  block under different conditions.
- The synthetic filesystem operations reproduce many-file metadata work,
  but the user's 30 GB storage/cache workload, installed app set,
  persist links, shell profiles and concurrent activity remain absent.
- Nu runs a disposable script with configuration and history disabled;
  its interactive prompt and line editor are not exercised.
- A three-second watchdog detects a freeze; measured subsecond stalls
  remain visible in the timing report and are not claimed to be absent.

## Next evidence needed

The next useful input is a redacted terminal recording or output capture
from the failing run, the last visible output line, the Nu and PowerShell
versions, and whether the command eventually finished. For a persistent freeze, a
thread stack captured from the frozen Kawoosh process would distinguish
blocked I/O from parsing, layout, rendering, or another subsystem.

Until a replay or isolated workload reproduces the reported symptom,
the synchronous writer is an unverified possibility. Changing it based
only on its appearance in the code would be a speculative fix.
