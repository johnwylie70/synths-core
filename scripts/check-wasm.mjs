// Loads the WebAssembly module the way an audio worklet would, lists what it
// imports and exports, plays a note and times a busy engine.
//
//     node scripts/check-wasm.mjs target/wasm/synths_core.wasm

import { readFileSync } from 'node:fs';

const path = process.argv[2] || 'target/wasm/synths_core.wasm';
const module = new WebAssembly.Module(readFileSync(path));

const imports = WebAssembly.Module.imports(module);
console.log('imports:', imports.length ? imports.map((i) => `${i.module}.${i.name}`).join(', ') : 'none');
console.log('exports:', WebAssembly.Module.exports(module).map((e) => e.name).join(', '));

// The four functions the module asks for are never called while it works:
// they are there for the case of a crash. These stand-ins are all a host needs.
let memory;
const wasi = {
  // Rust writes a message here if it panics. Say the bytes were written.
  fd_write(fd, iovs, iovsLen, nwritten) {
    const view = new DataView(memory.buffer);
    let total = 0;
    for (let i = 0; i < iovsLen; i++) total += view.getUint32(iovs + i * 8 + 4, true);
    view.setUint32(nwritten, total, true);
    return 0;
  },
  // There are no environment variables.
  environ_sizes_get(count, size) {
    const view = new DataView(memory.buffer);
    view.setUint32(count, 0, true);
    view.setUint32(size, 0, true);
    return 0;
  },
  environ_get() {
    return 0;
  },
  proc_exit(code) {
    throw new Error(`synths-core stopped (${code})`);
  },
};
const unknown = imports.filter((i) => i.module !== 'wasi_snapshot_preview1' || !(i.name in wasi));
if (unknown.length) {
  console.error('imports this script has no stand-in for:', unknown);
  process.exit(1);
}

const { exports: x } = new WebAssembly.Instance(module, { wasi_snapshot_preview1: wasi });
memory = x.memory;
if (x._initialize) x._initialize();

const text = (ptr, len) => new TextDecoder().decode(new Uint8Array(memory.buffer, ptr, len));
const table = JSON.parse(text(x.synths_param_table(), x.synths_param_table_len()));
console.log(`version ${table.version}, interface ${x.synths_abi_version()}, ${table.params.length} parameters, ${table.events.length} event types`);
const id = Object.fromEntries(table.params.map((p) => [p.name, p.id]));

const SR = 48000;
const engine = x.synths_create(SR);

// Write events into the engine's buffer, then hand them over.
function push(events) {
  const view = new DataView(memory.buffer, x.synths_event_buffer(engine), events.length * 32);
  events.forEach(([time, type, part, a, xv, y], i) => {
    const at = i * 32;
    view.setFloat64(at, time, true);
    view.setUint16(at + 8, type, true);
    view.setUint16(at + 10, part, true);
    view.setUint32(at + 12, a, true);
    view.setFloat32(at + 16, xv, true);
    view.setFloat32(at + 20, y || 0, true);
    view.setFloat32(at + 24, 0, true);
    view.setUint32(at + 28, 0, true);
  });
  const taken = x.synths_push_events(engine, events.length);
  if (taken !== events.length) throw new Error(`only ${taken} of ${events.length} events were taken`);
}

const NOTE_ON = 1, PARAM = 4;

// A sine at 440 Hz from frame 1000.
push([
  [0, PARAM, 0, id['osc1.wave'], 0],
  [0, PARAM, 0, id['amp.attack'], 0],
  [1000, NOTE_ON, 0, 0, 69, 1],
]);
const out = new Float32Array(SR);
for (let done = 0; done < SR; done += 128) {
  const n = x.synths_render(engine, Math.min(128, SR - done));
  out.set(new Float32Array(memory.buffer, x.synths_output_left(engine), n), done);
}
const first = out.findIndex((v) => v !== 0);
let crossings = 0;
for (let i = 24001; i < SR; i++) if (out[i - 1] < 0 && out[i] >= 0) crossings++;
const peak = out.reduce((m, v) => Math.max(m, Math.abs(v)), 0);
console.log(`a note: first sound at frame ${first}, ${crossings * 2} Hz, peak ${peak.toFixed(3)}`);
if (first !== 1000 || crossings !== 220 || !(peak > 0.1 && peak < 0.3)) {
  console.error('that is not the note that was asked for');
  process.exit(1);
}

// A sample, through memory from synths_alloc.
const frames = 4800;
const ptr = x.synths_alloc(frames * 4);
new Float32Array(memory.buffer, ptr, frames).fill(0.5);
if (x.synths_sample_load(engine, 0, ptr, frames, 1, SR) !== 0) throw new Error('the sample did not load');
x.synths_free(ptr);

// Time a busy engine: 64 voices with everything on, 10 seconds of sound.
x.synths_reset(engine);
const busy = [
  ['osc1.wave', 2], ['osc2.wave', 3], ['osc2.level', 0.8], ['osc2.fine', 7], ['sub.level', 0.5],
  ['noise.level', 0.1], ['noise.type', 1], ['filter.type', 1], ['filter.slope', 1], ['filter.cutoff', 1500],
  ['filter.resonance', 0.4], ['filter.env', 24], ['lfo1.pitch', 0.2], ['lfo1.pulse_width', 0.4],
  ['lfo2.cutoff', 12], ['lfo2.pan', 0.5], ['fx.drive', 0.2], ['fx.bits', 12],
  ['send.delay', 0.3], ['send.reverb', 0.3], ['send.chorus', 0.3],
];
const now = x.synths_time(engine);
push(busy.map(([name, value]) => [now, PARAM, 0xffff, id[name], value]));
push([[now, PARAM, 0, id['master.volume'], 0.05]]);
push(Array.from({ length: 64 }, (_, i) => [now, NOTE_ON, i % 16, 0, 36 + ((i * 7) % 60), 0.8]));
for (let i = 0; i < 100; i++) x.synths_render(engine, 128);
const voices = x.synths_active_voices(engine);
const blocks = (10 * SR) / 128;
const start = performance.now();
for (let i = 0; i < blocks; i++) x.synths_render(engine, 128);
const ms = performance.now() - start;
const left = new Float32Array(memory.buffer, x.synths_output_left(engine), 128);
if (!left.every(Number.isFinite) || voices !== 64) throw new Error('the busy engine went wrong');
console.log(`${voices} voices with everything on: ${(ms / 10).toFixed(1)} ms per second of sound (${(ms / 100).toFixed(1)}% of one core), ${(10000 / ms).toFixed(0)} times faster than real time`);
x.synths_destroy(engine);
console.log(`memory: ${(memory.buffer.byteLength / 1048576).toFixed(1)} MB`);
