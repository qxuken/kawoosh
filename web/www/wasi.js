// The WASI calls wasi-libc makes, answered for a page (web/README.md).
// Lua and the grammars are built against wasi-libc and linked into the
// module whole; the module imports these from `wasi_snapshot_preview1`,
// which the page's import map names this file for. A page has no files
// of libc's (the editor's disk is its own, in memory), no environment
// and no process to exit: stdout and stderr go to the console, the clock
// is the page's, and the rest refuse.

let memory;

/** The module's memory, once it is instantiated: before any call. */
export function bind(m) {
  memory = m;
}

const view = () => new DataView(memory.buffer);
const ESUCCESS = 0, EBADF = 8, ENOSYS = 52;
const pending = { 1: "", 2: "" };

export function environ_sizes_get(count, size) {
  view().setUint32(count, 0, true);
  view().setUint32(size, 0, true);
  return ESUCCESS;
}

export function environ_get() {
  return ESUCCESS;
}

export function clock_time_get(id, _precision, out) {
  // 0 is the realtime clock; the monotonic and CPU clocks all read the
  // page's monotonic one.
  const ms = id === 0 ? Date.now() : performance.now();
  view().setBigUint64(out, BigInt(Math.round(ms * 1e6)), true);
  return ESUCCESS;
}

export function fd_write(fd, iovs, count, written) {
  if (fd !== 1 && fd !== 2) return EBADF;
  const v = view();
  let total = 0;
  for (let i = 0; i < count; i++) {
    const ptr = v.getUint32(iovs + i * 8, true);
    const len = v.getUint32(iovs + i * 8 + 4, true);
    pending[fd] += new TextDecoder().decode(new Uint8Array(memory.buffer, ptr, len));
    total += len;
  }
  let nl;
  while ((nl = pending[fd].indexOf("\n")) >= 0) {
    (fd === 1 ? console.log : console.error)(pending[fd].slice(0, nl));
    pending[fd] = pending[fd].slice(nl + 1);
  }
  v.setUint32(written, total, true);
  return ESUCCESS;
}

export function fd_fdstat_get(fd, out) {
  if (fd > 2) return EBADF;
  const v = view();
  v.setUint8(out, 2); // a character device
  v.setUint16(out + 2, 0, true);
  v.setBigUint64(out + 8, 0n, true);
  v.setBigUint64(out + 16, 0n, true);
  return ESUCCESS;
}

export function fd_close() { return ESUCCESS; }
export function fd_prestat_get() { return EBADF; }
export function fd_prestat_dir_name() { return EBADF; }
export function fd_read() { return EBADF; }
export function fd_seek() { return ENOSYS; }
export function fd_fdstat_set_flags() { return ENOSYS; }
export function fd_renumber() { return ENOSYS; }
export function path_open() { return ENOSYS; }
export function path_remove_directory() { return ENOSYS; }
export function path_rename() { return ENOSYS; }
export function path_unlink_file() { return ENOSYS; }
export function random_get(buf, len) {
  crypto.getRandomValues(new Uint8Array(memory.buffer, buf, len));
  return ESUCCESS;
}
export function proc_exit(code) {
  throw new Error(`the module exited (${code})`);
}
