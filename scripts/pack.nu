#!/usr/bin/env nu
# The native extension pack (docs/design/native.md): what an author needs
# to build a Kawoosh extension without the workspace, for every platform
# Kawoosh ships — the two headers, an example, and on Windows the import
# library a DLL links against.
#
#   nu scripts/pack.nu                                  every platform
#   nu scripts/pack.nu --platforms win32-x64,darwin-arm64
#
# What lands in target/pack/, per platform:
#
#   kawoosh-ext-<version>-<platform>/
#     include/kawoosh.h   the doors (kw_*) and the entry points you define
#     include/kui.h       the values, strings and drawing (kui_*), at the
#                         kui this Kawoosh is built with
#     lib/kawoosh.lib     Windows only: the import library
#     example/dupes.c     native.md's example, as the tests build it
#     BUILD.txt           the versions, both ABI numbers, how the library
#                         was made, and the line that builds the example
#   kawoosh-ext-<version>-<platform>.tar.gz, and SHA256SUMS over every
#   tarball there
#
# Nothing is compiled for a platform, so every one is packed from any
# machine: no container, no SDK. On macOS and Linux an extension links
# against nothing — it leaves every kw_* and kui_* undefined, and the
# loader binds them from the kawoosh executable, which exports them
# (kawoosh/build.rs, `export_dynamic`). A Windows DLL may leave nothing
# undefined: it names the module each import comes from, and takes that
# name from an import library. Such a library holds no code, only the
# names kawoosh.exe exports and the module's name, so it is made here from
# the export list the build writes (`export_def`: the headers'
# prototypes), by llvm-dlltool or zig's copy of it. It imports by name, as
# the one link.exe writes beside kawoosh.exe does (the Kawoosh folder
# ships that one, scripts/windows-app.nu); a DLL linked against either
# loads into kawoosh.exe and no other host.

const ROOT = path self | path dirname | path dirname
const OUT = $ROOT | path join target pack

# Kawoosh's platforms, as kui's pack names them; `machine` is the import
# library's, for llvm-dlltool's -m.
const PLATFORMS = [
    [platform machine];
    [darwin-arm64 null]
    [linux-x64 null]
    [linux-arm64 null]
    [win32-x64 "i386:x86-64"]
]

def version [] {
    open ($ROOT | path join Cargo.toml) | get workspace.package.version
}

def main [
    --platforms: string # which to pack, comma-separated; default every one
] {
    let wanted = if $platforms == null { $PLATFORMS | get platform } else { $platforms | split row "," | str trim }
    for p in $wanted {
        if $p not-in ($PLATFORMS | get platform) {
            error make { msg: $"unknown platform ($p); there is: ($PLATFORMS | get platform | str join ', ')" }
        }
    }
    let todo = $PLATFORMS | where platform in $wanted

    let def = export-list
    let names = open --raw $def | lines | skip until {|l| ($l | str trim) == "EXPORTS" } | skip 1
        | each { str trim } | where {|l| $l != "" and not ($l | str starts-with ";") }
    let kui = kui-header
    let headers = {
        "kawoosh.h": ($ROOT | path join kawoosh include kawoosh.h)
        "kui.h": $kui.path
    }
    let abi = {
        kw: (open --raw $headers."kawoosh.h" | parse -r '#define KW_ABI_VERSION (?<v>\d+)' | get 0.v)
        kui: (open --raw $headers."kui.h" | parse -r '#define KUI_ABI_VERSION (?<v>\d+)u' | get 0.v)
    }
    let dlltool = if ($todo | any {|r| $r.machine != null }) { dlltool } else { [] }
    let commit = do { ^git -C $ROOT describe --always --dirty } | complete | get stdout | str trim

    mkdir $OUT
    let built = $todo | each {|row|
        print $"== ($row.platform)"
        stage $row {
            def: $def, names: $names, headers: $headers, kui: $kui.version
            abi: $abi, dlltool: $dlltool, commit: $commit
        }
    }

    # Over every tarball here, not only this run's: a run of one platform
    # leaves the others' packs, and their sums, where they were.
    cd $OUT
    let sums = glob "kawoosh-ext-*.tar.gz" | each {|t| sha256sum-of ($t | path basename) } | sort
    $sums | str join "\n" | save -f SHA256SUMS
    print ($built | wrap tarball | insert size {|r| ls $r.tarball | get 0.size } | table --width 100)
    print $"in ($OUT)"
}

# The export list the build wrote (kawoosh/build.rs, `export_def`), on
# any target: cargo says where the build script's OUT_DIR is. A build of
# the kawoosh library alone, the debug one a working tree has warm.
def export-list [] {
    cd $ROOT
    let id = ^cargo pkgid -p kawoosh | str trim
    let r = do { ^cargo build -p kawoosh --lib --message-format=json } | complete
    if $r.exit_code != 0 {
        print -e $r.stderr
        error make { msg: "the kawoosh build failed" }
    }
    let run = $r.stdout | lines | each {|l| try { $l | from json } catch { null } }
        | where {|m| $m != null and $m.reason? == "build-script-executed" and $m.package_id? == $id }
    if ($run | is-empty) {
        error make { msg: "cargo ran no kawoosh build script" }
    }
    let def = $run | last | get out_dir | path join kawoosh-exports.def
    if not ($def | path exists) {
        error make { msg: $"no ($def): the build could not read the headers; its warning says why" }
    }
    $def
}

# `kui.h` of the kui-ffi the build links, and that kui's version: the
# header the export list was read from. The graph for this host alone:
# unfiltered, `--offline` wants every target's sources.
def kui-header [] {
    let host = ^rustc -vV | lines | where $it starts-with "host: " | first | str substring 6..
    let pkg = ^cargo metadata --format-version 1 --offline --filter-platform $host --manifest-path ($ROOT | path join Cargo.toml)
        | from json | get packages | where name == "kui-ffi" | first
    { path: ($pkg.manifest_path | path dirname | path join include kui.h), version: $pkg.version }
}

# What makes an import library from a .def: llvm-dlltool (on the PATH, or
# Homebrew's keg-only LLVM), else zig's copy of it.
def dlltool [] {
    if (which llvm-dlltool | is-not-empty) { return [llvm-dlltool] }
    for p in ["/opt/homebrew/opt/llvm/bin/llvm-dlltool" "/usr/local/opt/llvm/bin/llvm-dlltool"] {
        if ($p | path exists) { return [$p] }
    }
    if (which zig | is-not-empty) { return [zig dlltool] }
    error make { msg: "a Windows pack needs llvm-dlltool (LLVM) or zig on the PATH" }
}

# The line that builds the example, from the pack's own folder.
def build-line [platform: string] {
    match ($platform | split row "-" | first) {
        "darwin" => "cc -O2 -shared -undefined dynamic_lookup -I include example/dupes.c -o dupes.dylib"
        "linux" => "cc -O2 -shared -fPIC -I include example/dupes.c -o dupes.so"
        "win32" => "clang -O2 -shared -I include example\\dupes.c lib\\kawoosh.lib -o dupes.dll"
    }
}

# target/pack/kawoosh-ext-<version>-<platform>/ and its tarball.
def stage [row: record, the: record] {
    let name = $"kawoosh-ext-(version)-($row.platform)"
    let dir = $OUT | path join $name
    rm -rf $dir
    mkdir ($dir | path join include) ($dir | path join example)
    # With LF line ends whatever the checkout has: a CRLF one (Git for
    # Windows' autocrlf) would otherwise ship files that differ by host.
    for h in ($the.headers | transpose file src) {
        open --raw $h.src | str replace -a "\r\n" "\n" | save -f --raw ($dir | path join include $h.file)
    }
    open --raw ($ROOT | path join kawoosh tests ext dupes.c) | str replace -a "\r\n" "\n"
        | save -f --raw ($dir | path join example dupes.c)

    let lib = if $row.machine == null {
        "none: an extension links against nothing, and the loader binds every kw_* and kui_* from the kawoosh executable"
    } else {
        mkdir ($dir | path join lib)
        let out = $dir | path join lib kawoosh.lib
        let tool = $the.dlltool
        let r = do { ^($tool | first) ...($tool | skip 1) -m $row.machine -d $the.def -D kawoosh.exe -l $out } | complete
        if $r.exit_code != 0 or not ($out | path exists) {
            print -e $r.stderr
            error make { msg: $"($tool | str join ' ') made no import library for ($row.platform)" }
        }
        $"lib/kawoosh.lib, made by ($tool | str join ' ') from the export list: ($the.names | length) names, imported by name from kawoosh.exe"
    }
    [
        $"Kawoosh native extension pack (version), ($row.platform)"
        $"commit: ($the.commit)"
        $"KW_ABI_VERSION: ($the.abi.kw) \(kawoosh.h); KUI_ABI_VERSION: ($the.abi.kui) \(kui.h, kui-ffi ($the.kui))"
        "  A Kawoosh whose numbers differ refuses the extension at load, saying both."
        $"library: ($lib)"
        ""
        "Build the example, from this folder:"
        ""
        $"  (build-line $row.platform)"
        ""
        "and load it: put the library in ext/ under Kawoosh's config directory"
        "(kawoosh.fs.config() answers where) and add to init.lua"
        ""
        "  kawoosh.extension(\"dupes\")"
        ""
        "then :dupes lists the buffer's duplicate lines. The headers say the"
        "rest; docs/design/native.md is the design."
        ""
    ] | str join "\n" | save -f ($dir | path join BUILD.txt)
    cd $OUT
    tar -czf $"($name).tar.gz" $name
    $"($name).tar.gz"
}

def sha256sum-of [file: string] {
    $"(open --raw $file | hash sha256)  ($file)"
}
