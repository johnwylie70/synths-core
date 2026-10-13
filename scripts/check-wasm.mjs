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
  events.forEach(([time, type, part, a, xv, y, z, b], i) => {
    const at = i * 32;
    view.setFloat64(at, time, true);
    view.setUint16(at + 8, type, true);
    view.setUint16(at + 10, part, true);
    view.setUint32(at + 12, a, true);
    view.setFloat32(at + 16, xv, true);
    view.setFloat32(at + 20, y || 0, true);
    view.setFloat32(at + 24, z || 0, true);
    view.setUint32(at + 28, b || 0, true);
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

// What a display reads. The scope's last frame is the last frame rendered.
const scope = x.synths_alloc(4096 * 4);
const got = x.synths_scope_read(engine, 0xffff, scope, 4096);
const seen = new Float32Array(memory.buffer, scope, 4096);
const right = new Float32Array(memory.buffer, x.synths_output_right(engine), 128);
const levels = x.synths_alloc(16);
x.synths_levels_read(engine, 0xffff, 1, levels);
const peakNow = new Float32Array(memory.buffer, levels, 4)[0];
const words = x.synths_alloc(32);
x.synths_notes_read(engine, 0, words);
const held = new Uint32Array(memory.buffer, words, 8);
const lastLeft = out[SR - 1];
console.log(`taps: ${got} scope frames, master peak ${peakNow.toFixed(3)}, note 69 held: ${(held[2] >>> 5) & 1}`);
if (got !== 4096 || Math.abs(seen[4095] - 0.5 * (lastLeft + right[127])) > 1e-6 || !(peakNow > 0.15) || ((held[2] >>> 5) & 1) !== 1) {
  console.error('the taps do not show what was played');
  process.exit(1);
}
// A step macro: three steps looping make a chord of one voice.
const macro = x.synths_alloc(15 * 4);
new Float32Array(memory.buffer, macro, 15).set([-1, 0, -1, 0, 1, -1, 4, -1, 0, 1, -1, 7, -1, 0, 1]);
if (x.synths_macro_set(engine, 0, macro, 3, 0, 3, 255, 0) !== 0) throw new Error('the macro was refused');
x.synths_render(engine, 4096);
x.synths_macro_set(engine, 0xffff, 0, 0, 0, 0, 0, 0);
[scope, levels, words, macro].forEach((p) => x.synths_free(p));

// A chord (its notes together under one handle), then an arpeggio struck in
// sixteenths at 150 beats a minute: four steps in 19200 frames at 48 kHz.
x.synths_reset(engine);
const ARP = 13, ARP_STYLE = 17, ARP_BEAT = 18, ARP_NOTES = 19, NOTE_OFF = 2;
let t0 = x.synths_time(engine);
push([
  [t0, PARAM, 0, id['amp.release'], 0],
  [t0, PARAM, 0, id.tempo, 150],
  [t0, ARP, 0, 0, -1, 6, 3, 0x00070300],
  [t0, NOTE_ON, 0, 5, 60, 0.3],
  [t0 + 4800, NOTE_OFF, 0, 5, 0],
]);
x.synths_render(engine, 2048);
const together = x.synths_active_voices(engine);
x.synths_render(engine, 4096);
t0 = x.synths_time(engine);
push([
  [t0, ARP_NOTES, 0, 0, 0, 0, 3, 0x00070300],
  [t0, ARP_STYLE, 0, 0, 0.25, 2, 0, 0],
  [t0, ARP_BEAT, 0, 0, 4],
  [t0, NOTE_ON, 0, 6, 60, 0.3],
  [t0 + 19200, NOTE_OFF, 0, 6, 0],
]);
let struck = 0, sounding = 0;
for (let i = 0; i < 180; i++) {
  x.synths_render(engine, 128);
  const now = x.synths_active_voices(engine);
  if (now > sounding) struck++;
  sounding = now;
}
console.log(`a chord of ${together} voices under one handle; an arpeggio in sixteenths at 150: ${struck} notes struck in 0.4 s`);
if (together !== 3 || struck !== 4 || sounding !== 0) {
  console.error('the chord or the arpeggio is not what was asked for');
  process.exit(1);
}
// A pattern for an arpeggio and one for a part's gate, both as text; text
// that is not a pattern is refused.
const say = (fn, where, words) => {
  const bytes = new TextEncoder().encode(words);
  const at = x.synths_alloc(bytes.length);
  new Uint8Array(memory.buffer, at, bytes.length).set(bytes);
  const answer = fn(engine, where, at, bytes.length);
  x.synths_free(at);
  return answer;
};
const steps = [say(x.synths_arp_pattern_text, 1, '1 - 3 2 _ 1 3 5^'), say(x.synths_gate_text, 0, 'x.x.xx.x x.x.x.xx'), say(x.synths_gate_text, 0, 'trance')];
const refused = [say(x.synths_arp_pattern_text, 1, '1 two'), say(x.synths_gate_text, 0, 'x.y')];
push([
  [x.synths_time(engine), PARAM, 0, id['gate.depth'], 1],
  [x.synths_time(engine), PARAM, 0, id['osc1.wave'], 3],
  [x.synths_time(engine), 22 /* clock */, 0, 0, 0],
  [x.synths_time(engine), NOTE_ON, 0, 7, 60, 0.5],
  [x.synths_time(engine) + 15000, NOTE_OFF, 0, 7, 0],
]);
// At 150 beats a minute a sixteenth is 4800 frames: "x.x." is open, closed, open.
const loud = [];
for (let i = 0; i < 3; i++) {
  for (let j = 0; j < 37; j++) x.synths_render(engine, 128);
  loud.push(Math.max(...new Float32Array(memory.buffer, x.synths_output_left(engine), 128).map(Math.abs)) > 0.01);
  x.synths_render(engine, 64);
}
for (let j = 0; j < 40; j++) x.synths_render(engine, 128);
console.log(`patterns as text: an arpeggio's of ${steps[0]} steps, a gate's of ${steps[1]} and ${steps[2]}; the gate's first steps: ${loud.map((on) => (on ? 'x' : '.')).join('')}`);
if (steps.join() !== '8,16,16' || refused.join() !== '-2,-2' || loud.join() !== 'true,false,true') {
  console.error('the patterns are not what was asked for');
  process.exit(1);
}
push([[x.synths_time(engine), PARAM, 0, id['gate.depth'], 0], [x.synths_time(engine), PARAM, 0, id['osc1.wave'], 0]]);
push(['amp.release', 'tempo'].map((name) => [x.synths_time(engine), PARAM, 0, id[name], table.params.find((p) => p.name === name).default]));

// A sample, through memory from synths_alloc.
const frames = 4800;
const ptr = x.synths_alloc(frames * 4);
new Float32Array(memory.buffer, ptr, frames).fill(0.5);
if (x.synths_sample_load(engine, 0, ptr, frames, 1, SR) !== 0) throw new Error('the sample did not load');
x.synths_free(ptr);

// A live input, as a microphone would be fed in: a 440 Hz tone goes into the
// engine's input buffer block by block, is recorded as it arrives, and
// comes out an octave higher through the pitch shifter of the input's part.
x.synths_reset(engine);
const RECORD = 16, VOICE = table.params.find((p) => p.name === 'input.part').default;
if (x.synths_record_buffer(engine, 1, 4800, 1) !== 0) throw new Error('no room to record into');
const then = x.synths_time(engine);
push([
  [then, PARAM, VOICE, id['fx.pitch'], 12],
  [then, RECORD, 0, 1, 1, 0],
]);
const heard = new Float32Array(SR);
for (let done = 0; done < SR; done += 128) {
  const input = new Float32Array(memory.buffer, x.synths_input_left(engine), 128);
  for (let i = 0; i < 128; i++) input[i] = 0.5 * Math.sin((2 * Math.PI * 440 * (done + i)) / SR);
  const n = x.synths_render_io(engine, 128, 1);
  heard.set(new Float32Array(memory.buffer, x.synths_output_left(engine), n), done);
}
let up = 0;
for (let i = 24001; i < SR; i++) if (heard[i - 1] < 0 && heard[i] >= 0) up++;
const state = x.synths_alloc(16);
x.synths_record_read(engine, state);
const [recording, , recorded, room] = new Uint32Array(memory.buffer, state, 4);
const tape = x.synths_alloc(4800 * 4);
const read = x.synths_sample_read(engine, 1, 0, tape, 4800);
const taped = new Float32Array(memory.buffer, tape, 4800);
const expected = Math.fround(0.5 * Math.sin((2 * Math.PI * 440 * 4799) / SR));
console.log(`a live input: 440 Hz in, ${up * 2} Hz out through the pitch shifter; ${recorded} of ${room} frames recorded, ${table.voice_presets.length} voice presets`);
if (up !== 440 || recording !== 0 || recorded !== 4800 || read !== 4800 || x.synths_sample_frames(engine, 1) !== 4800 || taped[4799] !== expected) {
  console.error('the live input did not come through as it should');
  process.exit(1);
}
[state, tape].forEach((p) => x.synths_free(p));
x.synths_sample_free(engine, 1);
push([[x.synths_time(engine), PARAM, VOICE, id['fx.pitch'], 0]]);

// A sample map: two recordings, of the A below middle C and the A two
// octaves above it, each covering half the keyboard. A note in each half
// comes out at its own pitch, from the recording nearest it.
x.synths_reset(engine);
[[2, 220], [3, 880]].forEach(([slot, hz]) => {
  const tone = x.synths_alloc(2 * SR * 4);
  new Float32Array(memory.buffer, tone, 2 * SR).forEach((_, i, data) => { data[i] = 0.5 * Math.sin((2 * Math.PI * hz * i) / SR); });
  if (x.synths_sample_load(engine, slot, tone, 2 * SR, 1, SR) !== 0) throw new Error('the sample did not load');
  x.synths_free(tone);
});
const ZONE = table.limits.zone_size;
const zones = x.synths_alloc(4 * ZONE);
new Uint8Array(memory.buffer, zones, 4 * ZONE).fill(0);
const zone = new DataView(memory.buffer, zones, 2 * ZONE);
[[2, 0, 68, 57], [3, 69, 127, 81]].forEach(([slot, lowest, highest, root], i) => {
  const at = i * ZONE;
  zone.setUint16(at, slot, true);
  zone.setUint8(at + 2, lowest);
  zone.setUint8(at + 3, highest);
  zone.setUint8(at + 5, 127);          // every velocity
  zone.setFloat32(at + 8, root, true);
  zone.setFloat32(at + 16, 1, true);   // volume
  zone.setFloat32(at + 24, -1, true);  // the part's own release
});
if (x.synths_zones_set(engine, 5, zones, 2, 0) !== 0) throw new Error('the zones were refused');
const readBack = x.synths_zones_read(engine, 5, zones + 2 * ZONE, 2);
const bytes = new Uint8Array(memory.buffer, zones, 4 * ZONE);
const same = bytes.subarray(0, 2 * ZONE).every((b, i) => b === bytes[2 * ZONE + i]);
const pitches = [45, 93].map((note) => {
  x.synths_reset(engine);
  const at = x.synths_time(engine);
  push([[at, PARAM, 5, id['osc1.wave'], 4], [at, PARAM, 5, id['amp.attack'], 0], [at, NOTE_ON, 5, 0, note, 1]]);
  const sound = new Float32Array(SR);
  for (let done = 0; done < SR; done += 128) {
    const n = x.synths_render(engine, Math.min(128, SR - done));
    sound.set(new Float32Array(memory.buffer, x.synths_output_left(engine), n), done);
  }
  // The pitch, from the time between the first and last upward crossings of zero.
  let ups = 0, from = 0, to = 0;
  for (let i = 12001; i < SR; i++) {
    if (sound[i - 1] < 0 && sound[i] >= 0) {
      to = i - 1 + sound[i - 1] / (sound[i - 1] - sound[i]);
      if (ups++ === 0) from = to;
    }
  }
  return ((ups - 1) * SR) / (to - from);
});
console.log(`a sample map: ${x.synths_zones_count(engine, 5)} zones of ${ZONE} bytes, notes 45 and 93 at ${pitches.map((hz) => hz.toFixed(1)).join(' and ')} Hz`);
if (readBack !== 2 || !same || x.synths_zones_count(engine, 5) !== 2 || Math.abs(pitches[0] - 110) > 0.1 || Math.abs(pitches[1] - 1760) > 1) {
  console.error('the sample map did not play the notes it should');
  process.exit(1);
}
x.synths_zones_clear(engine, 0xffff);
x.synths_free(zones);
x.synths_reset(engine);

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

// The reverb: one held note sent to each type, against the same note sent
// nowhere, 10 seconds of sound each. The difference is what the reverb costs.
function oneNote(type) {
  x.synths_reset(engine);
  const at = x.synths_time(engine);
  push([
    [at, PARAM, 0xffff, id['send.delay'], 0], [at, PARAM, 0xffff, id['send.chorus'], 0],
    [at, PARAM, 0xffff, id['send.reverb'], type < 0 ? 0 : 0.5], [at, PARAM, 0, id['reverb.type'], Math.max(type, 0)],
    [at, NOTE_ON, 0, 0, 60, 0.8],
  ]);
  for (let i = 0; i < 400; i++) x.synths_render(engine, 128);
  const start = performance.now();
  for (let i = 0; i < blocks; i++) x.synths_render(engine, 128);
  const took = (performance.now() - start) / 10;
  const out = new Float32Array(memory.buffer, x.synths_output_left(engine), 128);
  if (!out.every(Number.isFinite) || out.every((v) => v === 0)) throw new Error('the reverb went wrong');
  return took;
}
const types = table.params.find((p) => p.name === 'reverb.type').choices;
if (types.join(' ') !== 'room hall plate cathedral spring') throw new Error(`reverb types: ${types}`);
oneNote(-1);
const without = Math.min(oneNote(-1), oneNote(-1));
const costs = types.map((name, type) => `${name} ${Math.max(Math.min(oneNote(type), oneNote(type)) - without, 0).toFixed(2)}`);
console.log(`the reverb, in ms per second of sound: ${costs.join(', ')} (one voice without it: ${without.toFixed(2)})`);
x.synths_destroy(engine);
console.log(`memory: ${(memory.buffer.byteLength / 1048576).toFixed(1)} MB`);
